//! Effect styles in the Tokens tab and the inspector's Effects section.

use super::inspector::{EffectChange, effect_fields};
use super::tokens::{Pick, committed, pick_token};
use super::{SidebarTab, UiState};
use crate::editor::Editor;
use crate::model::{Effect, EffectKind, EffectStyleDef, Node, NodeId, StyleEffect, TokenKind};
use crate::theme::{self, Icon};
use egui::{Popup, PopupCloseBehavior, RichText, Ui};

#[derive(Default)]
pub struct EffectStylesUi {
    open: Option<Draft>,
}

/// Text fields commit on focus loss; effect rows update the style directly.
struct Draft {
    id: String,
    name: String,
    description: String,
    error: Option<String>,
}

impl Draft {
    fn of(s: &EffectStyleDef) -> Draft {
        Draft {
            id: s.id.clone(),
            name: s.name.clone(),
            description: s.description.clone().unwrap_or_default(),
            error: None,
        }
    }
}

impl EffectStylesUi {
    pub fn open(&mut self, ed: &Editor, id: &str) {
        self.open = ed.tokens.effect_style(id).map(Draft::of);
    }
}

/// Add a style, copying the first selected layer with effects.
pub fn add_entry(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let from = (ed.selection.iter())
        .filter_map(|id| ed.doc.get(*id))
        .find(|n| !n.style.effects.is_empty())
        .cloned();
    let label = if from.is_some() {
        "Effect style from selection"
    } else {
        "Effect style"
    };
    if ui.button(label).clicked() {
        let id = ed.edit_tokens("Add effect style", None, |t| {
            t.add_default_effect_style(from.as_ref())
        });
        st.tokens.search.clear();
        st.tokens.effect_styles.open(ed, &id);
    }
}

fn label(ed: &Editor, s: &EffectStyleDef) -> String {
    if s.system.is_some() {
        ed.tokens.effect_style_slug(s)
    } else {
        s.name.clone()
    }
}

fn preview(s: &EffectStyleDef) -> String {
    match s.effects.as_slice() {
        [] => "No effects".into(),
        [e] => e.effect.kind.label().into(),
        list => format!("{} effects", list.len()),
    }
}

/// Styles from `system` (None for this project's own) that match the search.
pub fn matching(ed: &Editor, system: Option<&str>, search: &str) -> Vec<EffectStyleDef> {
    ed.tokens
        .effect_styles()
        .iter()
        .filter(|s| s.system.as_deref() == system)
        .filter(|s| {
            search.is_empty()
                || s.name.to_lowercase().contains(search)
                || ed.tokens.effect_style_slug(s).contains(search)
        })
        .cloned()
        .collect()
}

/// Draw one group's effect styles.
pub fn section(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, list: &[EffectStyleDef]) {
    if list.is_empty() {
        return;
    }
    ui.add_space(6.0);
    super::section(ui, "Effect styles");
    for s in list {
        ui.push_id(("effect-style", &s.id), |ui| row(ui, ed, st, s));
    }
}

