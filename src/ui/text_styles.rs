//! Text styles in the Tokens tab and text inspector.

use super::UiState;
use super::tokens::committed;
use crate::editor::Editor;
use crate::model::{Node, NodeId, NodeKind, StyleField, StyleInput, TextStyleDef};
use crate::theme::{self, Icon};
use egui::{Popup, PopupCloseBehavior, RichText, Ui};

#[derive(Default)]
pub struct StylesUi {
    open: Option<Draft>,
}

const TEXT_FIELDS: [StyleField; 5] = [
    StyleField::Family,
    StyleField::Size,
    StyleField::Weight,
    StyleField::LineHeight,
    StyleField::LetterSpacing,
];

/// Draft fields commit on focus loss.
struct Draft {
    id: String,
    name: String,
    values: [String; 5],
    /// Fields typed in since their last commit. Committing one that has other modes' values
    /// sets every mode, even when the text matches the default.
    typed: [bool; 5],
    description: String,
    error: Option<String>,
}

impl Draft {
    fn of(ed: &Editor, s: &TextStyleDef) -> Draft {
        Draft {
            id: s.id.clone(),
            name: s.name.clone(),
            values: TEXT_FIELDS.map(|f| ed.tokens.style_value_css(s, f)),
            typed: [false; 5],
            description: s.description.clone().unwrap_or_default(),
            error: None,
        }
    }
}

impl StylesUi {
    pub fn open(&mut self, ed: &Editor, id: &str) {
        self.open = ed.tokens.style(id).map(|s| Draft::of(ed, s));
    }
}

fn selected_text(ed: &Editor) -> Vec<NodeId> {
    ed.selection
        .iter()
        .copied()
        .filter(|id| {
            ed.doc
                .get(*id)
                .is_some_and(|n| matches!(n.kind, NodeKind::Text { .. }))
        })
        .collect()
}

/// The + menu entry. A new style copies the first selected text layer.
pub fn add_entry(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let from = selected_text(ed)
        .first()
        .and_then(|id| ed.doc.get(*id))
        .cloned();
    let label = if from.is_some() {
        "Text style from selection"
    } else {
        "Text style"
    };
    if ui.button(label).clicked() {
        let id = ed.edit_tokens("Add text style", None, |t| {
            t.add_default_style(from.as_ref())
        });
        st.tokens.search.clear();
        st.tokens.styles.open(ed, &id);
    }
}

fn label(ed: &Editor, s: &TextStyleDef) -> String {
    if s.system.is_some() {
        ed.tokens.style_class(s)
    } else {
        s.name.clone()
    }
}

fn preview(ed: &Editor, s: &TextStyleDef) -> String {
    let Some(r) = ed.tokens.resolve_style(&s.id) else {
        return String::new();
    };
    let num = |v: f64| format!("{}", (v * 100.0).round() / 100.0);
    let mut out = format!("{} · {}", num(r.size), num(r.weight));
    if r.italic {
        out.push_str(" italic");
    }
    out
}

/// Styles from `system` (None for this project's own) that match the search.
pub fn matching(ed: &Editor, system: Option<&str>, search: &str) -> Vec<TextStyleDef> {
    ed.tokens
        .styles()
        .iter()
        .filter(|s| s.system.as_deref() == system)
        .filter(|s| {
            search.is_empty()
                || s.name.to_lowercase().contains(search)
                || ed.tokens.style_class(s).contains(search)
        })
        .cloned()
        .collect()
}

/// The Text styles heading and rows of one group.
pub fn section(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, list: &[TextStyleDef]) {
    if list.is_empty() {
        return;
    }
    ui.add_space(6.0);
    super::section(ui, "Text styles");
    for s in list {
        ui.push_id(("style", &s.id), |ui| row(ui, ed, st, s));
    }
}

