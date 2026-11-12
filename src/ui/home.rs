//! The projects home screen: a recents grid with thumbnails, folders, plus the trash.

use crate::project::{Entry, Folder, TrashEntry};
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq)]
pub enum HomeAction {
    New,
    Import,
    Open(PathBuf),
    Rename(PathBuf, String),
    Duplicate(PathBuf),
    /// Move a project into a folder, or to the top level when None.
    Move(PathBuf, Option<Folder>),
    /// Make a folder and name it, moving this project into it if given.
    NewFolder(Option<PathBuf>),
    RenameFolder(PathBuf, String),
    TrashFolder(PathBuf),
    Trash(PathBuf),
    Restore(PathBuf),
    DeleteForever(PathBuf),
    EmptyTrash,
}

#[derive(Default)]
pub struct HomeView {
    pub entries: Vec<Entry>,
    pub folders: Vec<Folder>,
    pub trash: Vec<TrashEntry>,
    pub show_trash: bool,
    /// Id of the folder being viewed. None shows Recents, every project at once.
    pub folder: Option<String>,
    /// The project or folder being renamed inline.
    pub renaming: Option<(PathBuf, String)>,
    /// Requests for the app, drained after each frame.
    pub actions: Vec<HomeAction>,
    /// "Now" for relative timestamps (unix ms); fixed in screenshots.
    pub now_ms: u64,
    pub root: PathBuf,
    thumbs: HashMap<PathBuf, (Option<SystemTime>, egui::TextureHandle)>,
}

const CARD_W: f32 = 232.0;
const THUMB_H: f32 = 148.0;

/// Drag payload: a project card on its way to a folder.
struct ProjectDrag {
    dir: PathBuf,
    name: String,
}

impl HomeView {
    /// Re-read the project list and trash from the store (keeps cached thumbnails).
    pub fn refresh(&mut self, store: &crate::project::Store, now_ms: u64) {
        self.root = store.root.clone();
        self.entries = store.list();
        self.folders = store.folders();
        self.trash = store.list_trash();
        self.now_ms = now_ms;
        if self.selected_folder().is_none() {
            self.folder = None;
        }
    }

    /// Show a folder that was just created, with its name ready to type over.
    pub fn name_new_folder(&mut self, f: Folder) {
        self.show_trash = false;
        self.folder = Some(f.id);
        self.renaming = Some((f.dir, f.name));
    }

    /// The folder being viewed, which New Project and Import put things in.
    pub fn selected_folder(&self) -> Option<&Folder> {
        let id = self.folder.as_ref()?;
        self.folders.iter().find(|f| f.id == *id)
    }

    /// The projects in the current view.
    fn visible(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| {
                self.folder.is_none() || e.folder.as_ref().map(|f| &f.id) == self.folder.as_ref()
            })
            .cloned()
            .collect()
    }

    fn folder_size(&self, f: &Folder) -> usize {
        self.entries
            .iter()
            .filter(|e| e.folder.as_ref().is_some_and(|c| c.id == f.id))
            .count()
    }

    pub fn load(store: &crate::project::Store, now_ms: u64) -> HomeView {
        let mut hv = HomeView::default();
        hv.refresh(store, now_ms);
        hv
    }

    fn thumb(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((t, h)) = self.thumbs.get(path)
            && *t == mtime
        {
            return Some(h.clone());
        }
        let img = image::open(path).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let h = ctx.load_texture(path.to_string_lossy(), ci, egui::TextureOptions::LINEAR);
        self.thumbs.insert(path.to_path_buf(), (mtime, h.clone()));
        Some(h)
    }
}

