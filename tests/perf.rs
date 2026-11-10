//! Performance guards: 10k elements stay smooth because unchanged nodes are never
//! re-tessellated and panning never re-uploads geometry.

mod common;

use common::*;
use sketch_rs::editor::Editor;
use sketch_rs::geom::{Rect, V2, v2};
use sketch_rs::model::{Color, Node, NodeId, NodeKind};
use sketch_rs::render::gpu::{CanvasRenderer, OFFSCREEN_FORMAT};
use sketch_rs::render::scene::{self, SceneCache};
use std::time::Instant;

pub fn ten_thousand() -> Editor {
    let mut e = editor(1600.0, 1000.0);
    let mut tx_nodes = Vec::new();
    for i in 0..10_000u64 {
        let (x, y) = ((i % 100) as f64 * 60.0, (i / 100) as f64 * 60.0);
        let kind = match i % 4 {
            0 => NodeKind::Rect,
            1 => NodeKind::Ellipse,
            2 => NodeKind::Star {
                points: 5,
                inner: 0.5,
            },
            _ => NodeKind::Text {
                text: format!("#{i}"),
                font_size: 12.0,
                auto_width: true,
                align: sketch_rs::model::TextAlign::Left,
            },
        };
        let mut n = Node::new(NodeId(0), kind, Rect::new(x, y, 40.0, 40.0));
        n.style
            .set_fill(Some(Color::rgb((i * 7 % 255) as u8, 120, 200)));
        tx_nodes.push(n);
    }
    e.begin("Bulk");
    {
        let mut tx = e.tx();
        for mut n in tx_nodes {
            n.id = tx.alloc_id();
            tx.insert(n, None, None);
        }
    }
    e.commit();
    e
}

#[test]
fn ten_thousand_elements_cache_and_pan_without_rework() {
    let mut e = ten_thousand();
    assert_eq!(e.doc.nodes.len(), 10_000);
    // Whole board in view (worst case: everything visible).
    e.zoom_to_fit();
    let mut cache = SceneCache::default();
    let t0 = Instant::now();
    let f1 = scene::canvas(&e, &mut cache, V2::ZERO, true);
    let first = t0.elapsed();
    assert_eq!(cache.last_misses, 10_000);
    let t1 = Instant::now();
    let f2 = scene::canvas(&e, &mut cache, V2::ZERO, true);
    let second = t1.elapsed();
    assert_eq!(
        cache.last_misses, 0,
        "an unchanged frame tessellates nothing"
    );
    assert_eq!(f1.ops.len(), f2.ops.len());
    eprintln!("10k nodes: first frame {first:?}, cached frame {second:?}");
    // The cached frame is far cheaper than the first and well under a 60 fps budget even
    // in an unoptimized test build.
    assert!(second * 4 < first, "{second:?} vs {first:?}");

    // Editing one node re-tessellates just that node.
    let id = e.doc.root[5000];
    e.edit("Move", |tx| tx.node_mut(id).unwrap().rect.x += 5.0);
    scene::canvas(&e, &mut cache, V2::ZERO, true);
    assert_eq!(cache.last_misses, 1);

    // Panning: same items, so the GPU world buffers are reused (no upload).
    let Some(g) = gpu() else { return };
    let mut r = CanvasRenderer::new(&g.device, OFFSCREEN_FORMAT);
    let tex = g.target([800, 500]);
    let view = tex.create_view(&Default::default());
    let mut draw = |e: &Editor, cache: &mut SceneCache| {
        let f = scene::canvas(e, cache, V2::ZERO, true);
        let mut enc = g.device.create_command_encoder(&Default::default());
        r.render(
            &g.device,
            &g.queue,
            &mut enc,
            &view,
            [800, 500],
            1.0,
            &f,
            [1.0; 4],
        );
        g.queue.submit([enc.finish()]);
    };
    draw(&e, &mut cache);
    for i in 0..5 {
        e.camera.pan = e.camera.pan + v2(3.0 * i as f64, 1.0);
        draw(&e, &mut cache);
    }
    drop(draw);
    assert_eq!(r.uploads, 1, "panning reuses the uploaded geometry");
}

