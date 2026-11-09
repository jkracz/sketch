//! Design systems in the Tokens tab: publishing this project's tokens, and using, removing
//! or detaching other projects' systems. The rules live in `crate::systems`.

use super::UiState;
use crate::editor::Editor;
use crate::editor::systems::plural;
use crate::model::SystemLink;
use crate::project::Store;
use crate::systems::{self, PublishRequest, Published};
use crate::theme::{self, Icon};
use egui::{Popup, RichText, Ui};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Reading the registry means reading every project's manifest, so the panel keeps a copy
/// and refreshes it now and then, or right after it changes something.
const REFRESH: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct SystemsUi {
    store: Option<Store>,
    /// Id and name of the open project.
    project: Option<(String, String)>,
    snapshot: Snapshot,
    refreshed: Option<Instant>,
    dialog: Option<PublishDraft>,
}

#[derive(Default)]
struct Snapshot {
    /// Systems this project could use.
    available: Vec<Published>,
    /// What this project last published, as the registry has it.
    own: Option<Published>,
    /// Systems this project uses whose source is unpublished, trashed or deleted.
    unavailable: HashSet<String>,
}

/// The publish dialog's fields.
struct PublishDraft {
    name: String,
    prefix: String,
    description: String,
    notes: String,
    /// The prefix can't change after the first publish.
    prefix_fixed: bool,
    /// Other projects that will update.
    users: usize,
    error: Option<String>,
}

impl SystemsUi {
    /// Called every frame with the store and open project, before the UI runs.
    pub fn set_context(&mut self, store: &Store, project: Option<(&str, &str)>) {
        let project = project.map(|(id, name)| (id.to_string(), name.to_string()));
        if self.project.as_ref().map(|p| &p.0) != project.as_ref().map(|p| &p.0) {
            self.refreshed = None;
            self.dialog = None;
        }
        self.project = project;
        if self.store.as_ref().map(|s| &s.root) != Some(&store.root) {
            self.store = Some(store.clone());
            self.refreshed = None;
        }
    }

    /// Read the registry again on the next frame.
    pub fn invalidate(&mut self) {
        self.refreshed = None;
    }

    fn refresh(&mut self, ed: &Editor) {
        if self.refreshed.is_some_and(|t| t.elapsed() < REFRESH) {
            return;
        }
        self.refreshed = Some(Instant::now());
        let Some(store) = &self.store else { return };
        let available = systems::available(store);
        let reg = systems::Registry::in_store(store);
        let own = ed.published.as_ref().and_then(|l| reg.get(&l.id));
        let unavailable = ed
            .tokens
            .systems()
            .iter()
            .filter(|s| !available.iter().any(|p| p.id == s.id))
            .map(|s| s.id.clone())
            .collect();
        self.snapshot = Snapshot {
            available,
            own,
            unavailable,
        };
    }

    /// Whether the open project's tokens or settings differ from its last publish.
    fn unpublished_changes(&self, ed: &Editor) -> bool {
        match (&ed.published, &self.snapshot.own) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(l), Some(p)) => p.tokens != ed.tokens.publishable() || p.link() != *l,
        }
    }

    pub fn is_unavailable(&self, id: &str) -> bool {
        self.snapshot.unavailable.contains(id)
    }
}

/// The publish state above the token list, when this project has published.
pub fn header(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    st.systems.refresh(ed);
    let Some(l) = ed.published.clone() else {
        return;
    };
    let p = theme::pal(ui);
    let live = st.systems.snapshot.own.is_some();
    let state = if !live {
        "Not published"
    } else if st.systems.unpublished_changes(ed) {
        "Unpublished changes"
    } else {
        "Up to date"
    };
    let text = if live {
        format!("v{} · {state}", l.release)
    } else {
        state.to_string()
    };
    // Right to left, so a long name truncates instead of widening the sidebar.
    let size = egui::vec2(ui.available_width(), 20.0);
    let rtl = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(size, rtl, |ui| {
        if state != "Up to date" && ui.small_button("Publish…").clicked() {
            open_publish(ed, st);
        }
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            )
            .truncate(),
        )
        .on_hover_text(format!("This file publishes {} (--{}-).", l.name, l.prefix));
    });
    ui.add_space(2.0);
}