/// "Edited 5 minutes ago" style relative time.
pub fn relative_time(then_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => plural(s / 60, "minute"),
        3600..=86_399 => plural(s / 3600, "hour"),
        86_400..=2_591_999 => plural(s / 86_400, "day"),
        _ => plural(s / 2_592_000, "month"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

pub fn draw(ui: &mut Ui, hv: &mut HomeView, native_chrome: bool, translucent: bool) {
    let p = theme::pal(ui);
    egui::Panel::left("home-sidebar")
        .resizable(false)
        .exact_size(200.0)
        .frame(
            egui::Frame::NONE
                .fill(if translucent {
                    Color32::TRANSPARENT
                } else {
                    p.sidebar
                })
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ui, |ui| {
            if native_chrome {
                super::titlebar_strip(ui, theme::TOOLBAR_H - 8.0);
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Sketch")
                    .font(theme::semibold(theme::SECONDARY))
                    .color(p.secondary),
            );
            ui.add_space(4.0);
            let recents = sidebar_row(
                ui,
                Icon::Grid,
                "Recents",
                None,
                !hv.show_trash && hv.folder.is_none(),
            );
            if recents.clicked() {
                hv.show_trash = false;
                hv.folder = None;
            }
            // Dropping on Recents is how a project leaves its folder without a menu.
            if let Some(d) = recents.dnd_release_payload::<ProjectDrag>() {
                hv.actions.push(HomeAction::Move(d.dir.clone(), None));
            }
            let n = hv.trash.len();
            if sidebar_item(
                ui,
                Icon::Trash,
                "Trash",
                (n > 0).then(|| n.to_string()),
                hv.show_trash,
            ) {
                hv.show_trash = true;
            }
            folder_list(ui, hv);
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(hv.root.display().to_string())
                        .size(theme::SECONDARY - 1.0)
                        .color(p.secondary),
                )
                .on_hover_text("Projects folder");
            });
        });
    egui::Panel::top("home-toolbar")
        .exact_size(theme::TOOLBAR_H + 8.0)
        .frame(
            egui::Frame::NONE
                .fill(p.window)
                .inner_margin(egui::Margin::symmetric(20, 0)),
        )
        .show(ui, |ui| {
            if native_chrome {
                super::titlebar_background(ui);
            }
            ui.horizontal_centered(|ui| {
                ui.label(
                    RichText::new(match hv.selected_folder() {
                        _ if hv.show_trash => "Trash",
                        Some(f) => f.name.as_str(),
                        None => "Recents",
                    })
                    .font(theme::semibold(17.0)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if hv.show_trash {
                        if ui
                            .add_enabled(!hv.trash.is_empty(), egui::Button::new("Empty Trash"))
                            .clicked()
                        {
                            hv.actions.push(HomeAction::EmptyTrash);
                        }
                    } else {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("New Project").color(Color32::WHITE),
                                )
                                .fill(theme::accent()),
                            )
                            .clicked()
                        {
                            hv.actions.push(HomeAction::New);
                        }
                        if ui.button("Import…").clicked() {
                            hv.actions.push(HomeAction::Import);
                        }
                    }
                });
            });
            let r = ui.max_rect();
            ui.painter().hline(
                r.x_range(),
                r.bottom() - 0.5,
                egui::Stroke::new(1.0, p.separator),
            );
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(20))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    if hv.show_trash {
                        trash_grid(ui, hv);
                    } else {
                        recents_grid(ui, hv);
                    }
                });
        });
    drag_label(ui);
}

/// The Folders section of the sidebar: a header with a + button, then one row per folder.
fn folder_list(ui: &mut Ui, hv: &mut HomeView) {
    let p = theme::pal(ui);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Folders")
                .font(theme::semibold(theme::SECONDARY))
                .color(p.secondary),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, Icon::Plus, 20.0, false)
                .on_hover_text("New Folder")
                .clicked()
            {
                hv.actions.push(HomeAction::NewFolder(None));
            }
        });
    });
    ui.add_space(2.0);
    let folders = hv.folders.clone();
    egui::ScrollArea::vertical()
        .max_height(ui.available_height() - 56.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for f in &folders {
                folder_row(ui, hv, f);
            }
        });
}