/// Every layer with an effect is the costly case. Box shadows ride in the main pass, so they
/// add none. Shape shadows and layer blurs share atlas pages, at three passes a page.
#[test]
fn effects_on_a_large_file_keep_their_cost_bounded() {
    use sketch_rs::model::{Effect, EffectKind};
    let Some(g) = gpu() else { return };
    let size = [1600u32, 1000];
    let tex = g.target(size);
    let view = tex.create_view(&Default::default());
    let frame_time = |e: &Editor, r: &mut CanvasRenderer, cache: &mut SceneCache| {
        let t = Instant::now();
        let f = scene::canvas(e, cache, V2::ZERO, true);
        let mut enc = g.device.create_command_encoder(&Default::default());
        r.render(
            &g.device, &g.queue, &mut enc, &view, size, 1.0, &f, [1.0; 4],
        );
        g.queue.submit([enc.finish()]);
        // Wait for the GPU, so the time covers drawing and not only recording.
        g.read(&tex);
        t.elapsed()
    };
    let with_effect = |effect: Option<Effect>, only: fn(&NodeKind) -> bool| {
        let mut e = ten_thousand();
        let ids: Vec<NodeId> = e.doc.root.clone();
        e.edit("Effects", |tx| {
            for id in ids {
                let n = tx.node_mut(id).unwrap();
                if only(&n.kind) {
                    n.style.effects.extend(effect);
                }
            }
        });
        e
    };
    let measure = |label: &str, mut e: Editor, zoom: Option<f64>| {
        match zoom {
            Some(z) => {
                e.camera.zoom = z;
                e.camera.pan = v2(2000.0, 2000.0);
            }
            None => e.zoom_to_fit(),
        }
        let mut r = CanvasRenderer::new(&g.device, OFFSCREEN_FORMAT);
        let mut cache = SceneCache::default();
        let first = frame_time(&e, &mut r, &mut cache);
        let mut cached = std::time::Duration::MAX;
        for i in 0..5 {
            e.camera.pan = e.camera.pan + v2(0.5 * i as f64, 0.0);
            cached = cached.min(frame_time(&e, &mut r, &mut cache));
        }
        eprintln!(
            "{label}: first frame {first:?}, panning frame {cached:?}, {} passes",
            r.last_passes
        );
        assert_eq!(
            r.uploads, 1,
            "{label}: panning reuses the uploaded geometry"
        );
        r.last_passes
    };
    let shadow = Effect::new(EffectKind::DropShadow);
    let any = |_: &NodeKind| true;
    let rect = |k: &NodeKind| matches!(k, NodeKind::Rect);
    let star = |k: &NodeKind| matches!(k, NodeKind::Star { .. });

    let plain = measure("10k layers, no effects", with_effect(None, any), None);
    assert_eq!(plain, 1);
    // 2,500 box shadows, all in view.
    let boxes = measure("2.5k box shadows", with_effect(Some(shadow), rect), None);
    assert_eq!(boxes, 1, "box shadows need no pass of their own");
    // 2,500 shape shadows, all in view. They share one atlas page, so it is the canvas pass
    // plus one to draw the page and two to blur it.
    let stars = with_effect(Some(shadow), star);
    assert_eq!(measure("2.5k shape shadows", stars, None), 4);
    // Up close fewer are in view and each is bigger.
    let stars = with_effect(Some(shadow), star);
    let near = measure("shape shadows at 400%", stars, Some(4.0));
    assert!(near <= 7, "{near}");
    let blur = Effect::new(EffectKind::LayerBlur);
    let all = measure("10k layer blurs", with_effect(Some(blur), any), None);
    assert!(all <= 7, "{all}");
    measure(
        "layer blurs at 100%",
        with_effect(Some(blur), any),
        Some(1.0),
    );
    // Each background blur ends the pass and blurs the canvas, so these do add up.
    let glass = Effect::new(EffectKind::BackgroundBlur);
    measure(
        "background blurs at 100%",
        with_effect(Some(glass), rect),
        Some(1.0),
    );
    measure(
        "background blurs at 400%",
        with_effect(Some(glass), rect),
        Some(4.0),
    );
}