/// Entries for the Tokens tab's More menu.
pub fn menu(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    st.systems.refresh(ed);
    let has_local = !ed.tokens.publishable().is_empty();
    let label = if ed.published.is_some() {
        "Publish changes…"
    } else {
        "Publish as design system…"
    };
    if ui
        .add_enabled(
            has_local && st.systems.project.is_some(),
            egui::Button::new(label),
        )
        .on_disabled_hover_text("Add a token first. Only projects can publish.")
        .clicked()
    {
        open_publish(ed, st);
        ui.close();
    }
    if st.systems.snapshot.own.is_some() && ui.button("Unpublish design system").clicked() {
        if let Some(store) = &st.systems.store {
            ed.status = systems::unpublish(store, ed).unwrap_or_else(|e| e);
        }
        st.systems.invalidate();
        ui.close();
    }
    let own = ed.published.as_ref().map(|l| l.id.clone());
    let choices: Vec<Published> = st
        .systems
        .snapshot
        .available
        .iter()
        .filter(|p| Some(&p.id) != own.as_ref() && ed.tokens.system(&p.id).is_none())
        .cloned()
        .collect();
    ui.menu_button("Use design system", |ui| {
        if choices.is_empty() {
            ui.label(
                RichText::new("No other design systems are published.")
                    .size(theme::SECONDARY)
                    .color(theme::pal(ui).secondary),
            );
        }
        for p in &choices {
            let r = ui.button(format!("{}  ·  --{}-", p.name, p.prefix));
            let r = match &p.description {
                Some(d) => r.on_hover_text(d),
                None => r,
            };
            if r.clicked() {
                ed.status = match ed.use_system(p) {
                    Ok(()) => format!("Using {}.", p.name),
                    Err(e) => e,
                };
                st.systems.invalidate();
                ui.close();
            }
        }
    });
}

/// The heading of a design system's group of tokens, with its actions.
pub fn system_header(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, link: &SystemLink) {
    let p = theme::pal(ui);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        small_icon(ui, Icon::Lock, p.secondary);
        ui.label(super::title_text(&link.name));
        ui.label(
            RichText::new(format!("v{}", link.release))
                .size(theme::SECONDARY)
                .color(p.secondary),
        );
        let more = theme::icon_button(ui, Icon::More, 18.0, false).on_hover_text("Design system");
        Popup::menu(&more).show(|ui| {
            if ui
                .button("Detach")
                .on_hover_text("Make these tokens local. Layers keep their bindings.")
                .clicked()
            {
                if let Err(e) = ed.detach_system(&link.id) {
                    ed.status = e;
                } else {
                    ed.status = format!("Detached {}. Its tokens are local now.", link.name);
                }
                st.systems.invalidate();
                ui.close();
            }
            if ui
                .button("Remove")
                .on_hover_text("Stop using it. Layers keep their values.")
                .clicked()
            {
                ed.status = match ed.remove_system(&link.id) {
                    Ok(0) => format!("Removed {}.", link.name),
                    Ok(n) => format!(
                        "Removed {}. {} keep their values.",
                        link.name,
                        plural(n, "layer")
                    ),
                    Err(e) => e,
                };
                st.systems.invalidate();
                ui.close();
            }
        });
    });
    if st.systems.is_unavailable(&link.id) {
        ui.label(
            RichText::new("Source unavailable. This copy won't update.")
                .size(theme::SECONDARY)
                .color(p.secondary),
        );
    } else if let Some(d) = &link.description {
        ui.label(RichText::new(d).size(theme::SECONDARY).color(p.secondary));
    }
}

fn small_icon(ui: &mut Ui, icon: Icon, color: egui::Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    theme::paint_icon(ui.painter(), r, icon, color);
}

/// Open the publish dialog for the open project.
pub fn open_publish(ed: &Editor, st: &mut UiState) {
    let s = &mut st.systems;
    let Some((_, project_name)) = &s.project else {
        return;
    };
    let prev = ed.published.as_ref();
    let name = prev.map_or_else(|| project_name.clone(), |l| l.name.clone());
    let users = match (&s.store, prev) {
        (Some(store), Some(l)) => systems::users(store, &l.id),
        _ => 0,
    };
    s.dialog = Some(PublishDraft {
        prefix: prev.map_or_else(|| systems::default_prefix(&name), |l| l.prefix.clone()),
        name,
        description: prev.and_then(|l| l.description.clone()).unwrap_or_default(),
        notes: prev.and_then(|l| l.notes.clone()).unwrap_or_default(),
        prefix_fixed: prev.is_some(),
        users,
        error: None,
    });
}

