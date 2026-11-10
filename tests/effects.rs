//! Layer effects through the real renderer: shadows and blurs at known pixels.

mod common;

use EffectKind::*;
use common::*;
use serde_json::{Value, json};
use sketch_rs::editor::{Editor, Key, Modifiers};
use sketch_rs::geom::{Rect, V2, v2};
use sketch_rs::io::png;
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::effects::edit_effects;
use sketch_rs::model::{
    Color, Effect, EffectKind, Node, NodeId, NodeKind, TextAlign, TokenKind, TokenProp,
};
use sketch_rs::project::Store;
use sketch_rs::render::frame;
use sketch_rs::ui::{UiState, inspector};
use std::path::PathBuf;

const WHITE: Color = Color::WHITE;
const RED: Color = Color::rgb(230, 40, 40);
const BLUE: Color = Color::rgb(30, 80, 230);

fn artifact(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".artifacts/effects");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn node(kind: NodeKind, r: Rect, fill: Color, effects: Vec<Effect>) -> Node {
    let mut n = Node::new(NodeId(0), kind, r);
    n.style.set_fill(Some(fill));
    n.style.effects = effects;
    n
}

fn shadow(kind: EffectKind, x: f64, y: f64, blur: f64, spread: f64, color: Color) -> Effect {
    Effect {
        x,
        y,
        blur,
        spread,
        color,
        ..Effect::new(kind)
    }
}

fn blur(kind: EffectKind, radius: f64) -> Effect {
    Effect {
        blur: radius,
        ..Effect::new(kind)
    }
}

/// A white page to paint on, so pixels read as plain colors.
fn page(e: &mut Editor) -> NodeId {
    let page = node(
        NodeKind::Frame { clip: false },
        Rect::new(0.0, 0.0, 800.0, 500.0),
        WHITE,
        vec![],
    );
    e.add_node(page, None)
}

fn luma(p: [u8; 4]) -> i32 {
    (p[0] as i32 + p[1] as i32 + p[2] as i32) / 3
}

const BLACK: Color = Color::BLACK;

/// Editor with a white page and one more layer on it.
fn scene(n: Node) -> (Editor, NodeId) {
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    let id = e.add_node(n, Some(page));
    (e, id)
}

fn card(effects: Vec<Effect>) -> Node {
    node(
        NodeKind::Rect,
        Rect::new(300.0, 200.0, 200.0, 100.0),
        WHITE,
        effects,
    )
}

#[test]
fn a_drop_shadow_fades_away_from_the_box_and_follows_its_offset() {
    let Some(g) = gpu() else { return };
    let (e, _) = scene(card(vec![shadow(DropShadow, 0.0, 20.0, 20.0, 0.0, BLACK)]));
    let img = render_canvas(g, &e);
    let at = |x, y| luma(px(&img, v2(x, y)));
    assert_eq!(at(400.0, 250.0), 255, "the box itself stays white");
    // Below the box: darkest at the edge, lighter further out, gone past the blur.
    let (near, mid, far) = (at(400.0, 304.0), at(400.0, 325.0), at(400.0, 360.0));
    assert!(near < 60, "{near}");
    assert!(near < mid && mid < far, "{near} {mid} {far}");
    assert_eq!(far, 255);
    assert_eq!(at(400.0, 185.0), 255, "the offset moves it off the top");
    // Half way out, a Gaussian shadow is half as dark. The shadow's edge is 20px down.
    let half = at(400.0, 320.0);
    assert!((half - 128).abs() < 12, "{half}");
}

#[test]
fn spread_grows_a_box_shadow_and_a_translucent_box_shows_none_under_it() {
    let Some(g) = gpu() else { return };
    let mut n = card(vec![shadow(DropShadow, 0.0, 0.0, 0.0, 10.0, RED)]);
    n.style.set_fill(Some(Color::rgba(255, 255, 255, 0)));
    let (e, _) = scene(n);
    let img = render_canvas(g, &e);
    assert!(
        close(px(&img, v2(295.0, 250.0)), RED, 2),
        "a hard 10px ring"
    );
    assert!(close(px(&img, v2(400.0, 306.0)), RED, 2));
    assert!(close(px(&img, v2(285.0, 250.0)), WHITE, 2));
    assert!(
        close(px(&img, v2(400.0, 250.0)), WHITE, 2),
        "nothing under the box, though the box is clear"
    );
}

#[test]
fn an_inner_shadow_stays_inside_the_box_and_its_stroke() {
    let Some(g) = gpu() else { return };
    let mut n = card(vec![shadow(InnerShadow, 0.0, 10.0, 10.0, 0.0, BLACK)]);
    n.style.stroke = Some(BLUE);
    n.style.stroke_width = 6.0;
    n.style.stroke_align = sketch_rs::model::StrokeAlign::Inside;
    let (e, _) = scene(n);
    let img = render_canvas(g, &e);
    let at = |x, y| px(&img, v2(x, y));
    assert!(close(at(400.0, 203.0), BLUE, 2), "the stroke is untouched");
    assert!(luma(at(400.0, 208.0)) < 60, "dark under the top edge");
    assert_eq!(luma(at(400.0, 250.0)), 255, "clear in the middle");
    assert!(luma(at(400.0, 292.0)) > 250, "and at the bottom");
    assert!(close(at(400.0, 190.0), WHITE, 0), "nothing outside");
}