fn row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &EffectStyleDef) {
    let p = theme::pal(ui);
    let (r, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let open = (st.tokens.effect_styles.open.as_ref()).is_some_and(|d| d.id == s.id);
    if open {
        ui.painter().rect_filled(r, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.hover);
    }
    let card = egui::Rect::from_center_size(
        egui::pos2(r.left() + 13.0, r.center().y - 1.0),
        egui::vec2(11.0, 11.0),
    );
    ui.painter()
        .rect_filled(card.translate(egui::vec2(2.0, 2.0)), 3.0, p.tertiary);
    ui.painter().rect_filled(card, 3.0, p.secondary);
    let vg = ui.painter().layout_no_wrap(
        preview(s),
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
    let name_x = r.left() + 30.0;
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
        st.tokens.effect_styles.open = (!open).then(|| Draft::of(s));
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

fn apply_button(ui: &mut Ui, ed: &mut Editor, s: &EffectStyleDef) -> bool {
    let targets = ed.top_selection();
    let clicked = ui
        .add_enabled(!targets.is_empty(), egui::Button::new("Apply to selection"))
        .on_disabled_hover_text("Select layers first.")
        .clicked();
    if clicked {
        ed.apply_effect_style(&targets, &s.id);
    }
    clicked
}

fn css(ed: &Editor, s: &EffectStyleDef) -> String {
    ed.tokens.stylesheet_with(&[], &[s], false)
}

fn context_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &EffectStyleDef) {
    if apply_button(ui, ed, s) {
        ui.close();
    }
    if ui.button("Copy as CSS").clicked() {
        ui.ctx().copy_text(css(ed, s));
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
        ed.edit_tokens("Delete effect style", None, |t| {
            t.delete_effect_styles(&ids)
        });
        st.tokens.effect_styles.open = None;
        ui.close();
    }
}

fn small(ui: &Ui, s: impl Into<String>) -> RichText {
    RichText::new(s.into())
        .size(theme::SECONDARY)
        .color(theme::pal(ui).secondary)
}

fn locked_fields(ui: &mut Ui, ed: &mut Editor, s: &EffectStyleDef) {
    ui.add_space(3.0);
    ui.label(small(ui, ed.tokens.effect_style_short(s)));
    if let Some(d) = &s.description {
        ui.label(small(ui, d.clone()));
    }
    ui.label(small(ui, ed.tokens.effect_style_var(s)).monospace());
    ui.label(small(ui, "Read-only. Detach the system to edit it."));
    apply_button(ui, ed, s);
    ui.add_space(4.0);
}

// Compare against resolved colors so other edits keep the color token.
fn changed(ed: &Editor, s: &EffectStyleDef, i: usize, change: &EffectChange) -> Vec<StyleEffect> {
    let mut list = s.effects.clone();
    let shown = shown_effects(ed, s);
    if let (EffectChange::Set(f), Some(e), Some(shown)) = (change, list.get_mut(i), shown.get(i)) {
        let mut probe = *shown;
        f(&mut probe);
        if probe.color != shown.color {
            e.color_token = None;
            e.effect.color = shown.color;
        }
    }
    change.apply(&mut list, i, |e| &mut e.effect);
    list
}

fn shown_effects(ed: &Editor, s: &EffectStyleDef) -> Vec<Effect> {
    (ed.tokens.resolve_effect_style_in(&s.id, None))
        .unwrap_or_else(|| s.effects.iter().map(|e| e.effect).collect())
}

fn fields(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, s: &EffectStyleDef) {
    let id = s.id.clone();
    let Some(d) = &mut st.tokens.effect_styles.open else {
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
        let r = ed.edit_tokens("Rename effect style", None, |t| {
            t.update_effect_style(&id, Some(&d.name), None, None)
        });
        d.error = r.err();
    }

    let mut edit: Option<(usize, Vec<StyleEffect>)> = None;
    let shown = shown_effects(ed, s);
    for (i, e) in shown.iter().enumerate() {
        let token = ed.tokens.effect_style_color_token(s, i).cloned();
        let mut pick = None;
        let change = ui.push_id(("effect", i), |ui| {
            let name = token.as_ref().map(|t| t.name.clone());
            effect_fields(ui, ed, i, shown.len(), e, name, |ui, ed| {
                pick = pick_token(ui, ed, "color", TokenKind::Color, None, token.as_ref());
            })
        });
        if let Some(change) = change.inner {
            edit = Some((i, changed(ed, s, i, &change)));
        }
        if let Some(pick) = pick {
            let mut list = s.effects.clone();
            list[i].color_token = match pick {
                Pick::Token(t) => Some(t),
                Pick::Detach => {
                    list[i].effect.color = e.color;
                    None
                }
            };
            edit = Some((i, list));
        }
    }
    if ui.small_button("+ Add effect").clicked() {
        let mut list = s.effects.clone();
        list.push(StyleEffect::literal(Effect::new(EffectKind::DropShadow)));
        edit = Some((list.len(), list));
    }
    if let Some((i, list)) = edit {
        let key = format!("effect-style:{id}:{i}");
        let r = ed.edit_tokens("Edit effect style", Some(&key), |t| {
            t.update_effect_style(&id, None, Some(list), None)
        });
        d.error = r.err();
    }

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
        let res = ed.edit_tokens("Describe effect style", None, |t| {
            t.update_effect_style(&id, None, None, Some(&d.description))
        });
        d.error = res.err();
    }

    // Undo and agent edits can change the style while this draft is open.
    if let Some(now) = ed.tokens.effect_style(&id) {
        if !name.has_focus() {
            d.name = now.name.clone();
        }
        if !description.has_focus() {
            d.description = now.description.clone().unwrap_or_default();
        }
    }

    ui.horizontal(|ui| {
        ui.label(small(ui, ed.tokens.effect_style_var(s)).monospace());
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

fn summary(e: &Effect) -> String {
    let num = |v: f64| format!("{}", (v * 100.0).round() / 100.0);
    let mut out = e.kind.label().to_string();
    if e.kind.is_shadow() {
        out.push_str(&format!(" {} {} {}", num(e.x), num(e.y), num(e.blur)));
        if e.spread != 0.0 {
            out.push_str(&format!(" {}", num(e.spread)));
        }
    } else {
        out.push_str(&format!(" {}", num(e.blur)));
    }
    if !e.visible {
        out.push_str(" (hidden)");
    }
    out
}

/// Pick, create, edit or detach the layer's effect style. Bound effects are read-only.
pub fn inspector_row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, ids: &[NodeId], first: &Node) {
    let bound = ed.bound_effect_style(first).cloned();
    let has_effects = !first.style.effects.is_empty();
    if bound.is_none() && ed.tokens.effect_styles().is_empty() && !has_effects {
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
                for s in ed.tokens.effect_styles().to_vec() {
                    let on = bound.as_ref().is_some_and(|b| b.id == s.id);
                    let r = ui.add(
                        egui::Button::selectable(on, label(ed, &s))
                            .shortcut_text(RichText::new(preview(&s)).color(p.secondary))
                            .min_size(egui::vec2(190.0, 22.0)),
                    );
                    if r.clicked() {
                        ed.apply_effect_style(ids, &s.id);
                    }
                }
                if !ed.tokens.effect_styles().is_empty() {
                    ui.separator();
                }
                match &bound {
                    Some(s) => {
                        if ui.button("Edit style").clicked() {
                            st.sidebar = SidebarTab::Tokens;
                            st.tokens.search.clear();
                            st.tokens.effect_styles.open(ed, &s.id);
                        }
                    }
                    None => {
                        let create = ui
                            .add_enabled(has_effects, egui::Button::new("New style from effects"))
                            .on_disabled_hover_text("Add an effect first.");
                        if create.clicked()
                            && let Some(id) = ed.create_effect_style_from(ids)
                        {
                            st.sidebar = SidebarTab::Tokens;
                            st.tokens.search.clear();
                            st.tokens.effect_styles.open(ed, &id);
                        }
                    }
                }
            });
        if bound.is_some()
            && theme::icon_button(ui, Icon::Minus, 20.0, false)
                .on_hover_text("Detach style")
                .clicked()
        {
            ed.detach_effect_style(ids);
        }
    });
    if bound.is_some() {
        for e in &first.style.effects {
            ui.label(small(ui, summary(e)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;

    #[test]
    fn a_hand_picked_color_replaces_the_token() {
        let mut ed = Editor::default();
        let id = ed.edit_tokens("add", None, |t| t.add_default_effect_style(None));
        let token = ed.edit_tokens("add", None, |t| {
            let token = t
                .create(TokenKind::Color, "shade", "#112233", None)
                .unwrap();
            let mut list = t.effect_style(&id).unwrap().effects.clone();
            list[0].color_token = Some(token.clone());
            t.update_effect_style(&id, None, Some(list), None).unwrap();
            token
        });
        let s = ed.tokens.effect_style(&id).cloned().unwrap();
        assert_eq!(s.effects[0].color_token, Some(token));
        let blur = EffectChange::Set(Box::new(|e| e.blur = 9.0));
        let list = changed(&ed, &s, 0, &blur);
        assert!(list[0].color_token.is_some(), "other fields keep the token");
        let red = EffectChange::Set(Box::new(|e| e.color = Color::rgb(255, 0, 0)));
        let list = changed(&ed, &s, 0, &red);
        assert_eq!(list[0].color_token, None);
        assert_eq!(list[0].effect.color, Color::rgb(255, 0, 0));
    }
}
