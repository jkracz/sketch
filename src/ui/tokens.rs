//! The Tokens tab of the left sidebar: add, edit, duplicate, copy and delete design tokens,
//! with a column per mode, and the design systems this project uses listed below its own
//! tokens (`ui::systems`). Also the token picker the inspector opens next to a bindable
//! property.

use super::{UiState, c32, from32};
use crate::editor::Editor;
use crate::model::{Mode, NodeId, SystemLink, Token, TokenKind, TokenProp, TokenValue};
use crate::theme::{self, Icon};
use egui::{Popup, PopupCloseBehavior, RichText, Ui};

#[derive(Default)]
pub struct TokensUi {
    pub(super) search: String,
    pub(super) styles: super::text_styles::StylesUi,
    /// Selected token ids, for the context menu's multi-token actions.
    selected: Vec<String>,
    /// The token whose fields are shown under its row.
    open: Option<Draft>,
    renaming: Option<(String, String)>,
}

/// Text being edited for the open token. It reaches the token when the field loses focus.
struct Draft {
    id: String,
    name: String,
    /// The first mode's value.
    value: String,
    /// Values for the file's other modes, in order. Empty follows the first.
    modes: Vec<String>,
    description: String,
    error: Option<String>,
}

impl TokensUi {
    /// Show this token's fields under its row.
    pub fn open(&mut self, ed: &Editor, id: &str) {
        self.selected = vec![id.to_string()];
        self.open = ed.tokens.get(id).map(|t| Draft::of(ed, t));
    }

    /// Show this text style's fields under its row.
    pub fn open_style(&mut self, ed: &Editor, id: &str) {
        self.styles.open(ed, id);
    }
}

impl Draft {
    fn of(ed: &Editor, t: &Token) -> Draft {
        Draft {
            id: t.id.clone(),
            name: t.name.clone(),
            value: ed.tokens.css_value(t),
            modes: (ed.tokens.modes().iter().skip(1))
                .map(|m| own_css(ed, t, m))
                .collect(),
            description: t.description.clone().unwrap_or_default(),
            error: None,
        }
    }
}