#[test]
fn shapes_cast_shadows_of_their_outline() {
    let Some(g) = gpu() else { return };
    let star = node(
        NodeKind::Star {
            points: 4,
            inner: 0.3,
        },
        Rect::new(300.0, 150.0, 200.0, 200.0),
        RED,
        vec![shadow(DropShadow, 60.0, 0.0, 0.0, 0.0, BLACK)],
    );
    let (mut e, id) = scene(star);
    let img = render_canvas(g, &e);
    let at = |x, y| px(&img, v2(x, y));
    // The right arm of the shadow, 60px right of the star's.
    assert!(is_dark(at(545.0, 250.0)));
    // Between two arms the star is hollow, and so is its shadow.
    assert!(close(at(545.0, 180.0), WHITE, 2));
    assert!(close(at(400.0, 250.0), RED, 2), "the star paints over it");

    // An inner shadow shows inside the outline only.
    e.edit("inner", |tx| {
        tx.node_mut(id).unwrap().style.effects =
            vec![shadow(InnerShadow, 0.0, 0.0, 16.0, 0.0, BLACK)];
    });
    let img = render_canvas(g, &e);
    let at = |x, y| px(&img, v2(x, y));
    assert!(close(at(400.0, 250.0), RED, 6), "the middle stays red");
    let rim = at(488.0, 250.0);
    assert!(
        rim[0] < 150 && rim[1] < 40,
        "dark red near the tip: {rim:?}"
    );
    assert!(close(at(545.0, 180.0), WHITE, 0), "nothing outside");
}

#[test]
fn text_and_groups_cast_shadows_of_what_they_paint() {
    let Some(g) = gpu() else { return };
    let text = node(
        NodeKind::Text {
            text: "IIII".into(),
            font_size: 120.0,
            auto_width: true,
            align: TextAlign::Left,
        },
        Rect::new(100.0, 100.0, 300.0, 140.0),
        RED,
        vec![shadow(DropShadow, 0.0, 200.0, 0.0, 0.0, BLACK)],
    );
    let (mut e, _) = scene(text);
    let img = render_canvas(g, &e);
    let glyphs = Rect::new(100.0, 100.0, 300.0, 140.0);
    let red = count(&img, glyphs, |p| close(p, RED, 40));
    let below = count(&img, glyphs.translate(v2(0.0, 200.0)), is_dark);
    assert!(red > 2000, "{red}");
    assert!(
        (red as f64 - below as f64).abs() < red as f64 * 0.05,
        "{red} {below}"
    );

    // A group has no shape of its own, so its children cast the shadow.
    let page = e.doc.root[0];
    let a = e.add_node(
        node(
            NodeKind::Ellipse,
            Rect::new(500.0, 100.0, 60.0, 40.0),
            BLUE,
            vec![],
        ),
        Some(page),
    );
    let b = e.add_node(
        node(
            NodeKind::Ellipse,
            Rect::new(600.0, 100.0, 60.0, 40.0),
            BLUE,
            vec![],
        ),
        Some(page),
    );
    e.select(vec![a, b]);
    key(&mut e, Key::Char('g'), Modifiers::cmd());
    let group = e.selection[0];
    assert!(e.doc.get(group).unwrap().is_group());
    inspector::apply(&mut e, "effects", &[group], |n| {
        n.style.effects = vec![shadow(DropShadow, 0.0, 100.0, 0.0, 0.0, BLACK)];
    });
    let img = render_canvas(g, &e);
    assert!(is_dark(px(&img, v2(530.0, 220.0))));
    assert!(is_dark(px(&img, v2(630.0, 220.0))));
    assert!(
        close(px(&img, v2(580.0, 220.0)), WHITE, 2),
        "the gap between them"
    );
}

#[test]
fn a_layer_blur_spreads_the_layer_and_what_is_in_it() {
    let Some(g) = gpu() else { return };
    let (mut e, id) = scene(node(
        NodeKind::Frame { clip: true },
        Rect::new(300.0, 200.0, 200.0, 100.0),
        BLUE,
        vec![],
    ));
    e.add_node(
        node(
            NodeKind::Rect,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            RED,
            vec![],
        ),
        Some(id),
    );
    let sharp = render_canvas(g, &e);
    assert!(close(px(&sharp, v2(399.0, 250.0)), RED, 2));
    assert!(close(px(&sharp, v2(401.0, 250.0)), BLUE, 2));
    inspector::apply(&mut e, "effects", &[id], |n| {
        n.style.effects = vec![blur(LayerBlur, 10.0)];
    });
    let img = render_canvas(g, &e);
    let at = |x, y| px(&img, v2(x, y));
    // Where red met blue there is now an even mix of the two.
    let seam = at(400.0, 250.0);
    assert!((seam[0] as i32 - 130).abs() < 12, "{seam:?}");
    assert!((seam[2] as i32 - 135).abs() < 12, "{seam:?}");
    assert!(
        close(at(330.0, 250.0), RED, 6),
        "far from an edge it is still red"
    );
    // The frame's edge is half gone, and reaches past where it was.
    let edge = at(500.0, 250.0);
    assert!(
        (luma(edge) - luma([142, 167, 242, 255])).abs() < 12,
        "{edge:?}"
    );
    assert!(luma(at(512.0, 250.0)) < 250);
    assert_eq!(luma(at(540.0, 250.0)), 255, "but only as far as the blur");
}

