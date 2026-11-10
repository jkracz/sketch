//! Layer effects through the real renderer: shadows and blurs at known pixels.

mod common;

use common::*;
use sketch_rs::editor::Editor;
use sketch_rs::geom::{Rect, v2};
use sketch_rs::model::{Color, Effect, EffectKind, Node, NodeId, NodeKind, TextAlign};
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

#[test]
fn gallery() {
    let Some(g) = gpu() else { return };
    let mut e = editor(800.0, 500.0);
    let page = page(&mut e);
    let black = Color::rgba(0, 0, 0, 128);
    use EffectKind::*;
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
    let img = render_canvas(g, &e);
    img.save(artifact("gallery.png")).unwrap();
    e.camera.zoom = 3.0;
    e.camera.pan = v2(20.0, 20.0);
    render_canvas(g, &e)
        .save(artifact("gallery-3x.png"))
        .unwrap();
    e.camera.zoom = 0.4;
    e.camera.pan = v2(-300.0, -200.0);
    render_canvas(g, &e)
        .save(artifact("gallery-small.png"))
        .unwrap();
    let _ = luma;
}
