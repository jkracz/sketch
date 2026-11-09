//! The Tokens tab of the left sidebar: add, edit, duplicate, copy and delete design tokens,
//! with the design systems this project uses listed below its own tokens (`ui::systems`).
//! Also the token picker the inspector opens next to a bindable property.

use super::{UiState, c32, from32};
use crate::editor::Editor;
use crate::model::{NodeId, SystemLink, Token, TokenKind, TokenProp, TokenValue};
use crate::theme::{self, Icon};
use egui::{Popup, PopupCloseBehavior, RichText, Ui};

#[derive(Default)]
pub struct TokensUi {
    search: String,
    /// Selected token ids, for the context menu's multi-token actions.
    selected: Vec<String>,
    /// The token whose fields are shown under its row.
    open: Option<Draft>,
}

/// Text being edited for the open token. It reaches the token when the field loses focus.
struct Draft {
    id: String,
    name: String,
    value: String,
    description: String,
    error: Option<String>,
}

impl TokensUi {
    /// Show this token's fields under its row.
    pub fn open(&mut self, ed: &Editor, id: &str) {
        self.selected = vec![id.to_string()];
        self.open = ed.tokens.get(id).map(|t| Draft::of(ed, t));
    }
}

impl Draft {
    fn of(ed: &Editor, t: &Token) -> Draft {
        Draft {
            id: t.id.clone(),
            name: t.name.clone(),
            value: ed.tokens.css_value(t),
            description: t.description.clone().unwrap_or_default(),
            error: None,
        }
    }
}

const ROW_H: f32 = 24.0;

/// Tokens grouped by kind, in listing order.
type Kinds = Vec<(TokenKind, Vec<Token>)>;

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.tokens.search)
                .hint_text("Search tokens")
                .desired_width(ui.available_width() - 48.0),
        );
        let add = theme::icon_button(ui, Icon::Plus, 20.0, false).on_hover_text("Add token");
        Popup::menu(&add).show(|ui| {
            for kind in TokenKind::ALL {
                if ui.button(kind.label()).clicked() {
                    let id = ed.edit_tokens("Add token", None, |t| t.add_default(kind));
                    st.tokens.search.clear();
                    st.tokens.open(ed, &id);
                }
            }
        });
        let more = theme::icon_button(ui, Icon::More, 20.0, false).on_hover_text("More");
        Popup::menu(&more).show(|ui| {
            let all = ed.tokens.sorted();
            if ui
                .add_enabled(!all.is_empty(), egui::Button::new("Copy all as CSS"))
                .clicked()
            {
                ui.ctx().copy_text(ed.tokens.stylesheet(&all, false));
            }
            if ui
                .add_enabled(!all.is_empty(), egui::Button::new("Copy all as Tailwind"))
                .clicked()
            {
                ui.ctx().copy_text(ed.tokens.stylesheet(&all, true));
            }
            if ui
                .add_enabled(ed.can_paste_tokens(), egui::Button::new("Paste tokens"))
                .clicked()
            {
                st.tokens.selected = ed.paste_tokens();
            }
            ui.separator();
            super::systems::menu(ui, ed, st);
        });
    });
    ui.add_space(4.0);
    super::systems::header(ui, ed, st);
    super::systems::dialog(ui.ctx(), ed, st);
    if ed.tokens.is_empty() {
        ui.label(
            RichText::new(
                "No tokens yet. Tokens are named colors and sizes that every page can share. \
                 Add one with +, or use a published design system from the menu.",
            )
            .size(theme::SECONDARY)
            .color(p.secondary),
        );
        return;
    }
    let search = st.tokens.search.trim().to_lowercase();
    let matches = |t: &Token| {
        search.is_empty()
            || t.name.to_lowercase().contains(&search)
            || ed.tokens.css_value(t).to_lowercase().contains(&search)
    };
    // This project's own tokens first, then each design system's.
    let mut sources: Vec<Option<SystemLink>> = vec![None];
    sources.extend(ed.tokens.systems().iter().cloned().map(Some));
    let groups: Vec<(Option<SystemLink>, Kinds)> = sources
        .into_iter()
        .map(|sys| {
            let id = sys.as_ref().map(|s| s.id.clone());
            let kinds = TokenKind::ALL
                .into_iter()
                .map(|k| {
                    let list = ed
                        .tokens
                        .of_kind(k)
                        .filter(|t| t.system == id && matches(t));
                    (k, list.cloned().collect::<Vec<Token>>())
                })
                .filter(|(_, list)| !list.is_empty())
                .collect();
            (sys, kinds)
        })
        .collect();
    let visible: Vec<String> = groups
        .iter()
        .flat_map(|(_, kinds)| {
            kinds
                .iter()
                .flat_map(|(_, l)| l.iter().map(|t| t.id.clone()))
        })
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("tokens")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (sys, kinds) in groups {
                if let Some(link) = &sys {
                    ui.push_id(&link.id, |ui| {
                        super::systems::system_header(ui, ed, st, link)
                    });
                }
                for (kind, list) in kinds {
                    ui.add_space(6.0);
                    super::section(ui, kind.label());
                    for t in &list {
                        ui.push_id(&t.id, |ui| row(ui, ed, st, t, &visible));
                    }
                }
            }
        });
}