#[test]
fn a_background_blur_blurs_what_is_behind_the_layer_only() {
    let Some(g) = gpu() else { return };
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    for i in 0..10 {
        let r = Rect::new(300.0 + i as f64 * 20.0, 100.0, 10.0, 300.0);
        e.add_node(node(NodeKind::Rect, r, BLACK, vec![]), Some(page));
    }
    let mut glass = node(
        NodeKind::Ellipse,
        Rect::new(320.0, 180.0, 160.0, 140.0),
        Color::rgba(255, 255, 255, 0),
        vec![blur(BackgroundBlur, 12.0)],
    );
    glass.style.fills.clear();
    let glass = e.add_node(glass, Some(page));
    let img = render_canvas(g, &e);
    let at = |x, y| luma(px(&img, v2(x, y)));
    // Inside, the stripes average out to gray.
    for x in [385.0, 395.0, 405.0] {
        assert!((at(x, 250.0) - 128).abs() < 25, "x {x}: {}", at(x, 250.0));
    }
    // Outside the ellipse they are as sharp as ever, corners of its box included.
    assert_eq!(at(305.0, 250.0), 0);
    assert_eq!(at(315.0, 250.0), 255);
    assert_eq!((at(325.0, 185.0), at(335.0, 185.0)), (0, 255));

    // Hidden, the effect does nothing.
    inspector::apply(&mut e, "effects", &[glass], |n| {
        n.style.effects[0].visible = false
    });
    let img = render_canvas(g, &e);
    assert_eq!(luma(px(&img, v2(385.0, 250.0))), 0);
    assert_eq!(luma(px(&img, v2(395.0, 250.0))), 255);
}

#[test]
fn spread_grows_and_shrinks_the_shadow_of_a_shape() {
    let Some(g) = gpu() else { return };
    // An ellipse 200 wide and 100 tall, centered on (400, 250).
    let oval = |effects| {
        node(
            NodeKind::Ellipse,
            Rect::new(300.0, 200.0, 200.0, 100.0),
            WHITE,
            effects,
        )
    };
    let (mut e, id) = scene(oval(vec![shadow(DropShadow, 0.0, 0.0, 0.0, 15.0, RED)]));
    let img = render_canvas(g, &e);
    let at = |x, y| px(&img, v2(x, y));
    assert!(close(at(510.0, 250.0), RED, 2), "10px past the right end");
    assert!(close(at(400.0, 190.0), RED, 2), "and past the top");
    assert!(
        close(at(520.0, 250.0), WHITE, 2),
        "but no further than the spread"
    );
    // It is still an ellipse: the corner of its grown box stays clear.
    assert!(close(at(505.0, 195.0), WHITE, 2));

    // An inner shadow's spread pushes it inward from the edge.
    let inner = |spread| vec![shadow(InnerShadow, 0.0, 0.0, 0.0, spread, BLUE)];
    e.edit("inner", |tx| {
        tx.node_mut(id).unwrap().style.effects = inner(20.0)
    });
    let img = render_canvas(g, &e);
    assert!(
        close(px(&img, v2(490.0, 250.0)), BLUE, 2),
        "a 20px band inside"
    );
    assert!(close(px(&img, v2(470.0, 250.0)), WHITE, 2));
    assert!(
        close(px(&img, v2(510.0, 250.0)), WHITE, 2),
        "nothing outside"
    );

    // Other outlines grow with round corners.
    let star = node(
        NodeKind::Star {
            points: 4,
            inner: 0.3,
        },
        Rect::new(300.0, 150.0, 200.0, 200.0),
        WHITE,
        vec![shadow(DropShadow, 0.0, 0.0, 0.0, 12.0, RED)],
    );
    let (e, _) = scene(star);
    let img = render_canvas(g, &e);
    assert!(
        close(px(&img, v2(508.0, 250.0)), RED, 2),
        "past the right tip"
    );
    assert!(close(px(&img, v2(516.0, 250.0)), WHITE, 2));
    assert!(
        close(px(&img, v2(400.0, 250.0)), WHITE, 2),
        "the star covers the rest"
    );
}