/// The publish dialog, when open. It shows what the publish will change before it happens.
pub fn dialog(ctx: &egui::Context, ed: &mut Editor, st: &mut UiState) {
    if st.systems.dialog.is_none() {
        return;
    }
    let mut publish = false;
    let mut cancel = false;
    let own = st.systems.snapshot.own.clone();
    let modal = egui::Modal::new(egui::Id::new("publish-design-system")).show(ctx, |ui| {
        let p = theme::pal(ui);
        let Some(d) = &mut st.systems.dialog else {
            return;
        };
        ui.set_width(360.0);
        ui.label(super::title_text(if d.prefix_fixed {
            "Publish changes"
        } else {
            "Publish as design system"
        }));
        ui.label(
            RichText::new(
                "Other files can use this file's tokens. They get each new version when \
                 they open.",
            )
            .size(theme::SECONDARY)
            .color(p.secondary),
        );
        ui.add_space(6.0);
        let field = |ui: &mut Ui, label: &str, text: &mut String, enabled: bool, hint: &str| {
            ui.label(
                RichText::new(label)
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            );
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
        };
        field(ui, "Name", &mut d.name, true, "Brand");
        field(ui, "Prefix", &mut d.prefix, !d.prefix_fixed, "brand");
        if d.prefix_fixed {
            ui.label(
                RichText::new("The prefix is fixed after the first publish.")
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            );
        }
        field(
            ui,
            "Description",
            &mut d.description,
            true,
            "One line: what it's for",
        );
        ui.label(
            RichText::new("Usage notes (Markdown)")
                .size(theme::SECONDARY)
                .color(p.secondary),
        );
        ui.add(
            egui::TextEdit::multiline(&mut d.notes)
                .hint_text("How to use these tokens, for people and agents")
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);
        let prefix = d.prefix.trim();
        let c = systems::changes(own.as_ref(), prefix, &ed.tokens.publishable());
        let summary = if own.is_none() {
            format!("Publishes {}.", plural(c.added.len(), "token"))
        } else if c.is_empty() {
            "No token changes since the last publish.".to_string()
        } else {
            [
                ("Adds", &c.added),
                ("Changes", &c.changed),
                ("Removes", &c.removed),
            ]
            .into_iter()
            .filter(|(_, l)| !l.is_empty())
            .map(|(w, l)| format!("{w} {}.", systems::name_list(l, 20)))
            .collect::<Vec<_>>()
            .join(" ")
        };
        ui.label(RichText::new(summary).size(theme::SECONDARY));
        if d.users > 0 {
            ui.label(
                RichText::new(format!(
                    "{} will update when opened.",
                    plural(d.users, "file")
                ))
                .size(theme::SECONDARY)
                .color(p.secondary),
            );
        }
        if let Some(e) = &d.error {
            ui.label(
                RichText::new(e)
                    .size(theme::SECONDARY)
                    .color(egui::Color32::from_rgb(0xd9, 0x3a, 0x3a)),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Publish").clicked() {
                publish = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if modal.should_close() || cancel {
        st.systems.dialog = None;
        return;
    }
    if publish {
        run_publish(ed, st);
    }
}

fn run_publish(ed: &mut Editor, st: &mut UiState) {
    let s = &mut st.systems;
    let (Some(store), Some((id, name)), Some(d)) = (&s.store, &s.project, &mut s.dialog) else {
        return;
    };
    let req = PublishRequest {
        name: Some(&d.name),
        prefix: (!d.prefix_fixed).then_some(d.prefix.as_str()),
        description: Some(&d.description),
        notes: Some(&d.notes),
    };
    match systems::publish(store, ed, id, name, &req) {
        Ok(r) => {
            ed.status = r.message();
            s.dialog = None;
            s.invalidate();
        }
        Err(e) => d.error = Some(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TokenKind;
    use crate::session::Session;

    #[test]
    fn publishing_from_the_dialog_reports_and_refreshes() {
        let root = std::env::temp_dir().join(format!("sketch-rs-ui-{:016x}", fastrand::u64(..)));
        let store = Store::open(root).unwrap();
        let mut s = Session::new(store.clone());
        let mut ed = Editor::default();
        s.create(&mut ed, "Brand").unwrap();
        ed.edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "brand", "#ff0000", None)
        })
        .unwrap();
        let mut st = UiState::default();
        let m = &s.project.as_ref().unwrap().manifest;
        st.systems
            .set_context(&store, Some((m.id.as_str(), m.name.as_str())));
        open_publish(&ed, &mut st);
        assert_eq!(st.systems.dialog.as_ref().unwrap().prefix, "brand");
        run_publish(&mut ed, &mut st);
        assert!(st.systems.dialog.is_none());
        assert!(ed.status.starts_with("Published Brand v1"), "{}", ed.status);

        st.systems.refresh(&ed);
        assert!(!st.systems.unpublished_changes(&ed));
        let id = ed.tokens.by_css_name("--brand").unwrap().id.clone();
        ed.edit_tokens("edit", None, |t| t.update(&id, None, Some("#00ff00"), None))
            .unwrap();
        assert!(st.systems.unpublished_changes(&ed));

        // The prefix can't change, and the dialog says so.
        open_publish(&ed, &mut st);
        assert!(st.systems.dialog.as_ref().unwrap().prefix_fixed);
    }
}