fn folder_row(ui: &mut Ui, hv: &mut HomeView, f: &Folder) {
    if hv.renaming.as_ref().is_some_and(|(d, _)| *d == f.dir) {
        let (_, name) = hv.renaming.as_mut().unwrap();
        let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        if let Some(keep) = super::rename_ended(ui, &r, name) {
            let (dir, name) = hv.renaming.take().unwrap();
            let name = name.trim();
            if keep && !name.is_empty() && name != f.name {
                hv.actions
                    .push(HomeAction::RenameFolder(dir, name.to_string()));
            }
        }
        return;
    }
    let n = hv.folder_size(f);
    let selected = !hv.show_trash && hv.folder.as_ref() == Some(&f.id);
    let row = sidebar_row(
        ui,
        Icon::Folder,
        &f.name,
        (n > 0).then(|| n.to_string()),
        selected,
    );
    if row.double_clicked() {
        hv.renaming = Some((f.dir.clone(), f.name.clone()));
    } else if row.clicked() {
        hv.folder = Some(f.id.clone());
        hv.show_trash = false;
    }
    if let Some(d) = row.dnd_release_payload::<ProjectDrag>() {
        hv.actions
            .push(HomeAction::Move(d.dir.clone(), Some(f.clone())));
    }
    row.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            hv.renaming = Some((f.dir.clone(), f.name.clone()));
            ui.close();
        }
        if ui
            .button("Move to Trash")
            .on_hover_text("Its projects go to the Trash, and restoring one brings the folder back")
            .clicked()
        {
            hv.actions.push(HomeAction::TrashFolder(f.dir.clone()));
            ui.close();
        }
    });
}

/// The name of the card being dragged, following the pointer.
fn drag_label(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(d) = egui::DragAndDrop::payload::<ProjectDrag>(&ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let p = theme::pal(ui);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("home-drag"),
    ));
    let g = painter.layout_no_wrap(
        d.name.clone(),
        egui::FontId::proportional(theme::BODY),
        Color32::WHITE,
    );
    let rect = egui::Rect::from_min_size(pos + egui::vec2(12.0, 8.0), g.size())
        .expand2(egui::vec2(8.0, 4.0));
    painter.rect_filled(rect, 6.0, theme::accent());
    painter.galley(rect.min + egui::vec2(8.0, 4.0), g, p.text);
}

fn sidebar_item(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    badge: Option<String>,
    selected: bool,
) -> bool {
    sidebar_row(ui, icon, label, badge, selected).clicked()
}

fn sidebar_row(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    badge: Option<String>,
    selected: bool,
) -> egui::Response {
    let p = theme::pal(ui);
    let (row, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(row, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, 6.0, p.hover);
    }
    let ir = egui::Rect::from_center_size(
        egui::pos2(row.left() + 16.0, row.center().y),
        egui::vec2(15.0, 15.0),
    );
    theme::paint_icon(ui.painter(), ir, icon, theme::accent());
    let mut right = row.right() - 8.0;
    if let Some(b) = badge {
        let r = ui.painter().text(
            egui::pos2(right, row.center().y),
            egui::Align2::RIGHT_CENTER,
            b,
            egui::FontId::proportional(theme::SECONDARY),
            p.secondary,
        );
        right = r.left() - 6.0;
    }
    let text = egui::Rect::from_min_max(
        egui::pos2(row.left() + 32.0, row.top()),
        egui::pos2(right, row.bottom()),
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_string(),
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(text.width());
    let g = ui.painter().layout_job(job);
    let elided = g.elided;
    ui.painter().galley(
        egui::pos2(text.left(), row.center().y - g.size().y / 2.0),
        g,
        p.text,
    );
    if resp.dnd_hover_payload::<ProjectDrag>().is_some() {
        ui.painter().rect_stroke(
            row,
            6.0,
            egui::Stroke::new(2.0, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    if elided {
        resp.on_hover_text(label)
    } else {
        resp
    }
}

fn columns(ui: &Ui) -> usize {
    ((ui.available_width() + 20.0) / (CARD_W + 20.0))
        .floor()
        .max(1.0) as usize
}

fn recents_grid(ui: &mut Ui, hv: &mut HomeView) {
    let p = theme::pal(ui);
    let entries = hv.visible();
    if entries.is_empty() {
        let in_folder = hv.folder.is_some();
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(if in_folder {
                    "This folder is empty"
                } else {
                    "No projects yet"
                })
                .font(theme::semibold(17.0)),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(if in_folder {
                    "Create a project here, or drag one onto the folder from Recents."
                } else {
                    "Projects are folders in your Documents, saved as you work."
                })
                .color(p.secondary),
            );
            ui.add_space(12.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("New Project").color(Color32::WHITE))
                        .fill(theme::accent()),
                )
                .clicked()
            {
                hv.actions.push(HomeAction::New);
            }
        });
        return;
    }
    let cols = columns(ui);
    for row in entries.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            for e in row {
                project_card(ui, hv, e);
            }
        });
        ui.add_space(16.0);
    }
}