#[test]
fn spread_follows_what_the_layer_paints() {
    let Some(g) = gpu() else { return };
    let spread = |s| vec![shadow(DropShadow, 0.0, 150.0, 0.0, s, BLACK)];
    // A ring: an ellipse with a stroke and no fill, and its shadow 150px below.
    let oval = Rect::new(300.0, 100.0, 200.0, 100.0);
    let mut ring = node(NodeKind::Ellipse, oval, WHITE, spread(0.0));
    ring.style.fills.clear();
    ring.style.stroke = Some(BLUE);
    ring.style.stroke_width = 10.0;
    let (mut e, id) = scene(ring);
    let look = |e: &Editor| {
        let img = render_canvas(g, e);
        // The shadow's middle, its right rim, and just outside the rim.
        [400.0, 500.0, 512.0].map(|x| is_dark(px(&img, v2(x, 300.0))))
    };
    assert_eq!(look(&e), [false, true, false]);
    // Grown by 10, the ring is still a ring.
    e.edit("spread", |tx| {
        tx.node_mut(id).unwrap().style.effects = spread(10.0)
    });
    assert_eq!(look(&e), [false, true, true]);

    // A line has no area, and its shadow grows from its stroke.
    let mut line = Node::new(
        NodeId(0),
        NodeKind::Line {
            a: v2(0.0, 0.0),
            b: v2(200.0, 0.0),
        },
        Rect::new(300.0, 100.0, 200.0, 0.0),
    );
    line.style.stroke_width = 4.0;
    line.style.effects = spread(8.0);
    let (e, _) = scene(line);
    let img = render_canvas(g, &e);
    assert!(is_dark(px(&img, v2(400.0, 258.0))), "8px above its middle");
    assert!(close(px(&img, v2(400.0, 236.0)), WHITE, 2));

    // Text grows from its glyphs.
    let text = |s| {
        node(
            NodeKind::Text {
                text: "IIII".into(),
                font_size: 120.0,
                auto_width: true,
                align: TextAlign::Left,
            },
            Rect::new(100.0, 60.0, 300.0, 140.0),
            RED,
            spread(s),
        )
    };
    let dark = |n: Node| {
        let (e, _) = scene(n);
        let img = render_canvas(g, &e);
        count(&img, Rect::new(80.0, 190.0, 340.0, 180.0), is_dark)
    };
    let (thin, thick) = (dark(text(0.0)), dark(text(4.0)));
    assert!(
        thin > 2000 && thick as f64 > thin as f64 * 1.3,
        "{thin} {thick}"
    );
}

#[test]
fn a_background_blur_fades_with_the_layer_and_survives_its_layer_blur() {
    let Some(g) = gpu() else { return };
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    for i in 0..10 {
        let r = Rect::new(300.0 + i as f64 * 20.0, 100.0, 10.0, 300.0);
        e.add_node(node(NodeKind::Rect, r, BLACK, vec![]), Some(page));
    }
    let mut glass = node(
        NodeKind::Rect,
        Rect::new(320.0, 180.0, 160.0, 140.0),
        WHITE,
        vec![blur(BackgroundBlur, 12.0)],
    );
    glass.style.fills.clear();
    let glass = e.add_node(glass, Some(page));
    // On a stripe, then on the gap next to it.
    let stripe_and_gap = |e: &Editor| {
        let img = render_canvas(g, e);
        (
            luma(px(&img, v2(385.0, 250.0))),
            luma(px(&img, v2(395.0, 250.0))),
        )
    };
    let (stripe, gap) = stripe_and_gap(&e);
    assert!(
        (stripe - 128).abs() < 25 && (gap - 128).abs() < 25,
        "{stripe} {gap}"
    );

    let set_opacity =
        |e: &mut Editor, o: f64| inspector::apply(e, "opacity", &[glass], |n| n.style.opacity = o);
    set_opacity(&mut e, 0.0);
    assert_eq!(stripe_and_gap(&e), (0, 255), "a clear layer blurs nothing");
    set_opacity(&mut e, 0.5);
    let (stripe, gap) = stripe_and_gap(&e);
    assert!(
        (stripe - 64).abs() < 20 && (gap - 191).abs() < 20,
        "{stripe} {gap}"
    );

    // On a see-through canvas, mixing leaves a flat backdrop as it was, alpha included.
    let mut flat = editor(800.0, 500.0);
    let under = Color::rgba(230, 40, 40, 128);
    let square = Rect::new(0.0, 0.0, 400.0, 400.0);
    let base = flat.add_node(node(NodeKind::Rect, square, under, vec![]), None);
    let mut pane = node(
        NodeKind::Rect,
        Rect::new(100.0, 100.0, 200.0, 200.0),
        WHITE,
        vec![blur(BackgroundBlur, 12.0)],
    );
    pane.style.fills.clear();
    pane.style.opacity = 0.5;
    let pane = flat.add_node(pane, None);
    let img = png::render(g, &flat.doc, Some(&[base, pane]), 1.0, None).unwrap();
    let (outside, inside) = (img.get_pixel(20, 20).0, img.get_pixel(200, 200).0);
    assert_eq!(outside[3], 128);
    assert!((inside[3] as i32 - 128).abs() <= 1, "{inside:?}");

    // With a layer blur as well, the layer still blurs what is behind it.
    set_opacity(&mut e, 1.0);
    inspector::apply(&mut e, "effects", &[glass], |n| {
        n.style.effects.push(blur(LayerBlur, 4.0))
    });
    let (stripe, gap) = stripe_and_gap(&e);
    assert!(
        (stripe - 128).abs() < 25 && (gap - 128).abs() < 25,
        "{stripe} {gap}"
    );
}

