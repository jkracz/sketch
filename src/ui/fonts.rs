//! The inspector's font controls: a searchable list of Google Fonts, and the weight menu.

use super::tokens::picker;
use crate::editor::Editor;
use crate::fonts::google::{self, Status};
use crate::model::{NodeId, TokenProp};
use crate::theme;
use egui::{Popup, PopupCloseBehavior, RichText, Sense, Ui};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const ROW_HEIGHT: f32 = 26.0;
const POPUP_WIDTH: f32 = 260.0;
/// More textures than a screen of rows. Past this they're dropped and reloaded from the cache.
const MAX_TEXTURES: usize = 96;

const CATEGORIES: [(&str, &str); 6] = [
    ("", "All"),
    ("sans-serif", "Sans serif"),
    ("serif", "Serif"),
    ("display", "Display"),
    ("handwriting", "Handwriting"),
    ("monospace", "Monospace"),
];

fn weight_name(weight: u16) -> &'static str {
    match weight {
        100 => "Thin",
        200 => "ExtraLight",
        300 => "Light",
        400 => "Regular",
        500 => "Medium",
        600 => "SemiBold",
        700 => "Bold",
        800 => "ExtraBold",
        900 => "Black",
        _ => "Custom",
    }
}

fn search_id() -> egui::Id {
    egui::Id::new("font-picker-search")
}

type Textures = Arc<Mutex<HashMap<&'static str, egui::TextureHandle>>>;

/// The families matching the search text and category, in catalog order.
fn matches(search: &str, category: &str) -> Vec<&'static google::Family> {
    let search = search.trim().to_ascii_lowercase();
    google::catalog()
        .iter()
        .filter(|f| category.is_empty() || f.category == category)
        .filter(|f| f.family.to_ascii_lowercase().contains(&search))
        .collect()
}

fn preview_texture(ui: &Ui, family: &'static str) -> Option<egui::TextureHandle> {
    let textures: Textures = ui.data_mut(|d| {
        d.get_temp_mut_or_default::<Textures>(search_id().with("textures"))
            .clone()
    });
    let mut textures = textures.lock().unwrap();
    if let Some(t) = textures.get(family) {
        return Some(t.clone());
    }
    let p = google::take_preview(family)?;
    if textures.len() >= MAX_TEXTURES {
        textures.clear();
    }
    let pixels: Vec<egui::Color32> = p
        .alpha
        .iter()
        .map(|a| egui::Color32::from_white_alpha(*a))
        .collect();
    let image = egui::ColorImage::new([p.width, p.height], pixels);
    let t = ui
        .ctx()
        .load_texture(format!("font-{family}"), image, Default::default());
    textures.insert(family, t.clone());
    Some(t)
}

fn family_row(ui: &mut Ui, family: &'static str, selected: bool) -> egui::Response {
    let pal = theme::pal(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
    if selected || response.hovered() {
        let fill = if selected { pal.pressed } else { pal.hover };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let left = rect.left() + 6.0;
    match preview_texture(ui, family) {
        Some(t) => {
            let size = t.size_vec2() * 0.5;
            let at = egui::pos2(left, rect.center().y - size.y * 0.5);
            let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            ui.painter().with_clip_rect(rect).image(
                t.id(),
                egui::Rect::from_min_size(at, size),
                uv,
                pal.text,
            );
        }
        None => {
            ui.painter().with_clip_rect(rect).text(
                egui::pos2(left, rect.center().y),
                egui::Align2::LEFT_CENTER,
                family,
                egui::FontId::proportional(theme::BODY),
                pal.text,
            );
        }
    }
    response
}

fn family_list(ui: &mut Ui, ed: &mut Editor, current: &str) {
    let mut search: String = ui.data_mut(|d| d.get_temp(search_id())).unwrap_or_default();
    let category_id = search_id().with("category");
    let mut category: &'static str = ui.data_mut(|d| d.get_temp(category_id)).unwrap_or("");
    let field = ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text("Search fonts")
            .desired_width(POPUP_WIDTH - 8.0),
    );
    // Read before refocusing: Enter gives up focus, and taking it back would hide that.
    let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let opened = ui.data_mut(|d| d.remove_temp::<bool>(search_id().with("focus")));
    if !submitted && (opened.is_some() || ui.memory(|m| m.focused().is_none())) {
        field.request_focus();
    }
    let label = CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map_or("All", |(_, l)| *l);
    egui::ComboBox::from_id_salt("font-category")
        .selected_text(label)
        .width(POPUP_WIDTH - 8.0)
        .show_ui(ui, |ui| {
            for (c, l) in CATEGORIES {
                ui.selectable_value(&mut category, c, l);
            }
        });
    ui.data_mut(|d| {
        d.insert_temp(search_id(), search.clone());
        d.insert_temp(category_id, category);
    });
    let list = matches(&search, category);
    let mut picked = submitted
        .then(|| list.first().map(|f| f.family.as_str()))
        .flatten();
    if list.is_empty() {
        ui.label(
            RichText::new("No matching fonts")
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
    }
    egui::ScrollArea::vertical()
        .id_salt("font-list")
        .max_height(ROW_HEIGHT * 12.0)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, list.len(), |ui, rows| {
            for f in &list[rows] {
                if family_row(ui, &f.family, f.family == current).clicked() {
                    picked = Some(&f.family);
                }
            }
        });
    if let Some(family) = picked {
        ed.style_text("font", |s| s.family = Some(family.into()));
        ui.close();
    }
}

/// The font family button, its list, and a token picker.
pub fn font_picker(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId]) {
    let current = ed.current_family().unwrap_or("Inter").to_string();
    ui.horizontal(|ui| {
        let width = ui.available_width() - 26.0;
        let button = ui.add(
            egui::Button::new(RichText::new(&current).size(theme::BODY))
                .right_text("▾")
                .min_size(egui::vec2(width, 24.0)),
        );
        if button.clicked() {
            ui.data_mut(|d| {
                d.remove_temp::<String>(search_id());
                d.insert_temp(search_id().with("focus"), true);
            });
        }
        Popup::from_toggle_button_response(&button)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(POPUP_WIDTH);
                family_list(ui, ed, &current);
            });
        picker(ui, ed, ids, TokenProp::FontFamily);
    });
}

