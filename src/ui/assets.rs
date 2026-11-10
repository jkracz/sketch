//! The Assets tab: the components of the design systems this file uses, with search and
//! thumbnails. Click one to insert it at the center of the view, or drag it onto the canvas.

use super::UiState;
use crate::editor::Editor;
use crate::geom::v2;
use crate::model::{Document, LinkedComponent};
use crate::render::gpu::Headless;
use crate::theme;
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct AssetsUi {
    pub query: String,
    /// Renders thumbnails. The app sets it from the window's device.
    pub gpu: Option<Headless>,
    /// The component being dragged out of the panel.
    pub dragging: Option<String>,
    thumbs: HashMap<String, Thumb>,
}

struct Thumb {
    /// The copy it shows. Updating the system makes a new one.
    doc: Arc<Document>,
    fonts: u64,
    ppp: f32,
    /// The box it fits in, in points.
    fit: f32,
    /// Texture pixels per canvas unit.
    scale: f32,
    /// `None` without a GPU or when the render failed.
    tex: Option<egui::TextureHandle>,
}

impl Thumb {
    fn current(&self, c: &LinkedComponent, ppp: f32, fit: f32) -> bool {
        Arc::ptr_eq(&self.doc, &c.doc)
            && self.fonts == crate::fonts::generation()
            && self.ppp == ppp
            && self.fit == fit
    }
}

/// Smallest tile edge. Tiles grow to fill the panel's width.
const TILE: f32 = 92.0;
const PAD: f32 = 8.0;
/// Thumbnails rendered per frame, so opening the tab on a big system doesn't stall.
const RENDERS_PER_FRAME: usize = 4;

/// The panel tile of component `id`.
pub fn tile_id(id: &str) -> egui::Id {
    egui::Id::new(("asset-tile", id))
}

/// Insert component `id` at the center of the view, as if dropped there.
pub fn insert(ed: &mut Editor, id: &str) {
    let at = ed.camera.screen_to_world(ed.viewport * 0.5);
    ed.end_text_edit();
    ed.drop_component(id, at);
}