#[test]
fn many_huge_background_blurs_fit_in_a_frame() {
    let Some(g) = gpu() else { return };
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    for _ in 0..8 {
        let glass = node(
            NodeKind::Rect,
            Rect::new(0.0, 0.0, 800.0, 500.0),
            Color::rgba(255, 255, 255, 40),
            vec![blur(BackgroundBlur, 1000.0)],
        );
        e.add_node(glass, Some(page));
    }
    e.camera.zoom = 256.0;
    e.camera.pan = v2(400.0, 250.0);
    let img = render_canvas(g, &e);
    assert!(luma(px(&img, v2(400.0, 250.0))) > 250);
}

#[test]
fn effects_scale_with_the_zoom() {
    let Some(g) = gpu() else { return };
    let (mut e, _) = scene(card(vec![
        shadow(DropShadow, 30.0, 0.0, 0.0, 0.0, BLACK),
        // Too slight to see when zoomed out, where the layer is then drawn as it is.
        blur(LayerBlur, 0.5),
    ]));
    for zoom in [0.25, 1.0, 4.0] {
        e.camera.zoom = zoom;
        e.camera.pan = v2(515.0, 250.0) - v2(400.0, 250.0) / zoom;
        let img = render_canvas(g, &e);
        let to_screen = |p: V2| e.camera.world_to_screen(p);
        let inside = to_screen(v2(515.0, 250.0));
        let outside = to_screen(v2(545.0, 250.0));
        assert!(is_dark(px(&img, inside)), "zoom {zoom}");
        assert!(close(px(&img, outside), WHITE, 2), "zoom {zoom}");
    }
}

#[test]
fn png_export_makes_room_for_shadows() {
    let Some(g) = gpu() else { return };
    let (mut e, id) = scene(card(vec![]));
    let plain = png::render(g, &e.doc, Some(&[id]), 1.0, None).unwrap();
    assert_eq!(plain.dimensions(), (200, 100));
    inspector::apply(&mut e, "effects", &[id], |n| {
        n.style.effects = vec![shadow(DropShadow, 0.0, 10.0, 20.0, 0.0, BLACK)];
    });
    let img = png::render(g, &e.doc, Some(&[id]), 1.0, None).unwrap();
    // 30px of blur on each side, moved 10px down.
    assert_eq!(img.dimensions(), (260, 160));
    assert_eq!(img.get_pixel(130, 70).0, [255, 255, 255, 255]);
    let under = img.get_pixel(130, 125).0;
    assert!(
        under[3] > 100 && under[0] < 10,
        "black, part clear: {under:?}"
    );
    assert_eq!(img.get_pixel(0, 0).0[3], 0);

    // A group casts its shadow from its children and their shadows, and needs room for it.
    let a = e.add_node(card(vec![]), Some(e.doc.root[0]));
    e.select(vec![id, a]);
    key(&mut e, Key::Char('g'), Modifiers::cmd());
    let group = e.selection[0];
    let right = |x| vec![shadow(DropShadow, x, 0.0, 0.0, 0.0, BLACK)];
    inspector::apply(&mut e, "effects", &[id], |n| n.style.effects = right(200.0));
    inspector::apply(&mut e, "effects", &[group], |n| {
        n.style.effects = right(200.0)
    });
    let img = png::render(g, &e.doc, Some(&[group]), 1.0, None).unwrap();
    // The cards, the child's shadow, then the group's shadow of that shadow.
    assert_eq!(img.dimensions(), (600, 100));
    assert_eq!(img.get_pixel(300, 50).0, [0, 0, 0, 255]);
    assert_eq!(img.get_pixel(500, 50).0, [0, 0, 0, 255]);
    inspector::apply(&mut e, "effects", &[group], |n| n.style.effects.clear());

    // The project thumbnail draws them too.
    let thumb = sketch_rs::session::render_thumbnail(g, &e.doc)
        .unwrap()
        .unwrap();
    let thumb = image::load_from_memory(&thumb).unwrap().to_rgba8();
    let (w, h) = thumb.dimensions();
    let dark = thumb.pixels().filter(|p| p.0[3] > 200 && p.0[0] < 200);
    assert!(dark.count() > 200, "{w}x{h}");
}

