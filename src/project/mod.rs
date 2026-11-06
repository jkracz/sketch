//! Local-first projects.
//!
//! Every project is a directory under the store root (`~/Documents/sketch-rs/` by default),
//! either directly or inside one level of folders:
//!
//! ```text
//! <root>/<Project name>/
//!     project.json        manifest: id, name, timestamps, ordered page list
//!     pages/<page-id>.json one infinite canvas per page
//!     assets/<hash>.<ext> images used by image fills
//!     thumbnail.png       preview for the home screen (written by the app)
//! <root>/<Folder name>/folder.json             marks a folder: id and display name
//! <root>/<Folder name>/<Project name>/          a project, laid out as above
//! <root>/.trash/<Project name>~<timestamp>/     deleted projects, restorable
//! ```
//!
//! All writes go through [`crate::io::atomic`], so a crash never leaves a torn file.

pub mod format;

use crate::io::atomic;
pub use format::{FolderManifest, FormatError, Manifest, PageData, PageRef, new_id, now_ms};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "project.json";
pub const FOLDER_MANIFEST: &str = "folder.json";
pub const PAGES_DIR: &str = "pages";
pub const THUMBNAIL: &str = "thumbnail.png";
pub const ASSETS_DIR: &str = "assets";
pub const TRASH_DIR: &str = ".trash";
const TRASH_INFO: &str = "trashinfo.json";

/// The directory holding all projects.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

/// A folder of projects. Folders sit directly under the store root and don't nest.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub dir: PathBuf,
    pub id: String,
    pub name: String,
}

/// A project as listed on the home screen.
#[derive(Debug, Clone)]
pub struct Entry {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub thumbnail: Option<PathBuf>,
    /// The folder it's in, or None at the top level.
    pub folder: Option<Folder>,
}

#[derive(Debug, Clone)]
pub struct TrashEntry {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// Unix ms when it was deleted.
    pub deleted: u64,
    /// Directory name it had before deletion.
    pub original: String,
    /// The folder it came from. Restoring recreates the folder if it has gone.
    pub folder: Option<TrashedFolder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrashedFolder {
    pub id: String,
    pub name: String,
}

/// An open project on disk.
#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// Move the projects folder from `old` to `new` unless `new` already exists. If the move fails,
/// keep using `old` so the projects don't disappear from the home screen.
fn adopt_old_root(old: PathBuf, new: PathBuf) -> PathBuf {
    if new.exists() || !old.is_dir() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(e) => {
            log::warn!("could not move {} to {}: {e}", old.display(), new.display());
            old
        }
    }
}

/// File-system safe folder name for a project name.
pub fn folder_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let s = s.trim_start_matches('.').trim().to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