/// Components whose name or system name contains every word of `query`, ignoring case.
pub fn search<'a>(ed: &'a Editor, query: &str) -> Vec<&'a LinkedComponent> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    (ed.tokens.components().iter())
        .filter(|c| {
            let system = ed.tokens.system(&c.system).map_or("", |s| s.name.as_str());
            let hay = format!("{} {system}", c.name).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

fn render(gpu: &Headless, ctx: &egui::Context, c: &LinkedComponent, ppp: f32, fit: f32) -> Thumb {
    let (w, h) = c.size();
    // Small components show at their size; large ones shrink to fit.
    let scale = (fit as f64 / w.max(h).max(1.0)).min(1.0) * ppp as f64;
    let tex = (c.main())
        .and_then(|m| crate::io::png::render(gpu, &c.doc, Some(&[m]), scale, None).ok())
        .map(|img| {
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            ctx.load_texture(format!("asset-{}", c.id), ci, egui::TextureOptions::LINEAR)
        });
    Thumb {
        doc: c.doc.clone(),
        fonts: crate::fonts::generation(),
        ppp,
        fit,
        scale: scale as f32,
        tex,
    }
}

/// Paint a thumbnail fitted in `rect`, at most at its own size.
fn paint_thumb(painter: &egui::Painter, tex: &egui::TextureHandle, rect: egui::Rect, ppp: f32) {
    let size = tex.size_vec2() / ppp;
    let s = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
    let r = egui::Rect::from_center_size(rect.center(), size * s);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(tex.id(), r, uv, Color32::WHITE);
}

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let p = theme::pal(ui);
    if ed.tokens.components().is_empty() {
        ui.label(
            RichText::new(
                "No components yet. Use a design system\nthat has some from the Tokens tab.",
            )
            .size(theme::SECONDARY)
            .color(p.secondary),
        );
        return;
    }
    let a = &mut st.assets;
    ui.add(
        egui::TextEdit::singleline(&mut a.query)
            .hint_text("Search components")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    let found: Vec<LinkedComponent> = search(ed, &a.query).into_iter().cloned().collect();
    if found.is_empty() {
        ui.label(
            RichText::new(format!("No components match “{}”.", a.query.trim()))
                .size(theme::SECONDARY)
                .color(p.secondary),
        );
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let cols = ((ui.available_width() + 6.0) / (TILE + 6.0))
        .floor()
        .max(1.0) as usize;
    let tile = ((ui.available_width() + 6.0) / cols as f32 - 6.0).max(TILE);
    // In steps of 8 points, so resizing the panel doesn't render thumbnails every frame.
    let fit = ((tile - 2.0 * PAD) / 8.0).floor() * 8.0;
    let mut budget = RENDERS_PER_FRAME;
    let mut insert_id = None;
    egui::ScrollArea::vertical()
        .id_salt("assets")
        .auto_shrink(false)
        .show(ui, |ui| {
            // `components()` is grouped by system.
            for group in found.chunk_by(|x, y| x.system == y.system) {
                let system = ed.tokens.system(&group[0].system);
                super::section(ui, system.map_or("Components", |s| s.name.as_str()));
                ui.add_space(2.0);
                for row in group.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for c in row {
                            ui.vertical(|ui| {
                                ui.set_width(tile);
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(tile, tile),
                                    egui::Sense::hover(),
                                );
                                let resp = ui.interact(
                                    rect,
                                    tile_id(&c.id),
                                    egui::Sense::click_and_drag(),
                                );
                                let dragged = a.dragging.as_deref() == Some(&c.id);
                                let bg = if resp.hovered() || dragged {
                                    p.hover
                                } else {
                                    p.control
                                };
                                ui.painter().rect_filled(rect, 8.0, bg);
                                ui.painter().rect_stroke(
                                    rect,
                                    8.0,
                                    egui::Stroke::new(1.0, p.separator),
                                    egui::StrokeKind::Inside,
                                );
                                let stale =
                                    a.thumbs.get(&c.id).is_none_or(|t| !t.current(c, ppp, fit));
                                if stale && ui.is_rect_visible(rect) {
                                    match (&a.gpu, budget) {
                                        (Some(gpu), 1..) => {
                                            budget -= 1;
                                            a.thumbs.insert(
                                                c.id.clone(),
                                                render(gpu, ui.ctx(), c, ppp, fit),
                                            );
                                        }
                                        (Some(_), 0) => ui.ctx().request_repaint(),
                                        (None, _) => {}
                                    }
                                }
                                if let Some(tex) = a.thumbs.get(&c.id).and_then(|t| t.tex.as_ref())
                                {
                                    paint_thumb(ui.painter(), tex, rect.shrink(PAD), ppp);
                                }
                                ui.add(
                                    egui::Label::new(RichText::new(&c.name).size(theme::SECONDARY))
                                        .truncate(),
                                );
                                if resp.drag_started() {
                                    a.dragging = Some(c.id.clone());
                                }
                                let (w, h) = c.size();
                                let resp = resp.on_hover_text(format!(
                                    "{} · {w:.0}×{h:.0}\nClick to insert, or drag onto the canvas",
                                    c.name
                                ));
                                if resp.clicked() {
                                    insert_id = Some(c.id.clone());
                                }
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
            }
        });
    // Drop thumbnails of components the file no longer has.
    let ids: Vec<&str> = ed
        .tokens
        .components()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    a.thumbs.retain(|id, _| ids.contains(&id.as_str()));
    if let Some(id) = insert_id {
        insert(ed, &id);
    }
}

/// Is `p` on the canvas itself, not on the floating toolbar or another window over it?
fn on_canvas(ctx: &egui::Context, canvas: egui::Rect, p: egui::Pos2) -> bool {
    canvas.contains(p)
        && ctx
            .layer_id_at(p)
            .is_none_or(|l| l.order == egui::Order::Background)
}

/// Follow a component dragged out of the panel: show it under the pointer at canvas scale,
/// outline the frame it would go into, and drop it there on release. Escape cancels.
pub fn drag(ctx: &egui::Context, ed: &mut Editor, st: &mut UiState, canvas: egui::Rect) {
    let a = &mut st.assets;
    let Some(id) = a.dragging.clone() else { return };
    let Some(c) = ed.tokens.component(&id).cloned() else {
        a.dragging = None;
        return;
    };
    let (released, down, esc, pos) = ctx.input(|i| {
        (
            i.pointer.any_released(),
            i.pointer.any_down(),
            i.key_pressed(egui::Key::Escape),
            i.pointer.latest_pos(),
        )
    });
    let world = pos.filter(|p| on_canvas(ctx, canvas, *p)).map(|p| {
        let local = p - canvas.min;
        ed.camera
            .screen_to_world(v2(local.x as f64, local.y as f64))
    });
    if esc || (!released && !down) {
        a.dragging = None;
        ed.hover = None;
        return;
    }
    if released {
        a.dragging = None;
        ed.hover = None;
        if let Some(w) = world {
            ed.end_text_edit();
            ed.drop_component(&id, w);
        }
        return;
    }
    ed.hover = world.and_then(|w| ed.drop_frame_at(w));
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let Some(pos) = pos else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("asset-drag"),
    ));
    let zoom = ed.camera.zoom as f32;
    match a.thumbs.get(&id).filter(|t| t.tex.is_some()) {
        Some(t) => {
            let tex = t.tex.as_ref().unwrap();
            // At canvas scale over the canvas, at thumbnail size over the panel.
            let size = match world {
                Some(_) => tex.size_vec2() / t.scale * zoom,
                None => tex.size_vec2() / t.ppp,
            };
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let r = egui::Rect::from_center_size(pos, size);
            painter.image(tex.id(), r, uv, Color32::from_white_alpha(200));
        }
        None => {
            let (w, h) = c.size();
            let r = egui::Rect::from_center_size(pos, egui::vec2(w as f32, h as f32) * zoom);
            let stroke = egui::Stroke::new(1.5, theme::accent());
            painter.rect_stroke(r, 4.0, stroke, egui::StrokeKind::Inside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::assets::test_button;
    use crate::geom::Rect;
    use crate::model::{Node, NodeId, NodeKind};
    use crate::render::frame;
    use crate::ui::SidebarTab;
    use egui::{Event, PointerButton};

    struct Harness {
        ctx: egui::Context,
        ed: Editor,
        st: UiState,
        canvas: egui::Rect,
    }

    impl Harness {
        fn new() -> Harness {
            let mut ed = Editor::default();
            test_button(&mut ed);
            let st = UiState {
                sidebar: SidebarTab::Assets,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            crate::ui::setup_style(&ctx, st.appearance);
            let mut h = Harness {
                ctx,
                ed,
                st,
                canvas: egui::Rect::NOTHING,
            };
            // egui needs a few passes to settle sizes.
            for _ in 0..3 {
                h.frame(vec![]);
            }
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut l = frame::layout(&self.ctx, input, &mut self.ed, &mut self.st);
            l.output.textures_delta.clear();
            self.canvas = l.canvas;
        }

        fn tile(&self, id: &str) -> egui::Rect {
            self.ctx.read_response(tile_id(id)).unwrap().rect
        }

        fn press(&mut self, p: egui::Pos2, pressed: bool) {
            self.frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn move_to(&mut self, p: egui::Pos2) {
            self.frame(vec![Event::PointerMoved(p)]);
        }

        fn screen(&self, x: f64, y: f64) -> egui::Pos2 {
            let s = self.ed.camera.world_to_screen(v2(x, y));
            self.canvas.min + egui::vec2(s.x as f32, s.y as f32)
        }

        /// Press on `from`, drag to `to` in a few steps and release there.
        fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
            self.move_to(from);
            self.press(from, true);
            for k in 1..=4 {
                self.move_to(from.lerp(to, k as f32 / 4.0));
            }
            self.press(to, false);
            self.frame(vec![]);
        }
    }

    fn instances(ed: &Editor) -> Vec<&Node> {
        (ed.doc.nodes.values())
            .filter(|n| n.instance.as_ref().is_some_and(|l| l.component.is_some()))
            .filter(|n| {
                n.parent
                    .is_none_or(|p| ed.doc.get(p).unwrap().instance.is_none())
            })
            .collect()
    }

    #[test]
    fn dragging_a_tile_onto_a_frame_drops_an_instance_there() {
        let mut h = Harness::new();
        let f = h.ed.add_node(
            Node::new(
                NodeId(0),
                NodeKind::Frame { clip: true },
                Rect::new(0.0, 0.0, 400.0, 300.0),
            ),
            None,
        );
        let from = h.tile("brand/button").center();
        let to = h.screen(200.0, 150.0);
        assert!(h.canvas.contains(to));
        // Halfway, the frame it would go into is outlined.
        h.move_to(from);
        h.press(from, true);
        h.move_to(from.lerp(to, 0.5));
        h.move_to(to);
        assert_eq!(h.st.assets.dragging.as_deref(), Some("brand/button"));
        assert_eq!(h.ed.hover, Some(f));
        h.press(to, false);
        h.frame(vec![]);
        assert_eq!(h.st.assets.dragging, None);
        let placed = instances(&h.ed);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].parent, Some(f));
        let r = h.ed.doc.world_rect(placed[0].id).unwrap();
        assert!((r.center().x - 200.0).abs() < 1.0 && (r.center().y - 150.0).abs() < 1.0);
        assert_eq!(h.ed.selection, [placed[0].id]);
    }

    #[test]
    fn a_drag_released_over_the_sidebar_or_cancelled_places_nothing() {
        let mut h = Harness::new();
        let from = h.tile("brand/button").center();
        h.drag(from, from + egui::vec2(0.0, 120.0));
        assert!(instances(&h.ed).is_empty());

        let to = h.screen(100.0, 100.0);
        h.move_to(from);
        h.press(from, true);
        h.move_to(to);
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(h.st.assets.dragging, None);
        h.press(to, false);
        assert!(instances(&h.ed).is_empty());
    }

    #[test]
    fn clicking_a_tile_inserts_at_the_center_of_the_view() {
        let mut h = Harness::new();
        let p = h.tile("brand/button").center();
        h.move_to(p);
        h.press(p, true);
        h.press(p, false);
        h.frame(vec![]);
        let placed = instances(&h.ed);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].parent, None);
        let c = h.ed.camera.screen_to_world(h.ed.viewport * 0.5);
        let r = h.ed.doc.world_rect(placed[0].id).unwrap();
        assert!((r.center().x - c.x).abs() <= 1.0 && (r.center().y - c.y).abs() <= 1.0);
    }

    #[test]
    fn search_matches_every_word_in_the_name_or_system() {
        let mut ed = Editor::default();
        test_button(&mut ed);
        assert_eq!(search(&ed, "").len(), 1);
        assert_eq!(search(&ed, "  BUT ").len(), 1);
        assert_eq!(search(&ed, "button card").len(), 0);
        assert_eq!(search(&ed, "input").len(), 0);
    }

    #[test]
    fn tiles_show_a_rendered_thumbnail() {
        let Ok(gpu) = Headless::new() else { return };
        let mut ed = Editor::default();
        test_button(&mut ed);
        let mut st = UiState {
            sidebar: SidebarTab::Assets,
            ..Default::default()
        };
        let img = frame::capture(&gpu, &mut ed, &mut st, [1200.0, 800.0], 2.0, |_| {});
        // The blue button shows in the sidebar, and only there.
        let blue = |x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..img.height()).map(move |y| (x, y)))
                .filter(|(x, y)| {
                    let p = img.get_pixel(*x, *y).0;
                    p[2] > 200 && p[0] < 60 && p[1] < 60
                })
                .count()
        };
        assert!(blue(0, 480) > 2000, "{} blue pixels", blue(0, 480));
        assert_eq!(blue(480, 1800), 0);
    }
}