#[test]
fn effect_edits_undo_and_instances_follow_until_overridden() {
    let (mut e, id) = scene(node(
        NodeKind::Frame { clip: true },
        Rect::new(100.0, 100.0, 100.0, 100.0),
        RED,
        vec![],
    ));
    e.select(vec![id]);
    e.create_component();
    let inst = e.create_instance(id, v2(400.0, 100.0)).unwrap();
    let effects = |e: &Editor, id: NodeId| e.doc.get(id).unwrap().style.effects.clone();

    inspector::apply(&mut e, "effects", &[id], |n| {
        edit_effects(n, |l| l.insert(0, (Effect::new(DropShadow), None)))
    });
    e.history.seal();
    assert_eq!(effects(&e, id), [Effect::new(DropShadow)]);
    assert_eq!(effects(&e, inst), effects(&e, id), "the instance follows");

    key(&mut e, Key::Char('z'), Modifiers::cmd());
    assert!(effects(&e, id).is_empty() && effects(&e, inst).is_empty());
    key(&mut e, Key::Char('z'), Modifiers::cmd_shift());
    assert_eq!(effects(&e, inst).len(), 1);

    // An effect edited on the instance is an override, so the main no longer reaches it.
    inspector::apply(&mut e, "effects", &[inst], |n| {
        n.style.effects[0].blur = 30.0
    });
    e.history.seal();
    let link = e.doc.get(inst).unwrap().instance.clone().unwrap();
    assert_eq!(link.overrides, ["effects"]);
    inspector::apply(&mut e, "effects", &[id], |n| n.style.effects.clear());
    assert!(effects(&e, id).is_empty());
    assert_eq!(effects(&e, inst)[0].blur, 30.0);
    // Until the override is reset.
    e.select(vec![inst]);
    e.reset_overrides();
    assert!(effects(&e, inst).is_empty());
}

#[test]
fn a_shadow_color_follows_its_token_through_reorders() {
    let (mut e, id) = scene(card(vec![
        shadow(DropShadow, 0.0, 2.0, 4.0, 0.0, BLACK),
        shadow(InnerShadow, 0.0, 1.0, 2.0, 0.0, BLACK),
    ]));
    let token = e
        .edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "shadow", "#ff0000", None)
        })
        .unwrap();
    let color = |e: &Editor, i: usize| e.doc.get(id).unwrap().style.effects[i].color;
    let bound = |e: &Editor, i: usize| {
        let n = e.doc.get(id).unwrap();
        e.bound_token(n, TokenProp::EffectColor(i)).is_some()
    };
    e.bind_token(&[id], TokenProp::EffectColor(1), &token);
    assert_eq!(color(&e, 1), Color::rgb(255, 0, 0));
    assert_eq!(color(&e, 0), BLACK);
    assert!(bound(&e, 1) && !bound(&e, 0));

    // Moving the effect moves its binding.
    inspector::apply(&mut e, "effects", &[id], |n| {
        edit_effects(n, |l| l.swap(0, 1))
    });
    assert!(bound(&e, 0) && !bound(&e, 1));
    e.edit_tokens("edit", None, |t| {
        t.update(&token, None, Some("#00ff00"), None)
    })
    .unwrap();
    assert_eq!(color(&e, 0), Color::rgb(0, 255, 0));
    assert_eq!(color(&e, 1), BLACK);

    // Blur and offset edits keep it. A hand-picked color is no longer the token's.
    inspector::apply(&mut e, "effects", &[id], |n| n.style.effects[0].blur = 9.0);
    assert!(bound(&e, 0));
    inspector::apply(&mut e, "effects", &[id], |n| {
        n.style.effects[0].color = BLUE
    });
    assert!(!bound(&e, 0));

    // Removing an effect drops its binding and keeps the others in place.
    e.bind_token(&[id], TokenProp::EffectColor(1), &token);
    inspector::apply(&mut e, "effects", &[id], |n| {
        edit_effects(n, |l| {
            l.remove(0);
        })
    });
    assert!(bound(&e, 0));
    assert_eq!(color(&e, 0), Color::rgb(0, 255, 0));
    assert_eq!(e.doc.get(id).unwrap().tokens.len(), 1);
}

// ------------------------------------------------------------------------------------ MCP