/// `t`'s own value in mode `m` as CSS, or empty when it follows the first mode.
fn own_css(ed: &Editor, t: &Token, m: &Mode) -> String {
    t.modes
        .get(&m.id)
        .map_or(String::new(), |v| ed.tokens.value_css(t.kind, v))
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
            ui.separator();
            super::text_styles::add_entry(ui, ed, st);
        });
        let more = theme::icon_button(ui, Icon::More, 20.0, false).on_hover_text("More");
        Popup::menu(&more).show(|ui| {
            let all = ed.tokens.sorted();
            let styles: Vec<&crate::model::TextStyleDef> = ed.tokens.styles().iter().collect();
            let sheet = |tailwind| {
                let mut css = ed.tokens.stylesheet(&all, tailwind);
                css.push_str(&ed.tokens.style_sheet(&styles, tailwind));
                css
            };
            let any = !ed.tokens.is_blank();
            if ui
                .add_enabled(any, egui::Button::new("Copy all as CSS"))
                .clicked()
            {
                ui.ctx().copy_text(sheet(false));
            }
            if ui
                .add_enabled(any, egui::Button::new("Copy all as Tailwind"))
                .clicked()
            {
                ui.ctx().copy_text(sheet(true));
            }
            if ui
                .add_enabled(ed.can_paste_tokens(), egui::Button::new("Paste tokens"))
                .clicked()
            {
                st.tokens.selected = ed.paste_tokens();
            }
            if ui.button("Add mode").clicked() {
                ed.edit_tokens("Add mode", None, |t| t.add_mode());
            }
            ui.separator();
            super::systems::menu(ui, ed, st);
        });
    });
    ui.add_space(4.0);
    super::systems::header(ui, ed, st);
    super::systems::dialog(ui.ctx(), ed, st);
    if ed.tokens.is_blank() {
        ui.label(
            RichText::new(
                "No tokens yet. Tokens are named colors and sizes that every page can share, \
                 and text styles name a set of text settings. Add one with +, or use a \
                 published design system from the menu.",
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
                if !kinds.is_empty() {
                    ui.push_id(("modes", sys.as_ref().map(|s| &s.id)), |ui| {
                        mode_header(ui, ed, st, sys.as_ref())
                    });
                }
                for (kind, list) in kinds {
                    ui.add_space(6.0);
                    super::section(ui, kind.label());
                    for t in &list {
                        ui.push_id(&t.id, |ui| row(ui, ed, st, t, &visible));
                    }
                }
                let system = sys.as_ref().map(|s| s.id.as_str());
                let styles = super::text_styles::matching(ed, system, &search);
                super::text_styles::section(ui, ed, st, &styles);
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

fn columns(r: egui::Rect, n: usize) -> Vec<egui::Rect> {
    let w = (r.width() * 0.55 / n as f32).min(84.0);
    (0..n)
        .map(|i| {
            let right = r.right() - 6.0 - (n - 1 - i) as f32 * w;
            egui::Rect::from_min_max(
                egui::pos2(right - w + 4.0, r.top()),
                egui::pos2(right, r.bottom()),
            )
        })
        .collect()
}

fn mode_header(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, sys: Option<&SystemLink>) {
    let modes: Vec<Mode> = match sys {
        Some(l) => l.modes.clone(),
        None => ed.tokens.modes().to_vec(),
    };
    if modes.len() < 2 {
        return;
    }
    let p = theme::pal(ui);
    let (r, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    for (m, col) in modes.iter().zip(columns(r, modes.len())) {
        let resp = ui.interact(col, ui.id().with(&m.id), egui::Sense::click());
        let color = if resp.hovered() { p.text } else { p.secondary };
        let g = ui.painter().layout_no_wrap(
            m.name.clone(),
            egui::FontId::proportional(theme::SECONDARY),
            color,
        );
        ui.painter()
            .with_clip_rect(col.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(col.left(), col.center().y - g.size().y / 2.0),
                g,
                color,
            );
        if sys.is_some() {
            resp.on_hover_text("This design system's modes. Pick one on a frame.");
            continue;
        }
        let resp = resp.on_hover_text("Rename or remove this mode");
        Popup::menu(&resp).show(|ui| mode_menu(ui, ed, st, m));
    }
}

fn mode_menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, m: &Mode) {
    let tk = &mut st.tokens;
    if tk.renaming.as_ref().is_none_or(|(id, _)| *id != m.id) {
        tk.renaming = Some((m.id.clone(), m.name.clone()));
    }
    let Some((_, text)) = &mut tk.renaming else {
        return;
    };
    let field = ui.add(egui::TextEdit::singleline(text).desired_width(140.0));
    if committed(ui, &field) && *text != m.name {
        let name = text.clone();
        if let Err(e) = ed.edit_tokens("Rename mode", None, |t| t.rename_mode(&m.id, &name)) {
            ed.status = e;
        }
        tk.renaming = None;
    }
    let last = ed.tokens.modes().len() < 2;
    if ui
        .add_enabled(!last, egui::Button::new("Remove mode"))
        .on_hover_text("Its values go. Frames that picked it follow the frame they sit in.")
        .clicked()
    {
        if let Err(e) = ed.edit_tokens("Remove mode", None, |t| t.delete_mode(&m.id)) {
            ed.status = e;
        }
        tk.renaming = None;
        tk.open = None;
        ui.close();
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
    let modes = ed.tokens.modes_of(t).to_vec();
    if modes.len() > 1 {
        mode_cells(ui, ed, t, r, &modes);
        let name_clip = egui::Rect::from_min_max(
            egui::pos2(r.left() + 6.0, r.top()),
            egui::pos2(columns(r, modes.len())[0].left() - 6.0, r.bottom()),
        );
        let ng = ui.painter().layout_no_wrap(
            t.name.clone(),
            egui::FontId::proportional(theme::BODY),
            p.text,
        );
        ui.painter()
            .with_clip_rect(name_clip.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(name_clip.left(), r.center().y - ng.size().y / 2.0),
                ng,
                p.text,
            );
        return row_end(ui, ed, st, t, visible, &resp);
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
    row_end(ui, ed, st, t, visible, &resp);
}

fn mode_cells(ui: &Ui, ed: &Editor, t: &Token, r: egui::Rect, modes: &[Mode]) {
    let p = theme::pal(ui);
    for (i, (m, col)) in modes.iter().zip(columns(r, modes.len())).enumerate() {
        let own = i == 0 || t.modes.contains_key(&m.id);
        let color = if own { p.secondary } else { p.tertiary };
        let mut x = col.left();
        if let Some(TokenValue::Color(c)) = ed.tokens.resolve_in(&t.id, Some(&m.name)) {
            let dot = egui::Rect::from_min_size(
                egui::pos2(x, col.center().y - 6.0),
                egui::vec2(12.0, 12.0),
            );
            ui.painter().rect_filled(dot, 3.0, c32(c));
            ui.painter().rect_stroke(
                dot,
                3.0,
                egui::Stroke::new(1.0, p.separator),
                egui::StrokeKind::Inside,
            );
            x += 16.0;
        }
        let g = ui.painter().layout_no_wrap(
            ed.tokens.css_value_in(t, Some(&m.name)),
            egui::FontId::proportional(theme::SECONDARY),
            color,
        );
        ui.painter()
            .with_clip_rect(col.intersect(ui.clip_rect()))
            .galley(egui::pos2(x, col.center().y - g.size().y / 2.0), g, color);
    }
}

fn row_end(
    ui: &mut Ui,
    ed: &mut Editor,
    st: &mut UiState,
    t: &Token,
    visible: &[String],
    resp: &egui::Response,
) {
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
    for (m, v) in ed.tokens.own_mode_values(t) {
        ui.label(small(format!("{m}: {v}")));
    }
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

    let modes = ed.tokens.modes().to_vec();
    let first = match modes.len() {
        1 => "Value",
        _ => modes[0].name.as_str(),
    };
    let value = ui
        .horizontal(|ui| {
            label(ui, first);
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

    d.modes.resize(modes.len().saturating_sub(1), String::new());
    let mut mode_fields = Vec::new();
    for (i, m) in modes.iter().enumerate().skip(1) {
        let text = &mut d.modes[i - 1];
        let r = ui
            .horizontal(|ui| {
                label(ui, &m.name);
                let mut w = field_w;
                if let (TokenKind::Color, Some(TokenValue::Color(c))) =
                    (t.kind, &ed.tokens.resolve_in(&id, Some(&m.name)))
                {
                    let mut c2 = c32(*c);
                    let r = ui.color_edit_button_srgba(&mut c2);
                    if r.changed() {
                        let key = format!("{id}/{}", m.id);
                        ed.edit_tokens("Edit token", Some(&key), |s| {
                            s.set_value_in(&id, &m.id, TokenValue::Color(from32(c2)))
                        });
                        *text = (ed.tokens.get(&id)).map_or(String::new(), |t| own_css(ed, t, m));
                    }
                    if r.drag_stopped() || r.lost_focus() {
                        ed.history.seal();
                    }
                    w -= 30.0;
                }
                let hint = format!("Same as {}", modes[0].name);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .desired_width(w),
                )
            })
            .inner;
        let now = ed
            .tokens
            .get(&id)
            .map(|t| own_css(ed, t, m))
            .unwrap_or_default();
        if committed(ui, &r) && *text != now {
            let raw = Some(text.trim()).filter(|s| !s.is_empty());
            let res = ed.edit_tokens("Edit token", None, |s| s.update_in_mode(&id, &m.id, raw));
            d.error = res.err();
        }
        mode_fields.push(r);
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
        for (i, r) in mode_fields.iter().enumerate() {
            if !r.has_focus() {
                d.modes[i] = own_css(ed, now, &modes[i + 1]);
            }
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
pub(super) fn committed(ui: &Ui, r: &egui::Response) -> bool {
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
        pub(in crate::ui) st: UiState,
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
    fn the_dark_field_sets_and_clears_the_dark_value() {
        let (mut h, id) = spacing_fields();
        let mut f = fields_frame(&id);
        focus_value(&mut h, &mut f);
        h.step(vec![key(egui::Key::Tab)], &mut f);
        h.step(vec![], &mut f);
        h.step(vec![typed("12px")], &mut f);
        h.step(vec![key(egui::Key::Enter)], &mut f);
        h.step(vec![], &mut f);
        let dark = |h: &Harness| h.ed.tokens.resolve_in(&id, Some("Dark"));
        assert_eq!(dark(&h), Some(TokenValue::Number(12.0)));
        assert_eq!(value(&h, &id), "8px", "Light keeps its value");
        h.ed.edit_tokens("clear", None, |t| t.update_in_mode(&id, "dark", None))
            .unwrap();
        h.step(vec![], &mut f);
        assert_eq!(
            dark(&h),
            Some(TokenValue::Number(8.0)),
            "Dark follows Light"
        );
        assert_eq!(
            h.st.tokens.open.as_ref().unwrap().modes,
            vec![String::new()]
        );
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