fn row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &TextStyleDef) {
    let p = theme::pal(ui);
    let (r, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let open = st.tokens.styles.open.as_ref().is_some_and(|d| d.id == s.id);
    if open {
        ui.painter().rect_filled(r, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.hover);
    }
    let x = r.left() + 6.0;
    let glyph = ui.painter().layout_no_wrap(
        "Ag".into(),
        egui::FontId::proportional(theme::BODY),
        p.secondary,
    );
    ui.painter().galley(
        egui::pos2(x, r.center().y - glyph.size().y / 2.0),
        glyph,
        p.secondary,
    );
    let vg = ui.painter().layout_no_wrap(
        preview(ed, s),
        egui::FontId::proportional(theme::SECONDARY),
        p.secondary,
    );
    let vw = vg.size().x.min(r.width() * 0.4);
    ui.painter()
        .with_clip_rect(r.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(r.right() - 6.0 - vw, r.center().y - vg.size().y / 2.0),
            vg,
            p.secondary,
        );
    let name_x = x + 24.0;
    let name_clip = egui::Rect::from_min_max(
        egui::pos2(name_x, r.top()),
        egui::pos2(r.right() - vw - 12.0, r.bottom()),
    );
    let ng = ui.painter().layout_no_wrap(
        s.name.clone(),
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    ui.painter()
        .with_clip_rect(name_clip.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(name_x, r.center().y - ng.size().y / 2.0),
            ng,
            p.text,
        );
    if resp.clicked() {
        st.tokens.styles.open = (!open).then(|| Draft::of(ed, s));
    }
    resp.context_menu(|ui| context_menu(ui, ed, st, s));
    if open {
        if s.system.is_some() {
            locked_fields(ui, ed, s);
        } else {
            fields(ui, ed, st, s);
        }
    }
}

fn apply_button(ui: &mut Ui, ed: &mut Editor, s: &TextStyleDef) -> bool {
    let targets = selected_text(ed);
    let clicked = ui
        .add_enabled(!targets.is_empty(), egui::Button::new("Apply to selection"))
        .on_disabled_hover_text("Select text layers first.")
        .clicked();
    if clicked {
        ed.apply_text_style(&targets, &s.id);
    }
    clicked
}

fn context_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &TextStyleDef) {
    if apply_button(ui, ed, s) {
        ui.close();
    }
    if ui.button("Copy as CSS").clicked() {
        ui.ctx().copy_text(ed.tokens.style_sheet(&[s], false));
        ui.close();
    }
    ui.separator();
    let locked = s.system.is_some();
    if ui
        .add_enabled(!locked, egui::Button::new("Delete"))
        .on_disabled_hover_text("Design system styles are read-only. Remove the system instead.")
        .clicked()
    {
        let ids = vec![s.id.clone()];
        ed.edit_tokens("Delete text style", None, |t| t.delete_styles(&ids));
        st.tokens.styles.open = None;
        ui.close();
    }
}

fn small(ui: &Ui, s: impl Into<String>) -> RichText {
    RichText::new(s.into())
        .size(theme::SECONDARY)
        .color(theme::pal(ui).secondary)
}

fn locked_fields(ui: &mut Ui, ed: &mut Editor, s: &TextStyleDef) {
    ui.add_space(3.0);
    ui.label(small(ui, ed.tokens.style_short(s)));
    for note in ed.tokens.style_mode_notes(s) {
        ui.label(small(ui, note));
    }
    if let Some(d) = &s.description {
        ui.label(small(ui, d.clone()));
    }
    ui.label(small(ui, format!(".{}", ed.tokens.style_class(s))).monospace());
    ui.label(small(ui, "Read-only. Detach the system to edit it."));
    apply_button(ui, ed, s);
    ui.add_space(4.0);
}

fn field_label(f: StyleField) -> &'static str {
    match f {
        StyleField::Family => "Family",
        StyleField::Size => "Size",
        StyleField::Weight => "Weight",
        StyleField::Italic => "Italic",
        StyleField::LineHeight => "Line h.",
        StyleField::LetterSpacing => "Spacing",
    }
}