fn host() -> exec::SessionHost {
    let dir = std::env::temp_dir().join(format!("sketch-rs-effects-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(dir).unwrap());
    h.session.create(&mut h.ed, "Effects").unwrap();
    h
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    match exec::call(h, name, &args) {
        Ok(outs) => match outs.into_iter().next() {
            Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
            _ => Value::Null,
        },
        Err(e) => panic!("{name} failed: {e}"),
    }
}

fn id_of(v: &Value) -> NodeId {
    let n = v.as_str().unwrap().rsplit(':').next().unwrap();
    NodeId(n.parse().unwrap())
}

#[test]
fn html_shadows_and_blurs_round_trip_through_the_mcp_tools() {
    let mut h = host();
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Card", "styles": {"width": "400px", "height": "300px"}}),
    );
    let html = r#"<div layer-name="Card" style="width:200px; height:100px; background:#fff;
        border-radius:12px; box-shadow: 0 8px 24px rgba(0,0,0,.2), inset 0 1px 0 #fff;
        backdrop-filter: blur(12px)">
          <p style="text-shadow: 0 1px 2px #000; filter: blur(1.5px)">Hi</p>
        </div>
        <div layer-name="Label" style="box-shadow: 0 0 0 2px red">Boxed</div>"#;
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab["id"], "mode": "insert-children", "html": html}),
    );
    assert!(r.get("ignoredStyles").is_none_or(Value::is_null), "{r}");
    let (card, label) = (&r["createdNodeIds"][0], &r["createdNodeIds"][1]);
    let kinds = |h: &exec::SessionHost, id: NodeId| -> Vec<EffectKind> {
        let n = h.ed.doc.get(id).unwrap();
        n.style.effects.iter().map(|e| e.kind).collect()
    };
    assert_eq!(
        kinds(&h, id_of(card)),
        [DropShadow, InnerShadow, BackgroundBlur]
    );
    let card_node = h.ed.doc.get(id_of(card)).unwrap();
    let text = card_node.children[0];
    assert_eq!(kinds(&h, text), [DropShadow, LayerBlur]);
    // Text with a box shadow needs a box to cast it.
    let label_node = h.ed.doc.get(id_of(label)).unwrap();
    assert!(label_node.is_frame(), "{:?}", label_node.kind);
    assert_eq!(label_node.style.effects[0].spread, 2.0);

    let text_key = format!(
        "{}:{}",
        card.as_str().unwrap().split(':').next().unwrap(),
        text.0
    );
    let styles = call(
        &mut h,
        "get_computed_styles",
        json!({"nodeIds": [card, text_key]}),
    );
    let s = &styles[card.as_str().unwrap()];
    assert_eq!(
        s["boxShadow"],
        "0px 8px 24px rgba(0, 0, 0, 0.2), inset 0px 1px 0px #ffffff"
    );
    assert_eq!(s["backdropFilter"], "blur(12px)");
    let t = &styles[text_key.as_str()];
    assert_eq!(t["textShadow"], "0px 1px 2px #000000");
    assert_eq!(t["filter"], "blur(1.5px)");

    let jsx = call(&mut h, "get_jsx", json!({"nodeId": card}));
    let jsx = jsx.as_str().unwrap();
    for part in [
        r#"boxShadow: "0px 8px 24px rgba(0, 0, 0, 0.2), inset 0px 1px 0px #ffffff""#,
        r#"backdropFilter: "blur(12px)""#,
        r#"textShadow: "0px 1px 2px #000000""#,
        r#"filter: "blur(1.5px)""#,
    ] {
        assert!(jsx.contains(part), "{part} not in {jsx}");
    }

    // update_styles replaces the list, drops other filters and reports them.
    let up = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [card], "styles": {
            "boxShadow": "none", "backdropFilter": "blur(4px) saturate(1.8)"
        }}]}),
    );
    assert_eq!(
        up["ignoredStyles"][card.as_str().unwrap()],
        json!(["backdropFilter"])
    );
    let n = h.ed.doc.get(id_of(card)).unwrap();
    assert_eq!(n.style.effects, [blur(BackgroundBlur, 4.0)]);
}

#[test]
fn a_shadow_binds_to_a_color_token_by_var_and_survives_a_reopen() {
    let mut h = host();
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--shadow", "value": "rgba(0, 0, 0, 0.5)"},
            {"type": "spacing", "name": "--lift", "value": "8px"},
        ]}),
    );
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Card", "styles": {
            "width": "200px", "height": "100px",
            "boxShadow": "0 var(--lift) 16px var(--shadow), inset 0 0 0 1px #ffffff",
        }}),
    );
    let id = id_of(&ab["id"]);
    let key = ab["id"].as_str().unwrap();
    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(
        n.style.effects[0].y, 8.0,
        "other tokens fill in their value"
    );
    assert_eq!(n.style.effects[0].color, Color::rgba(0, 0, 0, 128));
    assert!(n.tokens.contains_key(&TokenProp::EffectColor(0)));
    assert!(!n.tokens.contains_key(&TokenProp::EffectColor(1)));
    let styles = call(&mut h, "get_computed_styles", json!({"nodeIds": [key]}));
    assert_eq!(
        styles[key]["boxShadow"],
        "0px 8px 16px var(--shadow), inset 0px 0px 0px 1px #ffffff"
    );

    // Saved, reopened, and still following the token.
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();
    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    assert_eq!(
        again.ed.doc.get(id).unwrap().style.effects,
        h.ed.doc.get(id).unwrap().style.effects
    );
    call(
        &mut again,
        "set_tokens",
        json!({"tokens": [{"name": "--shadow", "value": "#ff0000"}]}),
    );
    let n = again.ed.doc.get(id).unwrap();
    assert_eq!(n.style.effects[0].color, Color::rgb(255, 0, 0));
    assert_eq!(n.style.effects[1].color, WHITE);

    // A file from before effects has none, and opens.
    let old: Node = serde_json::from_str(
        r##"{"id":1,"name":"Old","type":"rect","rect":{"x":0,"y":0,"w":10,"h":10},
            "style":{"fills":[{"type":"solid","color":"#ffffff"}],"stroke":null,
            "stroke_width":1.0,"corner_radius":0.0,"opacity":1.0},
            "tokens":{"fill":"t1"}}"##,
    )
    .unwrap();
    assert!(old.style.effects.is_empty());
    assert_eq!(old.tokens[&TokenProp::Fill], "t1");
}