/// The weight menu and, while a font is missing, what's happening to it.
pub fn weight_menu(ui: &mut Ui, ed: &mut Editor) {
    let family = ed.current_family().unwrap_or("Inter").to_string();
    let Some(cs) = ed.current_char_style() else {
        return;
    };
    let weights = google::find(&family).map_or(&[400, 700][..], |f| {
        let italics = (cs.italic && !f.italics.is_empty()) || f.weights.is_empty();
        if italics { &f.italics } else { &f.weights }
    });
    ui.horizontal(|ui| {
        let label = |w: u16| format!("{} {w}", weight_name(w));
        let mut weight = cs.weight;
        egui::ComboBox::from_id_salt("font-weight")
            .selected_text(label(cs.weight))
            .width(120.0)
            .show_ui(ui, |ui| {
                for w in weights {
                    ui.selectable_value(&mut weight, *w, label(*w));
                }
            });
        if weight != cs.weight {
            ed.style_text("bold", |s| s.weight = Some(weight));
        }
        let note = match google::status(&family, cs.weight, cs.italic) {
            Some(Status::Downloading) => "Downloading…",
            Some(Status::Failed(_)) => "Download failed, showing Inter",
            None => "",
        };
        if !note.is_empty() {
            ui.label(
                RichText::new(note)
                    .size(theme::SECONDARY)
                    .color(theme::pal(ui).secondary),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::tokens::tests::{Harness, key, pointer, typed};
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Node, NodeKind, TextAlign};

    fn text_node(h: &mut Harness) -> NodeId {
        let id = h.ed.edit("add", |tx| {
            let id = tx.alloc_id();
            let kind = NodeKind::Text {
                text: "Hi".into(),
                font_size: 16.0,
                auto_width: true,
                align: TextAlign::Left,
            };
            tx.insert(
                Node::new(id, kind, Rect::new(0.0, 0.0, 10.0, 10.0)),
                None,
                None,
            )
        });
        h.ed.select(vec![id]);
        id
    }

    #[test]
    fn search_narrows_by_name_and_category_in_popularity_order() {
        assert_eq!(matches("", "").len(), google::catalog().len());
        assert_eq!(matches("playfair d", "")[0].family, "Playfair Display");
        assert!(
            matches("roboto", "serif")
                .iter()
                .all(|f| f.category == "serif")
        );
        assert!(matches("no such font", "").is_empty());
    }

    #[test]
    fn the_picker_searches_and_applies_with_the_keyboard() {
        let mut h = Harness::new();
        let id = text_node(&mut h);
        let mut f = |ui: &mut Ui, ed: &mut Editor, _: &mut super::super::UiState| {
            font_picker(ui, ed, &[id]);
        };
        h.step(vec![], &mut f);
        let at = h.step(vec![], &mut f).left_center() + egui::vec2(20.0, 0.0);
        h.step(vec![egui::Event::PointerMoved(at)], &mut f);
        h.step(vec![pointer(at, true)], &mut f);
        h.step(vec![pointer(at, false)], &mut f);
        assert!(Popup::is_any_open(&h.ctx), "the click opens the list");
        h.step(vec![], &mut f);
        h.step(vec![typed("lora")], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        let n = h.ed.doc.get(id).unwrap();
        assert_eq!(n.text_style.family.as_deref(), Some("Lora"));
        assert!(!Popup::is_any_open(&h.ctx), "picking closes it");
    }

    #[test]
    fn the_weight_menu_lists_the_familys_weights() {
        let mut h = Harness::new();
        let id = text_node(&mut h);
        h.ed.style_text("font", |s| s.family = Some("Pacifico".into()));
        let mut f = |ui: &mut Ui, ed: &mut Editor, _: &mut super::super::UiState| {
            weight_menu(ui, ed);
        };
        h.step(vec![], &mut f);
        assert_eq!(h.ed.current_family(), Some("Pacifico"));
        assert_eq!(h.ed.doc.get(id).unwrap().text_style.base_weight(), 400);
        assert_eq!(weight_name(600), "SemiBold");
    }
}