impl Store {
    /// `$SKETCH_RS_HOME` (or `$PAPER_RS_HOME`), else `~/Documents/sketch-rs`. A
    /// `~/Documents/paper-rs` folder from before the rename is moved there the first time.
    pub fn default_root() -> PathBuf {
        if let Some(p) = crate::env_var("HOME") {
            return PathBuf::from(p);
        }
        let docs = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        adopt_old_root(docs.join("paper-rs"), docs.join("sketch-rs"))
    }

    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Store> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Store { root })
    }

    fn trash_root(&self) -> PathBuf {
        self.root.join(TRASH_DIR)
    }

    /// Every project, most recently modified first. Unreadable directories are skipped.
    pub fn list(&self) -> Vec<Entry> {
        let mut out = projects_in(&self.root, None);
        for f in self.folders() {
            out.extend(projects_in(&f.dir, Some(&f)));
        }
        out.sort_by(|a, b| {
            b.manifest
                .modified
                .cmp(&a.manifest.modified)
                .then_with(|| a.manifest.name.cmp(&b.manifest.name))
        });
        out
    }

    /// Folders by name. A directory counts only if it has a folder marker.
    pub fn folders(&self) -> Vec<Folder> {
        let mut out: Vec<Folder> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| read_folder(&e.path()))
            .collect();
        out.sort_by_key(|f| f.name.to_lowercase());
        out
    }

    /// The folder a project is in, or None at the top level.
    pub fn folder_of(&self, project_dir: &Path) -> Option<Folder> {
        let parent = project_dir.parent()?;
        if !same_path(parent.parent()?, &self.root) {
            return None;
        }
        read_folder(parent)
    }

    fn parent_dir<'a>(&'a self, folder: Option<&'a Folder>) -> &'a Path {
        folder.map_or(&self.root, |f| &f.dir)
    }

    /// A folder path for `name` that doesn't exist yet ("Name", "Name 2", ...).
    fn unique_dir(&self, parent: &Path, name: &str) -> PathBuf {
        let base = folder_name(name);
        let mut p = parent.join(&base);
        let mut i = 2;
        while p.exists() {
            p = parent.join(format!("{base} {i}"));
            i += 1;
        }
        p
    }

    /// A display name not used by a project in `parent` ("Untitled", "Untitled 2", ...).
    fn unique_name(&self, parent: &Path, name: &str) -> String {
        let taken: Vec<String> = projects_in(parent, None)
            .into_iter()
            .map(|e| e.manifest.name)
            .collect();
        let mut n = name.to_string();
        let mut i = 2;
        while taken.contains(&n) || parent.join(folder_name(&n)).exists() {
            n = format!("{name} {i}");
            i += 1;
        }
        n
    }

    /// Create an empty project with one blank page.
    pub fn create(&self, name: &str) -> Result<(Project, PageData), FormatError> {
        self.create_in(name, None)
    }

    /// Create an empty project with one blank page, inside `folder` if given.
    pub fn create_in(
        &self,
        name: &str,
        folder: Option<&Folder>,
    ) -> Result<(Project, PageData), FormatError> {
        let parent = self.parent_dir(folder);
        let name = self.unique_name(parent, name);
        let dir = self.unique_dir(parent, &name);
        std::fs::create_dir_all(dir.join(PAGES_DIR))?;
        let manifest = Manifest::new(&name);
        let page = PageData {
            id: manifest.pages[0].id.clone(),
            name: manifest.pages[0].name.clone(),
            doc: crate::model::Document {
                name: manifest.pages[0].name.clone(),
                ..Default::default()
            },
            camera: None,
        };
        let p = Project { dir, manifest };
        p.save_page(&page)?;
        p.write_manifest()?;
        Ok((p, page))
    }

    /// Create a project from existing pages (imports, legacy files).
    pub fn create_with_pages(
        &self,
        name: &str,
        pages: Vec<PageData>,
    ) -> Result<Project, FormatError> {
        self.create_with_pages_in(name, pages, None)
    }

    /// Create a project from existing pages, inside `folder` if given.
    pub fn create_with_pages_in(
        &self,
        name: &str,
        pages: Vec<PageData>,
        folder: Option<&Folder>,
    ) -> Result<Project, FormatError> {
        let parent = self.parent_dir(folder);
        let name = self.unique_name(parent, name);
        let dir = self.unique_dir(parent, &name);
        std::fs::create_dir_all(dir.join(PAGES_DIR))?;
        let mut manifest = Manifest::new(&name);
        manifest.pages = pages
            .iter()
            .map(|p| PageRef {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect();
        manifest.active_page = manifest.pages.first().map(|p| p.id.clone());
        let p = Project { dir, manifest };
        for page in &pages {
            crate::assets::save_to_dir(&p.dir.join(ASSETS_DIR), crate::assets::used_by(&page.doc))?;
            p.save_page(page)?;
        }
        p.write_manifest()?;
        Ok(p)
    }

    /// Rename a project: updates the manifest and renames its directory to match.
    pub fn rename(&self, dir: &Path, new_name: &str) -> Result<PathBuf, FormatError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(FormatError::Invalid("empty name".into()));
        }
        let mut p = Project::open(dir)?;
        p.manifest.name = new_name.to_string();
        p.manifest.modified = now_ms();
        p.write_manifest()?;
        let parent = dir.parent().unwrap_or(&self.root);
        let wanted = parent.join(folder_name(new_name));
        if wanted == dir {
            return Ok(dir.to_path_buf());
        }
        let target = if wanted.exists() {
            self.unique_dir(parent, new_name)
        } else {
            wanted
        };
        std::fs::rename(dir, &target)?;
        Ok(target)
    }

    /// Copy a project (all pages) under a fresh id and "Name copy".
    pub fn duplicate(&self, dir: &Path) -> Result<PathBuf, FormatError> {
        let src = Project::open(dir)?;
        let parent = dir.parent().unwrap_or(&self.root);
        let name = self.unique_name(parent, &format!("{} copy", src.manifest.name));
        let target = self.unique_dir(parent, &name);
        copy_dir(dir, &target)?;
        let mut p = Project::open(&target)?;
        p.manifest.id = new_id();
        p.manifest.name = name;
        p.manifest.created = now_ms();
        p.manifest.modified = p.manifest.created;
        p.write_manifest()?;
        Ok(target)
    }

    /// Move a project into `folder`, or to the top level when None. A project already there
    /// stays put. If the name is taken at the destination the project gets the next free name,
    /// so check the manifest for it. The edit time doesn't change, so a move doesn't reorder
    /// Recents.
    pub fn move_project(
        &self,
        dir: &Path,
        folder: Option<&Folder>,
    ) -> Result<PathBuf, FormatError> {
        let parent = self.parent_dir(folder);
        if dir.parent().is_some_and(|p| same_path(p, parent)) {
            return Ok(dir.to_path_buf());
        }
        let manifest = read_manifest(dir)?;
        let name = self.unique_name(parent, &manifest.name);
        let target = self.unique_dir(parent, &name);
        std::fs::rename(dir, &target)?;
        if name != manifest.name {
            let mut moved = Project {
                dir: target.clone(),
                manifest,
            };
            moved.manifest.name = name;
            moved.write_manifest()?;
        }
        Ok(target)
    }

    /// A folder name no other folder uses, ignoring case ("New Folder", "New Folder 2", ...).
    fn unique_folder_name(&self, name: &str, except: Option<&Path>) -> String {
        let taken: Vec<String> = self
            .folders()
            .into_iter()
            .filter(|f| Some(f.dir.as_path()) != except)
            .map(|f| f.name.to_lowercase())
            .collect();
        let mut n = name.to_string();
        let mut i = 2;
        while taken.contains(&n.to_lowercase()) {
            n = format!("{name} {i}");
            i += 1;
        }
        n
    }

    fn make_folder(&self, id: &str, name: &str) -> Result<Folder, FormatError> {
        let dir = self.unique_dir(&self.root, name);
        std::fs::create_dir_all(&dir)?;
        let manifest = FolderManifest::new(id, name);
        atomic::write(&dir.join(FOLDER_MANIFEST), manifest.to_json().as_bytes())?;
        Ok(Folder {
            dir,
            id: id.into(),
            name: name.into(),
        })
    }

    /// Create a folder. If the name is taken it gets the next free name.
    pub fn create_folder(&self, name: &str) -> Result<Folder, FormatError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(FormatError::Invalid("empty name".into()));
        }
        self.make_folder(&new_id(), &self.unique_folder_name(name, None))
    }

    /// Rename a folder and its directory. Its projects move with it, so any `dir` under the
    /// old path goes stale.
    pub fn rename_folder(&self, dir: &Path, new_name: &str) -> Result<Folder, FormatError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(FormatError::Invalid("empty name".into()));
        }
        let mut manifest = read_folder_manifest(dir)?;
        manifest.name = self.unique_folder_name(new_name, Some(dir));
        atomic::write(&dir.join(FOLDER_MANIFEST), manifest.to_json().as_bytes())?;
        let wanted = self.root.join(folder_name(&manifest.name));
        let target = if wanted == dir {
            wanted
        } else {
            let target = if wanted.exists() {
                self.unique_dir(&self.root, &manifest.name)
            } else {
                wanted
            };
            std::fs::rename(dir, &target)?;
            target
        };
        Ok(Folder {
            dir: target,
            id: manifest.id,
            name: manifest.name,
        })
    }

    /// Move a folder's projects to the trash, then remove the folder. Restoring any of them
    /// brings the folder back. Files Sketch didn't put there stay in place, along with the
    /// directory holding them.
    pub fn trash_folder(&self, dir: &Path) -> Result<(), FormatError> {
        read_folder_manifest(dir)?;
        for e in projects_in(dir, None) {
            self.trash(&e.dir)?;
        }
        std::fs::remove_file(dir.join(FOLDER_MANIFEST))?;
        let _ = std::fs::remove_file(dir.join(".DS_Store"));
        if let Err(e) = std::fs::remove_dir(dir) {
            log::warn!("left {} in place: {e}", dir.display());
        }
        Ok(())
    }

    /// Move a project to the trash. Returns its path in the trash.
    pub fn trash(&self, dir: &Path) -> Result<PathBuf, FormatError> {
        let original = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let folder = self.folder_of(dir).map(|f| TrashedFolder {
            id: f.id,
            name: f.name,
        });
        std::fs::create_dir_all(self.trash_root())?;
        let deleted = now_ms();
        let target = self
            .trash_root()
            .join(format!("{}~{deleted}", folder_name(&original)));
        std::fs::rename(dir, &target)?;
        let info =
            serde_json::json!({ "deleted": deleted, "original": original, "folder": folder });
        atomic::write(&target.join(TRASH_INFO), info.to_string().as_bytes())?;
        Ok(target)
    }

    pub fn list_trash(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = std::fs::read_dir(self.trash_root())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let dir = e.path();
                let manifest = read_manifest(&dir).ok()?;
                let info: serde_json::Value = std::fs::read_to_string(dir.join(TRASH_INFO))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Some(TrashEntry {
                    deleted: info["deleted"].as_u64().unwrap_or(0),
                    original: info["original"]
                        .as_str()
                        .unwrap_or(&manifest.name)
                        .to_string(),
                    folder: serde_json::from_value(info["folder"].clone()).unwrap_or_default(),
                    dir,
                    manifest,
                })
            })
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.deleted));
        out
    }

    /// Put a trashed project back where it was, recreating its folder if that has gone (under a
    /// unique directory name if the old one is taken).
    pub fn restore(&self, trash_dir: &Path) -> Result<PathBuf, FormatError> {
        let entry = self
            .list_trash()
            .into_iter()
            .find(|e| e.dir == trash_dir)
            .ok_or_else(|| FormatError::Invalid("not in trash".into()))?;
        let folder = match &entry.folder {
            Some(t) => Some(match self.folders().into_iter().find(|f| f.id == t.id) {
                Some(f) => f,
                None => self.make_folder(&t.id, &self.unique_folder_name(&t.name, None))?,
            }),
            None => None,
        };
        let target = self.unique_dir(self.parent_dir(folder.as_ref()), &entry.original);
        std::fs::rename(trash_dir, &target)?;
        let _ = std::fs::remove_file(target.join(TRASH_INFO));
        Ok(target)
    }

    pub fn delete_forever(&self, trash_dir: &Path) -> std::io::Result<()> {
        if trash_dir.parent() != Some(self.trash_root().as_path()) {
            return Err(std::io::Error::other(
                "refusing to delete outside the trash",
            ));
        }
        std::fs::remove_dir_all(trash_dir)
    }

    pub fn empty_trash(&self) -> std::io::Result<()> {
        for e in self.list_trash() {
            self.delete_forever(&e.dir)?;
        }
        Ok(())
    }
}