/// The inspector with every kind of effect on the selection, for a look at the section.
#[test]
fn the_inspector_lists_effects() {
    let Some(g) = gpu() else { return };
    let (mut e, id) = scene(card(vec![
        shadow(DropShadow, 0.0, 8.0, 24.0, -4.0, Color::rgba(0, 0, 0, 90)),
        shadow(InnerShadow, 0.0, 1.0, 0.0, 0.0, WHITE),
        Effect {
            visible: false,
            ..blur(LayerBlur, 4.0)
        },
        blur(BackgroundBlur, 12.0),
    ]));
    let token = e
        .edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "shadow/soft", "#0000005a", None)
        })
        .unwrap();
    e.bind_token(&[id], TokenProp::EffectColor(0), &token);
    let mut st = UiState::default();
    let img = frame::capture(g, &mut e, &mut st, [1280.0, 800.0], 2.0, |e| {
        e.zoom_to_fit();
        e.select(vec![id]);
    });
    img.save(artifact("inspector.png")).unwrap();
    assert_eq!(
        e.doc.get(id).unwrap().style.effects.len(),
        4,
        "looking edits nothing"
    );
}

/// Every effect on one page at three zooms, saved for a look. Nothing may fail to draw.
#[test]
fn every_effect_draws_at_any_zoom() {
    let Some(g) = gpu() else { return };
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    let black = Color::rgba(0, 0, 0, 128);
    // Box drop shadow, rounded, with spread.
    let mut card = node(
        NodeKind::Rect,
        Rect::new(40.0, 40.0, 140.0, 90.0),
        WHITE,
        vec![shadow(DropShadow, 0.0, 12.0, 24.0, 4.0, black)],
    );
    card.style.corner_radius = 16.0;
    e.add_node(card, Some(page));
    // Inner shadow.
    let mut well = node(
        NodeKind::Rect,
        Rect::new(230.0, 40.0, 140.0, 90.0),
        Color::rgb(240, 240, 240),
        vec![shadow(InnerShadow, 0.0, 6.0, 12.0, 0.0, black)],
    );
    well.style.radii = Some([30.0, 0.0, 30.0, 0.0]);
    e.add_node(well, Some(page));
    // Shape shadows go through the blur passes.
    e.add_node(
        node(
            NodeKind::Star {
                points: 5,
                inner: 0.5,
            },
            Rect::new(420.0, 30.0, 120.0, 110.0),
            RED,
            vec![
                shadow(DropShadow, 10.0, 10.0, 8.0, 0.0, black),
                shadow(InnerShadow, 0.0, 4.0, 6.0, 0.0, Color::rgba(0, 0, 0, 200)),
            ],
        ),
        Some(page),
    );
    e.add_node(
        node(
            NodeKind::Text {
                text: "Shadow".into(),
                font_size: 40.0,
                auto_width: true,
                align: TextAlign::Left,
            },
            Rect::new(590.0, 60.0, 160.0, 50.0),
            BLUE,
            vec![shadow(DropShadow, 3.0, 4.0, 4.0, 0.0, black)],
        ),
        Some(page),
    );
    // Layer blur.
    e.add_node(
        node(
            NodeKind::Ellipse,
            Rect::new(40.0, 220.0, 160.0, 120.0),
            BLUE,
            vec![blur(LayerBlur, 12.0)],
        ),
        Some(page),
    );
    // Background blur over stripes.
    for i in 0..10 {
        e.add_node(
            node(
                NodeKind::Rect,
                Rect::new(260.0 + i as f64 * 30.0, 200.0, 14.0, 200.0),
                if i % 2 == 0 { RED } else { BLUE },
                vec![],
            ),
            Some(page),
        );
    }
    let mut glass = node(
        NodeKind::Rect,
        Rect::new(300.0, 240.0, 220.0, 120.0),
        Color::rgba(255, 255, 255, 80),
        vec![
            blur(BackgroundBlur, 10.0),
            shadow(DropShadow, 0.0, 8.0, 20.0, 0.0, black),
        ],
    );
    glass.style.corner_radius = 24.0;
    e.add_node(glass, Some(page));
    // A circle is a box too.
    e.add_node(
        node(
            NodeKind::Ellipse,
            Rect::new(620.0, 240.0, 100.0, 100.0),
            WHITE,
            vec![
                shadow(DropShadow, 0.0, 0.0, 0.0, 6.0, RED),
                shadow(DropShadow, 0.0, 10.0, 20.0, 0.0, black),
            ],
        ),
        Some(page),
    );
    for (name, zoom, pan) in [
        ("gallery.png", 1.0, V2::ZERO),
        ("gallery-3x.png", 3.0, v2(20.0, 20.0)),
        ("gallery-small.png", 0.4, v2(-300.0, -200.0)),
    ] {
        e.camera.zoom = zoom;
        e.camera.pan = pan;
        let img = render_canvas(g, &e);
        img.save(artifact(name)).unwrap();
        // The card's shadow, just under its bottom edge.
        let under = e.camera.world_to_screen(v2(110.0, 136.0));
        assert!(luma(px(&img, under)) < 200, "{name}");
    }
}