fn fields(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &TextStyleDef) {
    let id = s.id.clone();
    let Some(d) = &mut st.tokens.styles.open else {
        return;
    };
    let field_w = ui.available_width() - 62.0;
    let label = |ui: &mut Ui, text: &str| {
        ui.add_sized(
            egui::vec2(54.0, 18.0),
            egui::Label::new(small(ui, text.to_string())),
        );
    };
    ui.add_space(3.0);
    ui.spacing_mut().item_spacing.y = 3.0;

    let name = ui
        .horizontal(|ui| {
            label(ui, "Name");
            ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(field_w))
        })
        .inner;
    if committed(ui, &name) && d.name != s.name {
        let r = ed.edit_tokens("Rename text style", None, |t| {
            t.update_style(&id, Some(&d.name), &StyleInput::default(), None)
        });
        d.error = r.err();
    }

    let mut responses = Vec::new();
    let by_mode = ed.tokens.style_mode_values(s);
    for (i, f) in TEXT_FIELDS.into_iter().enumerate() {
        let r = ui
            .horizontal(|ui| {
                label(ui, field_label(f));
                let hint = match f {
                    StyleField::LineHeight => "normal",
                    _ => "value or var(--token)",
                };
                ui.add(
                    egui::TextEdit::singleline(&mut d.values[i])
                        .hint_text(hint)
                        .desired_width(field_w),
                )
            })
            .inner;
        d.typed[i] |= r.changed();
        if r.lost_focus() && !committed(ui, &r) {
            d.typed[i] = false;
        }
        let moded = !s.mode_values(f).is_empty();
        if committed(ui, &r)
            && (d.values[i] != ed.tokens.style_value_css(s, f) || (moded && d.typed[i]))
        {
            d.typed[i] = false;
            let mut input = StyleInput::default();
            input.set(f, d.values[i].clone());
            let res = ed.edit_tokens("Edit text style", None, |t| {
                t.update_style(&id, None, &input, None)
            });
            d.error = res.err();
        }
        responses.push(r);
        // Values kept from a flattened token. Typing a value sets every mode.
        let others: Vec<String> = (by_mode.iter())
            .filter_map(|(m, fields)| {
                let (_, v) = fields.iter().find(|(x, _)| *x == f)?;
                Some(format!("{m}: {v}"))
            })
            .collect();
        if !others.is_empty() {
            ui.horizontal(|ui| {
                label(ui, "");
                ui.label(small(ui, others.join(", ")))
                    .on_hover_text("Typing a value sets it for every mode.");
            });
        }
    }

    ui.horizontal(|ui| {
        label(ui, "Italic");
        let mut italic = s.italic;
        if ui.checkbox(&mut italic, "").changed() {
            let mut input = StyleInput::default();
            input.set(
                StyleField::Italic,
                if italic { "italic" } else { "normal" }.into(),
            );
            let res = ed.edit_tokens("Edit text style", None, |t| {
                t.update_style(&id, None, &input, None)
            });
            d.error = res.err();
        }
    });

    let description = ui
        .horizontal(|ui| {
            label(ui, "Note");
            ui.add(
                egui::TextEdit::singleline(&mut d.description)
                    .hint_text("What it's for")
                    .desired_width(field_w),
            )
        })
        .inner;
    if committed(ui, &description) && d.description != s.description.clone().unwrap_or_default() {
        let res = ed.edit_tokens("Describe text style", None, |t| {
            t.update_style(&id, None, &StyleInput::default(), Some(&d.description))
        });
        d.error = res.err();
    }

    // Undo and agent edits can change the style while this draft is open.
    if let Some(now) = ed.tokens.style(&id) {
        if !name.has_focus() {
            d.name = now.name.clone();
        }
        for (i, f) in TEXT_FIELDS.into_iter().enumerate() {
            if !responses[i].has_focus() {
                d.values[i] = ed.tokens.style_value_css(now, f);
            }
        }
        if !description.has_focus() {
            d.description = now.description.clone().unwrap_or_default();
        }
    }

    ui.horizontal(|ui| {
        ui.label(small(ui, format!(".{}", ed.tokens.style_class(s))).monospace());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            apply_button(ui, ed, s)
        });
    });
    if let Some(e) = &d.error {
        ui.label(
            RichText::new(e)
                .size(theme::SECONDARY)
                .color(egui::Color32::from_rgb(0xd9, 0x3a, 0x3a)),
        );
    }
    ui.add_space(4.0);
}