fn read_manifest(dir: &Path) -> Result<Manifest, FormatError> {
    Manifest::from_json(&std::fs::read_to_string(dir.join(MANIFEST))?)
}

fn read_folder_manifest(dir: &Path) -> Result<FolderManifest, FormatError> {
    FolderManifest::from_json(&std::fs::read_to_string(dir.join(FOLDER_MANIFEST))?)
}

fn read_folder(dir: &Path) -> Option<Folder> {
    let m = read_folder_manifest(dir).ok()?;
    Some(Folder {
        dir: dir.to_path_buf(),
        id: m.id,
        name: m.name,
    })
}

/// The projects directly inside `parent`.
fn projects_in(parent: &Path, folder: Option<&Folder>) -> Vec<Entry> {
    std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let dir = e.path();
            let manifest = read_manifest(&dir).ok()?;
            let thumb = dir.join(THUMBNAIL);
            Some(Entry {
                thumbnail: thumb.is_file().then_some(thumb),
                dir,
                manifest,
                folder: folder.cloned(),
            })
        })
        .collect()
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

impl Project {
    pub fn open(dir: &Path) -> Result<Project, FormatError> {
        let manifest = read_manifest(dir)?;
        atomic::clean_stale_temps(dir);
        atomic::clean_stale_temps(&dir.join(PAGES_DIR));
        Ok(Project {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    pub fn page_path(&self, id: &str) -> PathBuf {
        self.dir.join(PAGES_DIR).join(format!("{id}.json"))
    }

    pub fn load_page(&self, id: &str) -> Result<PageData, FormatError> {
        let mut p = format::page_from_json(&std::fs::read_to_string(self.page_path(id))?)?;
        if let Some(r) = self.manifest.pages.iter().find(|r| r.id == id) {
            // The manifest owns page names (renames touch only the manifest).
            p.name = r.name.clone();
            p.doc.name = r.name.clone();
        }
        Ok(p)
    }

    /// Load every page listed in the manifest. A page whose file is missing comes back empty
    /// (e.g. after a crash between creating it and its first save).
    pub fn load_pages(&self) -> Result<Vec<PageData>, FormatError> {
        self.manifest
            .pages
            .iter()
            .map(|r| match self.load_page(&r.id) {
                Err(FormatError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(PageData {
                        id: r.id.clone(),
                        name: r.name.clone(),
                        doc: crate::model::Document {
                            name: r.name.clone(),
                            ..Default::default()
                        },
                        camera: None,
                    })
                }
                other => other,
            })
            .collect()
    }

    pub fn save_page(&self, page: &PageData) -> Result<(), FormatError> {
        std::fs::create_dir_all(self.dir.join(PAGES_DIR))?;
        atomic::write(
            &self.page_path(&page.id),
            format::page_to_json(page).as_bytes(),
        )?;
        Ok(())
    }

    pub fn delete_page_file(&self, id: &str) {
        let _ = std::fs::remove_file(self.page_path(id));
    }

    pub fn write_manifest(&self) -> Result<(), FormatError> {
        atomic::write(&self.dir.join(MANIFEST), self.manifest.to_json().as_bytes())?;
        Ok(())
    }

    pub fn thumbnail_path(&self) -> PathBuf {
        self.dir.join(THUMBNAIL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_store() -> Store {
        let root = std::env::temp_dir().join(format!("sketch-rs-store-{:016x}", fastrand::u64(..)));
        Store::open(root).unwrap()
    }

    #[test]
    fn create_list_rename_duplicate_trash_restore() {
        let s = temp_store();
        let (a, page) = s.create("Alpha").unwrap();
        assert!(a.dir.join(MANIFEST).is_file());
        assert!(a.page_path(&page.id).is_file());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (b, _) = s.create("Alpha").unwrap();
        assert_eq!(b.manifest.name, "Alpha 2");
        let names: Vec<String> = s.list().into_iter().map(|e| e.manifest.name).collect();
        assert_eq!(names, ["Alpha 2", "Alpha"], "most recent first");

        let renamed = s.rename(&a.dir, "Beta: plan").unwrap();
        assert_eq!(renamed.file_name().unwrap(), "Beta- plan");
        assert_eq!(Project::open(&renamed).unwrap().manifest.name, "Beta: plan");

        let dup = s.duplicate(&renamed).unwrap();
        let d = Project::open(&dup).unwrap();
        assert_eq!(d.manifest.name, "Beta: plan copy");
        assert_ne!(d.manifest.id, Project::open(&renamed).unwrap().manifest.id);
        assert_eq!(d.load_pages().unwrap().len(), 1);

        let t = s.trash(&renamed).unwrap();
        assert!(!renamed.exists());
        assert_eq!(s.list().len(), 2);
        let trash = s.list_trash();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].manifest.name, "Beta: plan");
        let back = s.restore(&t).unwrap();
        assert_eq!(back, renamed);
        assert_eq!(s.list().len(), 3);

        s.trash(&dup).unwrap();
        s.empty_trash().unwrap();
        assert!(s.list_trash().is_empty());
        assert!(s.delete_forever(&s.root.join("Alpha")).is_err());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn folders_hold_projects_and_only_marked_directories_count() {
        let s = temp_store();
        let acme = s.create_folder("Client: Acme").unwrap();
        assert_eq!(acme.dir.file_name().unwrap(), "Client- Acme");
        let (top, _) = s.create("Plan").unwrap();
        let (inside, _) = s.create_in("Plan", Some(&acme)).unwrap();
        assert_eq!(
            inside.manifest.name, "Plan",
            "names only clash within a folder"
        );
        assert_eq!(inside.dir.parent(), Some(acme.dir.as_path()));
        // A stray directory and a damaged project aren't folders.
        std::fs::create_dir_all(s.root.join("cache/fonts")).unwrap();
        std::fs::create_dir_all(s.root.join("Broken/pages")).unwrap();
        assert_eq!(s.folders(), std::slice::from_ref(&acme));

        let list = s.list();
        assert_eq!(list.len(), 2);
        let found = list.iter().find(|e| e.dir == inside.dir).unwrap();
        assert_eq!(found.folder.as_ref(), Some(&acme));
        assert_eq!(s.folder_of(&inside.dir).as_ref(), Some(&acme));
        assert_eq!(s.folder_of(&top.dir), None);

        // Duplicates and renames stay in the folder.
        let dup = s.duplicate(&inside.dir).unwrap();
        assert_eq!(dup.parent(), Some(acme.dir.as_path()));
        let renamed = s.rename(&dup, "Plan v2").unwrap();
        assert_eq!(renamed.parent(), Some(acme.dir.as_path()));

        assert_eq!(
            s.create_folder("client: acme").unwrap().name,
            "client: acme 2"
        );
        assert!(s.create_folder("  ").is_err());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn moving_a_project_keeps_its_edit_time_and_resolves_name_clashes() {
        let s = temp_store();
        let acme = s.create_folder("Acme").unwrap();
        let (a, _) = s.create("Plan").unwrap();
        let (b, _) = s.create_in("Plan", Some(&acme)).unwrap();

        let moved = s.move_project(&a.dir, Some(&acme)).unwrap();
        assert!(!a.dir.exists());
        let p = Project::open(&moved).unwrap();
        assert_eq!(p.manifest.name, "Plan 2");
        assert_eq!(p.manifest.modified, a.manifest.modified);
        assert_eq!(Project::open(&b.dir).unwrap().manifest.name, "Plan");

        assert_eq!(s.move_project(&moved, Some(&acme)).unwrap(), moved);
        let out = s.move_project(&moved, None).unwrap();
        assert_eq!(out.parent(), Some(s.root.as_path()));
        assert_eq!(s.folder_of(&out), None);
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn renaming_a_folder_keeps_its_id_and_projects() {
        let s = temp_store();
        let acme = s.create_folder("Acme").unwrap();
        s.create_folder("Beta").unwrap();
        s.create_in("Plan", Some(&acme)).unwrap();

        let renamed = s.rename_folder(&acme.dir, "Beta").unwrap();
        assert_eq!(renamed.id, acme.id);
        assert_eq!(renamed.name, "Beta 2");
        assert!(!acme.dir.exists());
        let list = s.list();
        assert_eq!(list[0].folder.as_ref(), Some(&renamed));
        assert_eq!(s.rename_folder(&renamed.dir, "Beta 2").unwrap(), renamed);
        assert!(s.rename_folder(&renamed.dir, " ").is_err());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn trashed_projects_restore_into_their_folder_even_after_it_is_gone() {
        let s = temp_store();
        let acme = s.create_folder("Acme").unwrap();
        let (one, _) = s.create_in("One", Some(&acme)).unwrap();
        s.create_in("Two", Some(&acme)).unwrap();

        let t = s.trash(&one.dir).unwrap();
        assert_eq!(s.list_trash()[0].folder.as_ref().unwrap().name, "Acme");
        assert_eq!(s.restore(&t).unwrap(), one.dir);

        // Deleting the folder trashes its projects, but not files Sketch doesn't own.
        std::fs::write(acme.dir.join("notes.txt"), "keep me").unwrap();
        s.trash_folder(&acme.dir).unwrap();
        assert!(s.folders().is_empty());
        assert!(s.list().is_empty());
        assert_eq!(s.list_trash().len(), 2);
        assert_eq!(
            std::fs::read_to_string(acme.dir.join("notes.txt")).unwrap(),
            "keep me"
        );
        std::fs::remove_dir_all(&acme.dir).unwrap();

        let trash = s.list_trash();
        let back = s.restore(&trash[0].dir).unwrap();
        let other = s.restore(&trash[1].dir).unwrap();
        let folders = s.folders();
        assert_eq!(folders.len(), 1, "both land in one recreated folder");
        assert_eq!(folders[0].id, acme.id);
        assert_eq!(s.folder_of(&back), Some(folders[0].clone()));
        assert_eq!(s.folder_of(&other), Some(folders[0].clone()));
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn pages_roundtrip_and_missing_page_is_empty() {
        let s = temp_store();
        let (mut p, mut page) = s.create("P").unwrap();
        page.doc.name = "Page 1".into();
        let id = page.doc.alloc_id();
        let n = crate::model::Node::new(
            id,
            crate::model::NodeKind::Rect,
            crate::geom::Rect::new(1.0, 2.0, 3.0, 4.0),
        );
        page.doc.nodes.insert(id, n);
        page.doc.root.push(id);
        p.save_page(&page).unwrap();
        p.manifest.pages.push(PageRef {
            id: "ghost".into(),
            name: "Ghost".into(),
        });
        p.write_manifest().unwrap();
        let p = Project::open(&p.dir).unwrap();
        let pages = p.load_pages().unwrap();
        assert_eq!(pages[0], page);
        assert!(pages[1].doc.nodes.is_empty());
        assert_eq!(pages[1].name, "Ghost");
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("a/b\\c"), "a-b-c");
        assert_eq!(folder_name("  ..hidden "), "hidden");
        assert_eq!(folder_name(""), "Untitled");
    }

    #[test]
    fn projects_folder_from_before_the_rename_moves_once() {
        let base = std::env::temp_dir().join(format!("sketch-rs-root-{:016x}", fastrand::u64(..)));
        let (old, new) = (base.join("paper-rs"), base.join("sketch-rs"));
        std::fs::create_dir_all(old.join("Plan")).unwrap();
        assert_eq!(adopt_old_root(old.clone(), new.clone()), new);
        assert!(new.join("Plan").is_dir());
        assert!(!old.exists());

        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(adopt_old_root(old.clone(), new.clone()), new);
        assert!(
            old.exists(),
            "an existing new folder wins and the old one is left alone"
        );
        std::fs::remove_dir_all(base).ok();
    }
}