/// How a token is labelled outside its group: a design system's token by its CSS name.
fn label(ed: &Editor, t: &Token) -> String {
    if t.system.is_some() {
        ed.tokens.slug(t)
    } else {
        t.name.clone()
    }
}

fn row(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, t: &Token, visible: &[String]) {
    let p = theme::pal(ui);
    let (r, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::click(),
    );
    let selected = st.tokens.selected.contains(&t.id);
    if selected {
        ui.painter().rect_filled(r, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.hover);
    }
    let mut x = r.left() + 6.0;
    if let Some(TokenValue::Color(c)) = ed.tokens.resolve(&t.id) {
        let dot =
            egui::Rect::from_center_size(egui::pos2(x + 7.0, r.center().y), egui::vec2(14.0, 14.0));
        ui.painter().rect_filled(dot, 4.0, c32(c));
        ui.painter().rect_stroke(
            dot,
            4.0,
            egui::Stroke::new(1.0, p.separator),
            egui::StrokeKind::Inside,
        );
        x += 20.0;
    }
    let value = ed.tokens.css_value(t);
    let vg = ui.painter().layout_no_wrap(
        value,
        egui::FontId::proportional(theme::SECONDARY),
        p.secondary,
    );
    let vw = vg.size().x.min(r.width() * 0.45);
    let clip = r.intersect(ui.clip_rect());
    ui.painter().with_clip_rect(clip).galley(
        egui::pos2(r.right() - 6.0 - vw, r.center().y - vg.size().y / 2.0),
        vg,
        p.secondary,
    );
    let name_clip = egui::Rect::from_min_max(
        egui::pos2(x, r.top()),
        egui::pos2(r.right() - vw - 12.0, r.bottom()),
    );
    let ng = ui.painter().layout_no_wrap(
        t.name.clone(),
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    ui.painter()
        .with_clip_rect(name_clip.intersect(ui.clip_rect()))
        .galley(egui::pos2(x, r.center().y - ng.size().y / 2.0), ng, p.text);
    if resp.clicked() {
        click(ui, ed, st, t, visible);
    }
    resp.context_menu(|ui| context_menu(ui, ed, st, t));
    let open = st.tokens.open.as_ref().is_some_and(|d| d.id == t.id);
    if open {
        fields(ui, ed, st, t);
    }
}

fn click(ui: &Ui, ed: &mut Editor, st: &mut UiState, t: &Token, visible: &[String]) {
    let mods = ui.input(|i| i.modifiers);
    let tk = &mut st.tokens;
    if mods.command {
        match tk.selected.iter().position(|s| *s == t.id) {
            Some(i) => {
                tk.selected.remove(i);
            }
            None => tk.selected.push(t.id.clone()),
        }
        return;
    }
    if mods.shift
        && let Some(anchor) = tk.selected.last()
        && let Some(a) = visible.iter().position(|v| v == anchor)
        && let Some(b) = visible.iter().position(|v| *v == t.id)
    {
        let (lo, hi) = (a.min(b), a.max(b));
        tk.selected = visible[lo..=hi].to_vec();
        return;
    }
    let was_open = tk.open.as_ref().is_some_and(|d| d.id == t.id);
    tk.selected = vec![t.id.clone()];
    tk.open = (!was_open).then(|| Draft::of(ed, t));
}

/// The selection when the clicked token is part of it, else just that token.
fn targets(st: &UiState, t: &Token) -> Vec<String> {
    if st.tokens.selected.contains(&t.id) {
        st.tokens.selected.clone()
    } else {
        vec![t.id.clone()]
    }
}

fn context_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, t: &Token) {
    let ids = targets(st, t);
    let n = ids.len();
    let plural = |verb: &str| {
        if n == 1 {
            verb.to_string()
        } else {
            format!("{verb} {n} tokens")
        }
    };
    if ui.button(plural("Duplicate")).clicked() {
        st.tokens.selected = ed.edit_tokens("Duplicate tokens", None, |s| s.duplicate(&ids));
        ui.close();
    }
    if ui.button(plural("Copy")).clicked() {
        ed.copy_tokens(&ids);
        ui.close();
    }
    if ui.button("Copy as CSS").clicked() {
        let chosen: Vec<&Token> = ed
            .tokens
            .sorted()
            .into_iter()
            .filter(|t| ids.contains(&t.id))
            .collect();
        ui.ctx().copy_text(ed.tokens.stylesheet(&chosen, false));
        ui.close();
    }
    if ui
        .add_enabled(ed.can_paste_tokens(), egui::Button::new("Paste"))
        .clicked()
    {
        st.tokens.selected = ed.paste_tokens();
        ui.close();
    }
    ui.separator();
    // `delete` skips design system tokens, so count only the rest.
    let deletable = ids
        .iter()
        .filter(|id| ed.tokens.locked(id).is_none())
        .count();
    let label = if deletable > 1 {
        format!("Delete {deletable} tokens")
    } else {
        "Delete".to_string()
    };
    if ui
        .add_enabled(deletable > 0, egui::Button::new(label))
        .on_disabled_hover_text("Design system tokens are read-only. Remove the system instead.")
        .clicked()
    {
        ed.edit_tokens("Delete tokens", None, |s| s.delete(&ids));
        st.tokens.selected.clear();
        st.tokens.open = None;
        ui.close();
    }
}