/// Style controls and overrides, hidden when no style is bound or available.
pub fn inspector_row(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], first: &Node) {
    let bound = ed.bound_style(first).cloned();
    if bound.is_none() && ed.tokens.styles().is_empty() {
        return;
    }
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        let text = match &bound {
            Some(s) => label(ed, s),
            None => "No style".into(),
        };
        let button = ui.add(
            egui::Button::new(RichText::new(text).color(if bound.is_some() {
                p.text
            } else {
                p.secondary
            }))
            .min_size(egui::vec2(ui.available_width() - 28.0, 22.0)),
        );
        Popup::from_toggle_button_response(&button)
            .close_behavior(PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                ui.set_min_width(190.0);
                for s in ed.tokens.styles().to_vec() {
                    let on = bound.as_ref().is_some_and(|b| b.id == s.id);
                    let r = ui.add(
                        egui::Button::selectable(on, label(ed, &s))
                            .shortcut_text(RichText::new(preview(ed, &s)).color(p.secondary))
                            .min_size(egui::vec2(190.0, 22.0)),
                    );
                    if r.clicked() {
                        ed.apply_text_style(ids, &s.id);
                    }
                }
            });
        if bound.is_some()
            && theme::icon_button(ui, Icon::Minus, 20.0, false)
                .on_hover_text("Detach style")
                .clicked()
        {
            ed.detach_text_style(ids);
        }
    });
    let Some(s) = bound else { return };
    let Some(r) = &first.text_style_ref else {
        return;
    };
    if r.overrides.is_empty() {
        return;
    }
    let names: Vec<&str> = r.overrides.iter().map(|f| f.label()).collect();
    ui.horizontal(|ui| {
        ui.label(small(ui, format!("Overrides {}", names.join(", "))));
        if ui
            .small_button("Reset")
            .on_hover_text("Use the style's values again")
            .clicked()
        {
            ed.apply_text_style(ids, &s.id);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::tokens::tests::{Harness, key, typed};
    use super::*;
    use crate::model::TokenKind;

    #[test]
    fn a_field_edit_reaches_the_style() {
        let mut h = Harness::new();
        let id = h.ed.edit_tokens("add", None, |t| t.add_default_style(None));
        h.st.tokens.styles.open(&h.ed, &id);
        let mut f = |ui: &mut Ui, ed: &mut Editor, st: &mut UiState| {
            let s = ed.tokens.style(&id).cloned().unwrap();
            fields(ui, ed, st, &s);
        };
        h.step(vec![], &mut f);
        // Focus Size after Name and Family.
        for _ in 0..3 {
            h.step(vec![key(egui::Key::Tab)], &mut f);
            h.step(vec![], &mut f);
        }
        for _ in 0..4 {
            h.step(vec![key(egui::Key::Backspace)], &mut f);
        }
        h.step(vec![typed("28px")], &mut f);
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        let r = h.ed.tokens.resolve_style(&id).unwrap();
        assert_eq!(r.size, 28.0);
    }

    #[test]
    fn retyping_the_default_value_sets_every_mode() {
        let mut h = Harness::new();
        let id = h.ed.edit_tokens("add", None, |t| {
            let dark = vec![("dark".to_string(), "48px".to_string())];
            let size = t
                .create_in_modes(TokenKind::FontSize, "size", "12px", &dark, None)
                .unwrap();
            let mut i = StyleInput::default();
            i.set(StyleField::Size, "var(--size)".into());
            let id = t.create_style("Body", &i, None).unwrap();
            t.delete(&[size]);
            id
        });
        h.st.tokens.styles.open(&h.ed, &id);
        let mut f = |ui: &mut Ui, ed: &mut Editor, st: &mut UiState| {
            let s = ed.tokens.style(&id).cloned().unwrap();
            fields(ui, ed, st, &s);
        };
        h.step(vec![], &mut f);
        for _ in 0..3 {
            h.step(vec![key(egui::Key::Tab)], &mut f);
            h.step(vec![], &mut f);
        }
        // Leaving the field untouched keeps the Dark value.
        h.step(vec![key(egui::Key::Tab)], &mut f);
        h.step(vec![], &mut f);
        assert!(!h.ed.tokens.style(&id).unwrap().modes.is_empty());
        let back = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        h.step(vec![back], &mut f);
        h.step(vec![], &mut f);
        for _ in 0..4 {
            h.step(vec![key(egui::Key::Backspace)], &mut f);
        }
        h.step(vec![typed("12px")], &mut f);
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        assert!(h.ed.tokens.style(&id).unwrap().modes.is_empty());
        let dark = h.ed.tokens.resolve_style_in(&id, Some("Dark")).unwrap();
        assert_eq!(dark.size, 12.0);
    }

    #[test]
    fn escape_keeps_other_modes_values() {
        let mut h = Harness::new();
        let id = h.ed.edit_tokens("add", None, |t| {
            let dark = vec![("dark".to_string(), "48px".to_string())];
            let size = t
                .create_in_modes(TokenKind::FontSize, "size", "12px", &dark, None)
                .unwrap();
            let mut i = StyleInput::default();
            i.set(StyleField::Size, "var(--size)".into());
            let id = t.create_style("Body", &i, None).unwrap();
            t.delete(&[size]);
            id
        });
        h.st.tokens.styles.open(&h.ed, &id);
        let mut f = |ui: &mut Ui, ed: &mut Editor, st: &mut UiState| {
            let s = ed.tokens.style(&id).cloned().unwrap();
            fields(ui, ed, st, &s);
        };
        h.step(vec![], &mut f);
        for _ in 0..3 {
            h.step(vec![key(egui::Key::Tab)], &mut f);
            h.step(vec![], &mut f);
        }
        h.step(vec![typed("9")], &mut f);
        h.step(vec![key(egui::Key::Escape)], &mut f);
        for _ in 0..3 {
            h.step(vec![], &mut f);
        }
        assert!(!h.ed.tokens.style(&id).unwrap().modes.is_empty());
        let dark = h.ed.tokens.resolve_style_in(&id, Some("Dark")).unwrap();
        assert_eq!(dark.size, 48.0);
    }
}