fn card_frame(
    ui: &mut Ui,
    hv: &mut HomeView,
    thumb: Option<&Path>,
    id: egui::Id,
) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(CARD_W, THUMB_H), egui::Sense::click_and_drag());
    let hovered = resp.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, p.thumbnail);
    if let Some(t) = thumb.and_then(|t| hv.thumb(ui.ctx(), t)) {
        // Fit the thumbnail inside the card, centered.
        let [tw, th] = t.size();
        let s = ((rect.width() - 16.0) / tw as f32)
            .min((rect.height() - 16.0) / th as f32)
            .min(1.0);
        let r =
            egui::Rect::from_center_size(rect.center(), egui::vec2(tw as f32 * s, th as f32 * s));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        theme::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(rect.center(), egui::vec2(28.0, 28.0)),
            Icon::Page,
            p.tertiary,
        );
    }
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(
            if hovered { 2.0 } else { 1.0 },
            if hovered {
                theme::accent()
            } else {
                p.separator
            },
        ),
        egui::StrokeKind::Inside,
    );
    let _ = id;
    resp
}

fn project_card(ui: &mut Ui, hv: &mut HomeView, e: &Entry) {
    let p = theme::pal(ui);
    ui.vertical(|ui| {
        ui.set_width(CARD_W);
        ui.spacing_mut().item_spacing.y = 2.0;
        let id = egui::Id::new(("card", &e.dir));
        let resp = card_frame(ui, hv, e.thumbnail.as_deref(), id);
        if resp.double_clicked() {
            hv.actions.push(HomeAction::Open(e.dir.clone()));
        }
        resp.dnd_set_drag_payload(ProjectDrag {
            dir: e.dir.clone(),
            name: e.manifest.name.clone(),
        });
        resp.context_menu(|ui| card_menu(ui, hv, e));
        ui.add_space(6.0);
        let renaming = hv.renaming.as_ref().is_some_and(|(d, _)| *d == e.dir);
        if renaming {
            let (_, name) = hv.renaming.as_mut().unwrap();
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(CARD_W));
            if let Some(keep) = super::rename_ended(ui, &r, name) {
                let (dir, name) = hv.renaming.take().unwrap();
                if keep && name.trim() != e.manifest.name {
                    hv.actions.push(HomeAction::Rename(dir, name));
                }
            }
        } else {
            ui.horizontal(|ui| {
                let name = ui.add(
                    egui::Label::new(
                        RichText::new(&e.manifest.name).font(theme::semibold(theme::BODY)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                );
                if name.double_clicked() {
                    hv.renaming = Some((e.dir.clone(), e.manifest.name.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more =
                        theme::icon_button(ui, Icon::More, 20.0, false).on_hover_text("More");
                    egui::Popup::menu(&more).show(|ui| card_menu(ui, hv, e));
                });
            });
        }
        // Recents mixes every folder, so name the folder there. It replaces the page count
        // to keep the caption on one line.
        let edited = relative_time(e.manifest.modified, hv.now_ms);
        let pages = e.manifest.pages.len();
        let caption = match e.folder.as_ref().filter(|_| hv.folder.is_none()) {
            Some(f) => format!("{} · Edited {edited}", f.name),
            None => format!(
                "Edited {edited} · {pages} page{}",
                if pages == 1 { "" } else { "s" }
            ),
        };
        ui.add(
            egui::Label::new(
                RichText::new(caption)
                    .size(theme::SECONDARY)
                    .color(p.secondary),
            )
            .truncate(),
        );
    });
}

fn move_menu(ui: &mut Ui, hv: &mut HomeView, e: &Entry) {
    if ui.button("New Folder…").clicked() {
        hv.actions.push(HomeAction::NewFolder(Some(e.dir.clone())));
        ui.close();
    }
    ui.separator();
    let current = e.folder.as_ref().map(|f| &f.id);
    for f in hv.folders.clone() {
        if ui
            .add_enabled(current != Some(&f.id), egui::Button::new(&f.name))
            .clicked()
        {
            hv.actions.push(HomeAction::Move(e.dir.clone(), Some(f)));
            ui.close();
        }
    }
    if !hv.folders.is_empty() {
        ui.separator();
    }
    if ui
        .add_enabled(current.is_some(), egui::Button::new("No Folder"))
        .clicked()
    {
        hv.actions.push(HomeAction::Move(e.dir.clone(), None));
        ui.close();
    }
}

fn card_menu(ui: &mut Ui, hv: &mut HomeView, e: &Entry) {
    if ui.button("Open").clicked() {
        hv.actions.push(HomeAction::Open(e.dir.clone()));
        ui.close();
    }
    if ui.button("Rename").clicked() {
        hv.renaming = Some((e.dir.clone(), e.manifest.name.clone()));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        hv.actions.push(HomeAction::Duplicate(e.dir.clone()));
        ui.close();
    }
    ui.menu_button("Move to", |ui| move_menu(ui, hv, e));
    ui.separator();
    if ui.button("Move to Trash").clicked() {
        hv.actions.push(HomeAction::Trash(e.dir.clone()));
        ui.close();
    }
}

fn trash_grid(ui: &mut Ui, hv: &mut HomeView) {
    let p = theme::pal(ui);
    if hv.trash.is_empty() {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Trash is empty").font(theme::semibold(17.0)));
            ui.label(
                RichText::new("Deleted projects stay here until you empty the trash.")
                    .color(p.secondary),
            );
        });
        return;
    }
    let cols = columns(ui);
    let entries = hv.trash.clone();
    for row in entries.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            for e in row {
                ui.vertical(|ui| {
                    ui.set_width(CARD_W);
                    let thumb = e.dir.join(crate::project::THUMBNAIL);
                    let thumb = thumb.is_file().then_some(thumb);
                    let resp = card_frame(ui, hv, thumb.as_deref(), egui::Id::new(&e.dir));
                    resp.context_menu(|ui| {
                        if ui.button("Restore").clicked() {
                            hv.actions.push(HomeAction::Restore(e.dir.clone()));
                            ui.close();
                        }
                        if ui.button("Delete Permanently").clicked() {
                            hv.actions.push(HomeAction::DeleteForever(e.dir.clone()));
                            ui.close();
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&e.manifest.name).font(theme::semibold(theme::BODY)),
                            )
                            .truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::icon_button(ui, Icon::Restore, 20.0, false)
                                .on_hover_text("Restore")
                                .clicked()
                            {
                                hv.actions.push(HomeAction::Restore(e.dir.clone()));
                            }
                        });
                    });
                    let deleted = relative_time(e.deleted, hv.now_ms);
                    ui.add(
                        egui::Label::new(
                            RichText::new(match &e.folder {
                                Some(f) => format!("{} · Deleted {deleted}", f.name),
                                None => format!("Deleted {deleted}"),
                            })
                            .size(theme::SECONDARY)
                            .color(p.secondary),
                        )
                        .truncate(),
                    );
                });
            }
        });
        ui.add_space(16.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        let now = 10_000_000_000;
        assert_eq!(relative_time(now - 5_000, now), "just now");
        assert_eq!(relative_time(now - 60_000, now), "1 minute ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 hours ago");
        assert_eq!(relative_time(now - 2 * 86_400_000, now), "2 days ago");
        assert_eq!(relative_time(now + 1, now), "just now");
    }
}