/// A design system's token: what it is, without fields to edit.
fn locked_fields(ui: &mut Ui, ed: &Editor, t: &Token) {
    let p = theme::pal(ui);
    ui.add_space(3.0);
    let small = |s: String| RichText::new(s).size(theme::SECONDARY).color(p.secondary);
    ui.label(small(ed.tokens.css_value(t)));
    if let Some(d) = &t.description {
        ui.label(small(d.clone()));
    }
    ui.label(small(ed.tokens.css_name(t)).monospace());
    ui.label(small(
        "Read-only. Alias it from a local token, or detach the system to edit it.".into(),
    ));
    ui.add_space(4.0);
}

fn fields(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, t: &Token) {
    if t.system.is_some() {
        return locked_fields(ui, ed, t);
    }
    let p = theme::pal(ui);
    let id = t.id.clone();
    let Some(d) = &mut st.tokens.open else { return };
    let field_w = ui.available_width() - 62.0;
    let label = |ui: &mut Ui, text: &str| {
        ui.add_sized(
            egui::vec2(54.0, 18.0),
            egui::Label::new(
                RichText::new(text)
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            ),
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
    if committed(ui, &name) && d.name != t.name {
        let r = ed.edit_tokens("Rename token", None, |s| {
            s.update(&id, Some(&d.name), None, None)
        });
        d.error = r.err();
    }

    let value = ui
        .horizontal(|ui| {
            label(ui, "Value");
            let mut w = field_w;
            if let (TokenKind::Color, Some(TokenValue::Color(c))) =
                (t.kind, &ed.tokens.resolve(&id))
            {
                let mut c2 = c32(*c);
                let r = ui.color_edit_button_srgba(&mut c2);
                if r.changed() {
                    ed.edit_tokens("Edit token", Some(&id), |s| {
                        s.set_value(&id, TokenValue::Color(from32(c2)))
                    });
                    d.value = ed
                        .tokens
                        .get(&id)
                        .map_or(String::new(), |t| ed.tokens.css_value(t));
                }
                if r.drag_stopped() || r.lost_focus() {
                    ed.history.seal();
                }
                w -= 30.0;
            }
            ui.add(egui::TextEdit::singleline(&mut d.value).desired_width(w))
        })
        .inner;
    if committed(ui, &value) && d.value != ed.tokens.css_value(t) {
        let r = ed.edit_tokens("Edit token", None, |s| {
            s.update(&id, None, Some(&d.value), None)
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
    if committed(ui, &description) && d.description != t.description.clone().unwrap_or_default() {
        let r = ed.edit_tokens("Describe token", None, |s| {
            s.update(&id, None, None, Some(&d.description))
        });
        d.error = r.err();
    }

    // Outside the field being typed in, show the token as it is now: undo or an agent may
    // have changed it, and stale text would be written back when the field next lost focus.
    if let Some(now) = ed.tokens.get(&id) {
        if !name.has_focus() {
            d.name = now.name.clone();
        }
        if !value.has_focus() {
            d.value = ed.tokens.css_value(now);
        }
        if !description.has_focus() {
            d.description = now.description.clone().unwrap_or_default();
        }
    }

    ui.label(
        RichText::new(ed.tokens.css_name(t))
            .monospace()
            .size(theme::SECONDARY)
            .color(p.secondary),
    );
    if let Some(e) = &d.error {
        ui.label(
            RichText::new(e)
                .size(theme::SECONDARY)
                .color(egui::Color32::from_rgb(0xd9, 0x3a, 0x3a)),
        );
    }
    ui.add_space(4.0);
}

/// The field gave up focus with an edit to keep. Escape discards the edit instead. egui
/// reports a lost focus for two frames, so callers check that the text really differs.
fn committed(ui: &Ui, r: &egui::Response) -> bool {
    r.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape))
}

/// A small button next to a property: lit when a token owns the property, and a click opens
/// the picker. It stays out of the way when the project has no token of the right kind.
pub fn picker(ui: &mut Ui, ed: &mut Editor, ids: &[NodeId], prop: TokenProp) {
    let Some(first) = ids.first().and_then(|id| ed.doc.get(*id)) else {
        return;
    };
    let bound = ed.bound_token(first, prop).cloned();
    if bound.is_none() && ed.tokens.of_kind(prop.kind()).next().is_none() {
        return;
    }
    let tip = match &bound {
        Some(t) => format!("Token: {}", label(ed, t)),
        None => "Apply a token".into(),
    };
    let button = ui.push_id(prop, |ui| {
        theme::icon_button(ui, Icon::Token, 20.0, bound.is_some()).on_hover_text(tip)
    });
    let button = button.inner;
    if button.clicked() {
        ui.data_mut(|d| {
            d.remove_temp::<String>(search_id());
            d.insert_temp(search_id().with("focus"), true);
        });
    }
    Popup::from_toggle_button_response(&button)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(190.0);
            picker_list(ui, ed, ids, prop, bound.as_ref());
        });
}

/// Only one picker is open at a time, so they share their search text.
fn search_id() -> egui::Id {
    egui::Id::new("token-picker-search")
}

fn picker_list(
    ui: &mut Ui,
    ed: &mut Editor,
    ids: &[NodeId],
    prop: TokenProp,
    bound: Option<&Token>,
) {
    let p = theme::pal(ui);
    let mut search: String = ui.data_mut(|d| d.get_temp(search_id())).unwrap_or_default();
    // Before the field runs: Backspace on the last letter edits the search, it doesn't detach.
    let was_empty = search.is_empty();
    let field = ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text("Search tokens")
            .desired_width(180.0),
    );
    // Read before refocusing: Enter gives up focus, and taking it back would hide that.
    let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let opened = ui.data_mut(|d| d.remove_temp::<bool>(search_id().with("focus")));
    if !submitted && (opened.is_some() || ui.memory(|m| m.focused().is_none())) {
        field.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(search_id(), search.clone()));
    let detach = bound.is_some()
        && (ui
            .add(egui::Button::new("Detach").shortcut_text("⌫"))
            .clicked()
            || (was_empty && ui.input(|i| i.key_pressed(egui::Key::Backspace))));
    if detach {
        ed.detach_token(ids, prop);
        ui.close();
        return;
    }
    ui.separator();
    let list: Vec<Token> = ed
        .tokens
        .of_kind(prop.kind())
        .filter(|t| matches_search(&search, &label(ed, t), &ed.tokens.css_value(t)))
        .cloned()
        .collect();
    if submitted && let Some(t) = list.first() {
        ed.bind_token(ids, prop, &t.id);
        ui.close();
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("picker")
        .max_height(220.0)
        .show(ui, |ui| {
            for t in &list {
                let on = bound.is_some_and(|b| b.id == t.id);
                let value = ed.tokens.css_value(t);
                let swatch = match ed.tokens.resolve(&t.id) {
                    Some(TokenValue::Color(c)) => Some(c32(c)),
                    _ => None,
                };
                let r = ui.add(
                    egui::Button::selectable(on, label(ed, t))
                        .shortcut_text(RichText::new(value).color(p.secondary))
                        .min_size(egui::vec2(180.0, 22.0)),
                );
                if let Some(c) = swatch {
                    let dot = egui::Rect::from_center_size(
                        egui::pos2(r.rect.right() - 8.0, r.rect.center().y),
                        egui::vec2(10.0, 10.0),
                    );
                    ui.painter().rect_filled(dot, 3.0, c);
                }
                if r.clicked() {
                    ed.bind_token(ids, prop, &t.id);
                    ui.close();
                }
            }
            if list.is_empty() {
                ui.label(
                    RichText::new("No matching tokens")
                        .size(theme::SECONDARY)
                        .color(p.secondary),
                );
            }
        });
}

/// Search matches the start of the value, the name, or any word in the name.
fn matches_search(search: &str, name: &str, value: &str) -> bool {
    let q = search.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    value.to_lowercase().starts_with(&q)
        || name.starts_with(&q)
        || name
            .split(|c: char| c.is_whitespace() || matches!(c, ':' | '-' | '_'))
            .any(|w| w.starts_with(&q))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Color, Node, NodeKind};

    /// Runs a widget for real: each `step` is the input of one frame.
    pub(in crate::ui) struct Harness {
        pub(in crate::ui) ctx: egui::Context,
        pub(in crate::ui) ed: Editor,
        st: UiState,
    }

    pub(in crate::ui) fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    pub(in crate::ui) fn typed(s: &str) -> egui::Event {
        egui::Event::Text(s.into())
    }

    pub(in crate::ui) fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    impl Harness {
        pub(in crate::ui) fn new() -> Harness {
            Harness {
                ctx: egui::Context::default(),
                ed: Editor::default(),
                st: UiState::default(),
            }
        }

        pub(in crate::ui) fn step(
            &mut self,
            events: Vec<egui::Event>,
            f: &mut dyn FnMut(&mut Ui, &mut Editor, &mut UiState),
        ) -> egui::Rect {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let (ed, st) = (&mut self.ed, &mut self.st);
            let mut rect = egui::Rect::NOTHING;
            let mut out = self.ctx.run_ui(input, |ui| {
                f(ui, ed, st);
                rect = ui.min_rect();
            });
            out.textures_delta.clear(); // no renderer here to take them
            rect
        }

        fn rect_with_tokens(&mut self) -> NodeId {
            let id = self.ed.edit("add", |tx| {
                let id = tx.alloc_id();
                tx.insert(
                    Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0)),
                    None,
                    None,
                )
            });
            self.ed.edit_tokens("add", None, |t| {
                t.create(TokenKind::Color, "brand", "#ff0000", None)
                    .unwrap();
                t.create(TokenKind::Color, "accent", "#00ff00", None)
                    .unwrap();
            });
            id
        }
    }

    #[test]
    fn the_picker_opens_filters_and_applies_with_the_keyboard() {
        let mut h = Harness::new();
        let id = h.rect_with_tokens();
        let mut f =
            |ui: &mut Ui, ed: &mut Editor, _: &mut UiState| picker(ui, ed, &[id], TokenProp::Fill);
        h.step(vec![], &mut f);
        let at = h.step(vec![], &mut f).center();
        h.step(vec![egui::Event::PointerMoved(at)], &mut f);
        h.step(vec![pointer(at, true)], &mut f);
        h.step(vec![pointer(at, false)], &mut f);
        assert!(Popup::is_any_open(&h.ctx), "the click opens the picker");
        h.step(vec![], &mut f);
        h.step(vec![typed("acc")], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        let n = h.ed.doc.get(id).unwrap();
        assert_eq!(n.style.fill(), Some(Color::rgb(0, 255, 0)));
        assert!(n.tokens.contains_key(&TokenProp::Fill));
        assert!(!Popup::is_any_open(&h.ctx), "applying closes it");
    }

    #[test]
    fn backspace_detaches_only_from_an_empty_search() {
        let mut h = Harness::new();
        let id = h.rect_with_tokens();
        let token = h.ed.tokens.by_css_name("brand").unwrap().id.clone();
        h.ed.bind_token(&[id], TokenProp::Fill, &token);
        let mut f = |ui: &mut Ui, ed: &mut Editor, _: &mut UiState| {
            let bound = ed.tokens.get(&token).cloned();
            ui.push_id("list", |ui| {
                picker_list(ui, ed, &[id], TokenProp::Fill, bound.as_ref())
            });
        };
        h.step(vec![], &mut f);
        h.step(vec![typed("a")], &mut f);
        h.step(vec![key(egui::Key::Backspace)], &mut f);
        h.step(vec![], &mut f);
        let n = h.ed.doc.get(id).unwrap();
        assert!(
            n.tokens.contains_key(&TokenProp::Fill),
            "that was the search"
        );
        h.step(vec![key(egui::Key::Backspace)], &mut f);
        let n = h.ed.doc.get(id).unwrap();
        assert!(!n.tokens.contains_key(&TokenProp::Fill));
        assert_eq!(
            n.style.fill(),
            Some(Color::rgb(255, 0, 0)),
            "keeps the value"
        );
    }

    fn spacing_fields() -> (Harness, String) {
        let mut h = Harness::new();
        let id = h.ed.edit_tokens("add", None, |t| {
            t.create(TokenKind::Spacing, "gap", "8px", None).unwrap()
        });
        let ed = &h.ed;
        h.st.tokens.open(ed, &id);
        (h, id)
    }

    fn fields_frame(id: &str) -> impl FnMut(&mut Ui, &mut Editor, &mut UiState) {
        let id = id.to_string();
        move |ui, ed, st| {
            let t = ed.tokens.get(&id).cloned().unwrap();
            fields(ui, ed, st, &t);
        }
    }

    /// Tab from nothing to the name field, then to the value field.
    fn focus_value(h: &mut Harness, f: &mut dyn FnMut(&mut Ui, &mut Editor, &mut UiState)) {
        h.step(vec![], f);
        h.step(vec![key(egui::Key::Tab)], f);
        h.step(vec![], f);
        h.step(vec![key(egui::Key::Tab)], f);
        h.step(vec![], f);
    }

    /// Replace the text of the focused field, whose caret is at the end.
    fn retype(h: &mut Harness, f: &mut dyn FnMut(&mut Ui, &mut Editor, &mut UiState), s: &str) {
        for _ in 0..3 {
            h.step(vec![key(egui::Key::Backspace)], f);
        }
        h.step(vec![typed(s)], f);
    }

    fn value(h: &Harness, id: &str) -> String {
        h.ed.tokens.css_value(h.ed.tokens.get(id).unwrap())
    }

    #[test]
    fn enter_saves_a_field() {
        let (mut h, id) = spacing_fields();
        let mut f = fields_frame(&id);
        focus_value(&mut h, &mut f);
        retype(&mut h, &mut f, "20px");
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        assert_eq!(value(&h, &id), "20px");
    }

    #[test]
    fn escape_discards_a_field() {
        let (mut h, id) = spacing_fields();
        let mut f = fields_frame(&id);
        focus_value(&mut h, &mut f);
        retype(&mut h, &mut f, "20px");
        h.step(vec![key(egui::Key::Escape)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        assert_eq!(value(&h, &id), "8px");
        assert_eq!(h.st.tokens.open.as_ref().unwrap().value, "8px");
    }

    #[test]
    fn a_bad_value_reports_and_changes_nothing() {
        let (mut h, id) = spacing_fields();
        let mut f = fields_frame(&id);
        focus_value(&mut h, &mut f);
        retype(&mut h, &mut f, "wide");
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![], &mut f);
        assert_eq!(value(&h, &id), "8px");
        let d = h.st.tokens.open.as_ref().unwrap();
        let e = d.error.as_ref().expect("the error stays on screen");
        assert!(e.contains("Invalid value"), "{e}");
    }

    #[test]
    fn changes_from_elsewhere_replace_stale_field_text() {
        let (mut h, id) = spacing_fields();
        let mut f = fields_frame(&id);
        h.step(vec![], &mut f);
        h.ed.edit_tokens("agent", None, |t| {
            t.update(&id, Some("gutter"), Some("32px"), None)
        })
        .unwrap();
        h.step(vec![], &mut f);
        let d = h.st.tokens.open.as_ref().unwrap();
        assert_eq!((d.name.as_str(), d.value.as_str()), ("gutter", "32px"));
    }

    #[test]
    fn search_matches_word_starts_and_values() {
        assert!(matches_search("", "anything", "#fff"));
        assert!(matches_search("prim", "color-primary", "#000000"));
        assert!(matches_search("PRIM", "Primary Blue", "#000000"));
        assert!(matches_search("#00", "color-primary", "#000000"));
        assert!(!matches_search("rimary", "color-primary", "#000000"));
    }
}
