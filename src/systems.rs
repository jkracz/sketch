//! Published design systems, shared by every project in the store
//! (`<projects folder>/systems/<id>.json`).
//!
//! A project publishes its local tokens as one design system. The registry keeps only the
//! latest version. Projects that use a system hold a read-only copy of the version they last
//! saw (see `TokenSet::attach`) and move to a newer one when they open.

use crate::editor::Editor;
use crate::model::{
    Mode, SystemLink, TextStyleDef, Token, TokenSet,
    text_styles::style_slug,
    tokens::{css_slug, linked_id},
};
use crate::project::{Store, new_id, now_ms};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "sketch-rs-system";
pub const VERSION: u32 = 1;
pub const DIR: &str = "systems";
pub const MAX_PREFIX: usize = 16;
pub const MAX_NAME: usize = 64;
pub const MAX_DESCRIPTION: usize = 280;
pub const MAX_NOTES: usize = 10_000;

/// A design system as published: its settings plus a snapshot of the source's tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Published {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    pub prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Counts publishes from 1.
    pub release: u32,
    /// The id of the project it's published from.
    pub source: String,
    /// Unix milliseconds.
    pub published: u64,
    /// Local tokens only, with aliases to other systems flattened (`TokenSet::publishable`).
    pub tokens: Vec<Token>,
    /// Local text styles, flattened the same way (`TokenSet::publishable_styles`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_styles: Vec<TextStyleDef>,
    /// The source's modes. Tokens key their other values by these ids. Empty for a system
    /// published before modes existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<Mode>,
}

impl Published {
    pub fn link(&self) -> SystemLink {
        SystemLink {
            id: self.id.clone(),
            name: self.name.clone(),
            prefix: self.prefix.clone(),
            release: self.release,
            description: self.description.clone(),
            notes: self.notes.clone(),
            modes: (self.modes.iter())
                .map(|m| Mode {
                    id: linked_id(&self.id, &m.id),
                    name: m.name.clone(),
                })
                .collect(),
        }
    }

    /// The CSS name a token gets in projects that use this system.
    pub fn css_name(&self, t: &Token) -> String {
        format!("--{}-{}", self.prefix, css_slug(&t.name))
    }

    /// The tokens as a set that resolves aliases and names them with the prefix, for
    /// listing a system without using it.
    pub fn token_set(&self) -> TokenSet {
        let mut s = TokenSet::default();
        // A fresh set can't clash or share a prefix, so this can't fail.
        let _ = s.attach(self.link(), &self.tokens, &self.text_styles);
        s
    }
}

/// The folder of published systems. Every call reads the disk, so there is nothing to keep
/// in step when another part of the app publishes.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
}

impl Registry {
    pub fn in_store(store: &Store) -> Registry {
        Registry {
            dir: store.root.join(DIR),
        }
    }

    /// Ids come from `new_id`; anything else could point outside the folder.
    fn path(&self, id: &str) -> Option<PathBuf> {
        let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric());
        ok.then(|| self.dir.join(format!("{id}.json")))
    }

    pub fn get(&self, id: &str) -> Option<Published> {
        read(&self.path(id)?)
    }

    /// Every published system, by name.
    pub fn list(&self) -> Vec<Published> {
        let mut out: Vec<Published> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read(&e.path()))
            .collect();
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    pub fn save(&self, p: &Published) -> std::io::Result<()> {
        let path = self
            .path(&p.id)
            .ok_or_else(|| std::io::Error::other("bad design system id"))?;
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_string_pretty(p).expect("design system serializes");
        crate::io::atomic::write(&path, json.as_bytes())
    }

    pub fn remove(&self, id: &str) -> std::io::Result<()> {
        match self.path(id) {
            Some(p) if p.exists() => std::fs::remove_file(p),
            _ => Ok(()),
        }
    }
}

fn read(path: &Path) -> Option<Published> {
    let s = std::fs::read_to_string(path).ok()?;
    let p: Published = serde_json::from_str(&s)
        .map_err(|e| log::warn!("{}: {e}", path.display()))
        .ok()?;
    (p.format == FORMAT && p.version <= VERSION).then_some(p)
}

/// Systems other projects can use: published, with the source project in the store and not
/// in the trash.
pub fn available(store: &Store) -> Vec<Published> {
    let sources: HashSet<String> = store.list().into_iter().map(|e| e.manifest.id).collect();
    Registry::in_store(store)
        .list()
        .into_iter()
        .filter(|p| sources.contains(&p.source))
        .collect()
}

/// Whether a system a project uses can still change: false once it's unpublished or its
/// source project is trashed or deleted. The project keeps its copy either way.
pub fn source_available(store: &Store, id: &str) -> bool {
    Registry::in_store(store)
        .get(id)
        .is_some_and(|p| store.list().iter().any(|e| e.manifest.id == p.source))
}

/// `names` joined, at most `max` of them, then "and N more".
pub fn name_list(names: &[String], max: usize) -> String {
    let mut s = names
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > max {
        s.push_str(&format!(" and {} more", names.len() - max));
    }
    s
}

// ---------------------------------------------------------------------------- publishing

/// What to publish. None keeps what the last publish used (or a default).
#[derive(Debug, Clone, Default)]
pub struct PublishRequest<'a> {
    pub name: Option<&'a str>,
    pub prefix: Option<&'a str>,
    /// An empty string clears it.
    pub description: Option<&'a str>,
    /// An empty string clears it.
    pub notes: Option<&'a str>,
}

/// What a publish changed. Token lists hold CSS names.
#[derive(Debug, Clone, PartialEq)]
pub struct PublishReport {
    pub system: Published,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    /// Other projects that use the system and will update when they next open.
    pub users: usize,
}

impl PublishReport {
    pub fn message(&self) -> String {
        let s = &self.system;
        let mut parts = Vec::new();
        for (what, list) in [
            ("added", &self.added),
            ("changed", &self.changed),
            ("removed", &self.removed),
        ] {
            if !list.is_empty() {
                parts.push(format!("{what} {}", name_list(list, 20)));
            }
        }
        let mut m = format!("Published {} v{}", s.name, s.release);
        if !parts.is_empty() && s.release > 1 {
            m.push_str(&format!(": {}", parts.join("; ")));
        }
        m.push('.');
        if self.users > 0 {
            m.push_str(&format!(
                " {} will update when opened.",
                crate::editor::systems::plural(self.users, "file")
            ));
        }
        m
    }
}

/// Check a prefix: lowercase letters, digits and single dashes, starting with a letter.
pub fn check_prefix(prefix: &str) -> Result<(), String> {
    let ok = prefix.len() <= MAX_PREFIX
        && prefix.starts_with(|c: char| c.is_ascii_lowercase())
        && !prefix.ends_with('-')
        && !prefix.contains("--")
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "Prefix \"{prefix}\" must start with a letter and use only lowercase letters, \
             digits and single dashes, at most {MAX_PREFIX} characters."
        ))
    }
}

/// A prefix made from a system name.
pub fn default_prefix(name: &str) -> String {
    let slug = css_slug(&name.replace('_', "-"));
    let mut p: String = slug
        .trim_start_matches(|c: char| !c.is_ascii_lowercase())
        .chars()
        .take(MAX_PREFIX)
        .collect();
    while p.ends_with('-') {
        p.pop();
    }
    if p.is_empty() { "ds".into() } else { p }
}

fn limit(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.chars().count() > max {
        return Err(format!("{what} can be at most {max} characters."));
    }
    Ok(())
}

/// A text setting: the request's value (empty clears it), else the previous one.
fn setting(req: Option<&str>, prev: Option<&String>) -> Option<String> {
    match req {
        Some(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        None => prev.cloned(),
    }
}

/// Publish the open project's local tokens as a new version of its design system. Not an
/// undo step: to roll back, publish again.
pub fn publish(
    store: &Store,
    ed: &mut Editor,
    project_id: &str,
    project_name: &str,
    req: &PublishRequest,
) -> Result<PublishReport, String> {
    let prev = ed.published.clone();
    let reg = Registry::in_store(store);
    let old = prev.as_ref().and_then(|l| reg.get(&l.id));
    let name = req
        .name
        .map(str::trim)
        .map(String::from)
        .or_else(|| prev.as_ref().map(|l| l.name.clone()))
        .unwrap_or_else(|| project_name.trim().to_string());
    if name.is_empty() {
        return Err("A design system needs a name.".into());
    }
    limit("A design system name", &name, MAX_NAME)?;
    let prefix = match (&prev, req.prefix.map(str::trim)) {
        (Some(l), Some(p)) if p != l.prefix => {
            return Err(format!(
                "The prefix is fixed after the first publish, so it stays \"{}\". Changing it \
                 would rename every variable files use.",
                l.prefix
            ));
        }
        (Some(l), _) => l.prefix.clone(),
        (None, Some(p)) => p.to_string(),
        (None, None) => default_prefix(&name),
    };
    check_prefix(&prefix)?;
    let id = prev.as_ref().map_or_else(new_id, |l| l.id.clone());
    if let Some(other) = reg
        .list()
        .into_iter()
        .find(|p| p.id != id && p.prefix == prefix)
    {
        return Err(format!(
            "The {} design system already uses the prefix \"{prefix}\". Pick another one.",
            other.name
        ));
    }
    if ed.tokens.systems().iter().any(|s| s.id == id) {
        return Err("A file can't publish a design system it uses.".into());
    }
    let description = setting(
        req.description,
        prev.as_ref().and_then(|l| l.description.as_ref()),
    );
    let notes = setting(req.notes, prev.as_ref().and_then(|l| l.notes.as_ref()));
    if let Some(d) = &description {
        limit("The description", d, MAX_DESCRIPTION)?;
    }
    if let Some(n) = &notes {
        limit("The notes", n, MAX_NOTES)?;
    }
    let tokens = ed.tokens.publishable();
    let text_styles = ed.tokens.publishable_styles();
    if tokens.is_empty() && text_styles.is_empty() {
        return Err(
            "Add at least one token or text style before publishing a design system.".into(),
        );
    }
    let release = old
        .as_ref()
        .map(|o| o.release)
        .max(prev.as_ref().map(|l| l.release))
        .unwrap_or(0)
        + 1;
    let system = Published {
        format: FORMAT.into(),
        version: VERSION,
        id,
        name,
        prefix,
        description,
        notes,
        release,
        source: project_id.to_string(),
        published: now_ms(),
        tokens,
        text_styles,
        modes: ed.tokens.modes().to_vec(),
    };
    let report = report(store, old.as_ref(), system);
    reg.save(&report.system).map_err(|e| e.to_string())?;
    ed.published = Some(report.system.link());
    ed.meta_dirty = true;
    Ok(report)
}

/// Tokens and text styles a publish would add, change and remove, as CSS names and classes
/// with `prefix`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

pub fn changes(
    old: Option<&Published>,
    prefix: &str,
    tokens: &[Token],
    styles: &[TextStyleDef],
) -> Changes {
    let mut c = Changes::default();
    let old_tokens = old.map(|o| o.tokens.as_slice()).unwrap_or_default();
    diff_into(
        &mut c,
        old_tokens,
        tokens,
        |t| t.id.as_str(),
        |t| format!("--{prefix}-{}", css_slug(&t.name)),
    );
    let old_styles = old.map(|o| o.text_styles.as_slice()).unwrap_or_default();
    diff_into(
        &mut c,
        old_styles,
        styles,
        |s| s.id.as_str(),
        |s| format!(".{prefix}-{}", style_slug(&s.name)),
    );
    c
}

fn diff_into<T: PartialEq>(
    c: &mut Changes,
    old: &[T],
    new: &[T],
    id: impl Fn(&T) -> &str,
    name: impl Fn(&T) -> String,
) {
    let find = |list: &'_ [T], key: &str| list.iter().position(|t| id(t) == key);
    for t in new {
        match find(old, id(t)) {
            None => c.added.push(name(t)),
            Some(i) if old[i] != *t => c.changed.push(name(t)),
            Some(_) => {}
        }
    }
    c.removed
        .extend(old.iter().filter(|o| find(new, id(o)).is_none()).map(&name));
}

/// Projects in the store that use design system `id`.
pub fn users(store: &Store, id: &str) -> usize {
    store
        .list()
        .iter()
        .filter(|e| e.manifest.systems.iter().any(|s| s.id == id))
        .count()
}

fn report(store: &Store, old: Option<&Published>, system: Published) -> PublishReport {
    let Changes {
        added,
        changed,
        removed,
    } = changes(old, &system.prefix, &system.tokens, &system.text_styles);
    let users = users(store, &system.id);
    PublishReport {
        system,
        added,
        changed,
        removed,
        users,
    }
}

/// Whether the open project's tokens or settings differ from what it last published. False
/// when it has never published.
pub fn has_unpublished_changes(store: &Store, ed: &Editor) -> bool {
    let Some(l) = &ed.published else { return false };
    match Registry::in_store(store).get(&l.id) {
        Some(p) => {
            p.tokens != ed.tokens.publishable()
                || p.text_styles != ed.tokens.publishable_styles()
                || p.modes != ed.tokens.modes()
                || p.link() != *l
        }
        None => true,
    }
}

/// Remove the open project's design system from the registry. Files that use it keep their
/// copy. The project keeps the settings, so publishing again brings those files back.
pub fn unpublish(store: &Store, ed: &Editor) -> Result<String, String> {
    let l = ed
        .published
        .as_ref()
        .ok_or("This file hasn't published a design system.")?;
    Registry::in_store(store)
        .remove(&l.id)
        .map_err(|e| e.to_string())?;
    Ok(format!("Unpublished {}.", l.name))
}

/// Move every design system the open project uses to its latest version. Returns a message
/// per update.
pub fn update_all(store: &Store, ed: &mut Editor) -> Vec<String> {
    let reg = Registry::in_store(store);
    let ids: Vec<String> = ed.tokens.systems().iter().map(|s| s.id.clone()).collect();
    ids.iter()
        .filter_map(|id| reg.get(id))
        .filter_map(|p| ed.update_system(&p))
        .map(|u| u.message())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Color, Node, NodeKind, TokenKind, TokenProp};
    use crate::session::Session;

    fn temp_store() -> Store {
        let root =
            std::env::temp_dir().join(format!("sketch-rs-systems-{:016x}", fastrand::u64(..)));
        Store::open(root).unwrap()
    }

    /// A store with a source project that has `brand` (#ff0000) and `space` (8px).
    fn source(store: &Store) -> (Session, Editor) {
        let mut s = Session::new(store.clone());
        let mut ed = Editor::default();
        s.create(&mut ed, "Brand").unwrap();
        ed.edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "brand", "#ff0000", None)
                .unwrap();
            t.create(TokenKind::Spacing, "space", "8px", None).unwrap();
        });
        (s, ed)
    }

    fn publish_open(s: &Session, ed: &mut Editor, req: &PublishRequest) -> PublishReport {
        let p = s.project.as_ref().unwrap();
        publish(&s.store, ed, &p.manifest.id, &p.manifest.name, req).unwrap()
    }

    fn consumer(store: &Store, system: &Published) -> (Session, Editor, crate::model::NodeId) {
        let mut s = Session::new(store.clone());
        let mut ed = Editor::default();
        s.create(&mut ed, "App").unwrap();
        ed.use_system(system).unwrap();
        let id = ed.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(
                Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0)),
                None,
                None,
            )
        });
        let token = ed.tokens.by_css_name("--brand-brand").unwrap().id.clone();
        ed.bind_token(&[id], TokenProp::Fill, &token);
        s.save(&mut ed).unwrap();
        (s, ed, id)
    }

    fn fill(ed: &Editor, id: crate::model::NodeId) -> Option<Color> {
        ed.doc.get(id).unwrap().style.fill()
    }

    #[test]
    fn publishing_writes_the_registry_and_counts_versions() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.system.release, 1);
        assert_eq!(r.system.prefix, "brand");
        assert_eq!(r.added, ["--brand-brand", "--brand-space"]);
        assert!(!has_unpublished_changes(&store, &ed));

        let id = ed.tokens.by_css_name("--space").unwrap().id.clone();
        ed.edit_tokens("edit", None, |t| t.update(&id, None, Some("12px"), None))
            .unwrap();
        assert!(has_unpublished_changes(&store, &ed));
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.system.release, 2);
        assert_eq!(r.changed, ["--brand-space"]);
        assert!(r.added.is_empty());
        assert_eq!(available(&store).len(), 1);
    }

    #[test]
    fn the_prefix_is_fixed_and_unique() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        publish_open(&s, &mut ed, &PublishRequest::default());
        let p = s.project.as_ref().unwrap();
        let req = PublishRequest {
            prefix: Some("other"),
            ..Default::default()
        };
        let err = publish(&store, &mut ed, &p.manifest.id, "Brand", &req).unwrap_err();
        assert!(err.contains("fixed"), "{err}");

        let (s2, mut ed2) = source(&store);
        let p2 = s2.project.as_ref().unwrap();
        let err = publish(
            &store,
            &mut ed2,
            &p2.manifest.id,
            "Brand",
            &PublishRequest::default(),
        )
        .unwrap_err();
        assert!(err.contains("already uses the prefix"), "{err}");
        assert!(check_prefix("Bad").is_err());
        assert!(check_prefix("a--b").is_err());
        assert!(check_prefix("brand-2").is_ok());
        assert_eq!(default_prefix("2 Acme Design!"), "acme-design");
        assert_eq!(default_prefix("My__System"), "my-system");
    }

    #[test]
    fn using_a_system_names_tokens_with_its_prefix_and_locks_them() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let (_c, mut app, id) = consumer(&store, &r.system);
        assert_eq!(fill(&app, id), Some(Color::rgb(255, 0, 0)));
        let t = app.tokens.by_css_name("--brand-brand").unwrap().id.clone();
        let err = app
            .edit_tokens("edit", None, |s| s.update(&t, None, Some("#000000"), None))
            .unwrap_err();
        assert!(err.contains("belongs to the Brand design system"), "{err}");
        app.edit_tokens("delete", None, |s| s.delete(std::slice::from_ref(&t)));
        assert!(app.tokens.get(&t).is_some());
        let err = app
            .edit_tokens("add", None, |s| {
                s.create(TokenKind::Color, "brand-brand", "#000000", None)
            })
            .unwrap_err();
        assert!(err.contains("design system"), "{err}");
        let alias = app
            .edit_tokens("add", None, |s| {
                s.create(TokenKind::Color, "cta", "var(--brand-brand)", None)
            })
            .unwrap();
        assert_eq!(app.tokens.resolve(&alias), app.tokens.resolve(&t));
        assert!(app.use_system(&r.system).is_err());
    }

    #[test]
    fn opening_a_file_moves_it_to_the_latest_version() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let (c, _, id) = consumer(&store, &r.system);
        let dir = c.project.as_ref().unwrap().dir.clone();

        let brand = ed.tokens.by_css_name("--brand").unwrap().id.clone();
        let space = ed.tokens.by_css_name("--space").unwrap().id.clone();
        ed.edit_tokens("edit", None, |t| {
            t.update(&brand, None, Some("#0000ff"), None).unwrap();
            t.delete(std::slice::from_ref(&space));
        });
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.users, 1);
        assert_eq!(r.removed, ["--brand-space"]);

        let mut c = Session::new(store.clone());
        let mut app = Editor::default();
        c.open(&mut app, &dir).unwrap();
        assert_eq!(fill(&app, id), Some(Color::rgb(0, 0, 255)));
        assert_eq!(app.tokens.system(&r.system.id).unwrap().release, 2);
        assert!(app.tokens.by_css_name("--brand-space").is_none());
        assert!(app.status.contains("Updated Brand to v2"), "{}", app.status);

        // Undo goes back to v1 for this session.
        app.undo();
        assert_eq!(app.tokens.system(&r.system.id).unwrap().release, 1);
        assert_eq!(fill(&app, id), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn removing_a_token_reports_the_layers_bound_to_it() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let (_c, mut app, id) = consumer(&store, &r.system);
        let brand = ed.tokens.by_css_name("--brand").unwrap().id.clone();
        ed.edit_tokens("delete", None, |t| t.delete(std::slice::from_ref(&brand)));
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let u = app.update_system(&r.system).unwrap();
        assert_eq!(u.removed, ["--brand-brand"]);
        assert_eq!(u.unbound, 1);
        assert_eq!(fill(&app, id), Some(Color::rgb(255, 0, 0)));
        assert!(
            app.bound_token(app.doc.get(id).unwrap(), TokenProp::Fill)
                .is_none()
        );
    }

    #[test]
    fn aliases_to_other_systems_are_flattened_when_publishing() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let base = publish_open(&s, &mut ed, &PublishRequest::default()).system;

        let mut s2 = Session::new(store.clone());
        let mut ed2 = Editor::default();
        s2.create(&mut ed2, "Marketing").unwrap();
        ed2.use_system(&base).unwrap();
        ed2.edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "hero", "var(--brand-brand)", None)
        })
        .unwrap();
        let r = publish_open(&s2, &mut ed2, &PublishRequest::default());
        assert_eq!(r.system.tokens.len(), 1);
        assert_eq!(
            r.system.tokens[0].value,
            crate::model::TokenValue::Color(Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn detaching_keeps_names_and_bindings_and_removing_keeps_values() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let (_c, mut app, id) = consumer(&store, &r.system);
        app.detach_system(&r.system.id).unwrap();
        let t = app.tokens.by_css_name("--brand-brand").unwrap();
        assert!(t.system.is_none());
        assert!(
            app.bound_token(app.doc.get(id).unwrap(), TokenProp::Fill)
                .is_some()
        );
        app.undo();

        assert_eq!(app.remove_system(&r.system.id).unwrap(), 1);
        assert!(app.tokens.systems().is_empty());
        assert_eq!(fill(&app, id), Some(Color::rgb(255, 0, 0)));
        app.undo();
        assert!(
            app.bound_token(app.doc.get(id).unwrap(), TokenProp::Fill)
                .is_some()
        );
    }

    #[test]
    fn unpublishing_or_trashing_the_source_makes_the_system_unavailable() {
        let store = temp_store();
        let (mut s, mut ed) = source(&store);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let id = r.system.id.clone();
        assert!(source_available(&store, &id));

        unpublish(&store, &ed).unwrap();
        assert!(!source_available(&store, &id));
        let again = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(
            (again.system.id.as_str(), again.system.release),
            (id.as_str(), 2)
        );

        s.save_all(&mut ed).unwrap();
        let dir = s.project.as_ref().unwrap().dir.clone();
        s.close(&mut ed).unwrap();
        let trashed = store.trash(&dir).unwrap();
        assert!(!source_available(&store, &id));
        assert!(available(&store).is_empty());
        store.restore(&trashed).unwrap();
        assert!(source_available(&store, &id));
    }

    #[test]
    fn a_duplicated_source_does_not_publish_the_same_system() {
        let store = temp_store();
        let (mut s, mut ed) = source(&store);
        publish_open(&s, &mut ed, &PublishRequest::default());
        s.save_all(&mut ed).unwrap();
        let dir = s.project.as_ref().unwrap().dir.clone();
        let copy = store.duplicate(&dir).unwrap();
        let p = crate::project::Project::open(&copy).unwrap();
        assert!(p.manifest.published.is_none());
    }

    fn add_heading(ed: &mut Editor) -> String {
        ed.edit_tokens("add", None, |t| {
            t.create(crate::model::TokenKind::FontSize, "size", "24px", None)
                .unwrap();
            let mut i = crate::model::StyleInput::default();
            i.set(crate::model::StyleField::Size, "var(--size)".into());
            t.create_style("Heading", &i, None).unwrap()
        })
    }

    fn text_layer(ed: &mut Editor) -> crate::model::NodeId {
        ed.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(
                Node::new(
                    id,
                    NodeKind::Text {
                        text: "Hi".into(),
                        font_size: 16.0,
                        auto_width: true,
                        align: Default::default(),
                    },
                    Rect::new(0.0, 0.0, 10.0, 10.0),
                ),
                None,
                None,
            )
        })
    }

    #[test]
    fn text_styles_publish_with_the_prefix_and_follow_new_versions() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let heading = add_heading(&mut ed);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert!(
            r.added.contains(&".brand-heading".to_string()),
            "{:?}",
            r.added
        );
        let (c, mut app, _) = consumer(&store, &r.system);
        let style = app
            .tokens
            .style_by_name(".brand-heading")
            .unwrap()
            .id
            .clone();
        let err = app
            .edit_tokens("edit", None, |t| {
                t.update_style(&style, Some("x"), &Default::default(), None)
            })
            .unwrap_err();
        assert!(err.contains("design system"), "{err}");
        let id = text_layer(&mut app);
        app.apply_text_style(&[id], &style);
        assert_eq!(app.doc.get(id).unwrap().kind.font_size(), Some(24.0));
        let mut c = c;
        c.save(&mut app).unwrap();
        let dir = c.project.as_ref().unwrap().dir.clone();

        let size = ed.tokens.by_css_name("--size").unwrap().id.clone();
        ed.edit_tokens("edit", None, |t| t.update(&size, None, Some("40px"), None))
            .unwrap();
        assert!(has_unpublished_changes(&store, &ed));
        publish_open(&s, &mut ed, &PublishRequest::default());
        let mut c = Session::new(store.clone());
        let mut app = Editor::default();
        c.open(&mut app, &dir).unwrap();
        assert_eq!(app.doc.get(id).unwrap().kind.font_size(), Some(40.0));

        ed.edit_tokens("delete", None, |t| t.delete_styles(&[heading]));
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.removed, [".brand-heading"]);
        let u = app.update_system(&r.system).unwrap();
        assert_eq!(u.removed, [".brand-heading"]);
        assert_eq!(u.unbound, 1);
        assert_eq!(app.doc.get(id).unwrap().kind.font_size(), Some(40.0));
        assert!(app.bound_style(app.doc.get(id).unwrap()).is_none());
    }

    #[test]
    fn detaching_a_system_keeps_its_style_classes() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        add_heading(&mut ed);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        let (_c, mut app, _) = consumer(&store, &r.system);
        let style = app
            .tokens
            .style_by_name(".brand-heading")
            .unwrap()
            .id
            .clone();
        app.detach_system(&r.system.id).unwrap();
        let st = app.tokens.style(&style).unwrap();
        assert!(st.system.is_none());
        assert_eq!(app.tokens.style_class(st), "brand-heading");
        app.undo();
        app.remove_system(&r.system.id).unwrap();
        assert!(app.tokens.styles().is_empty());
    }

    #[test]
    fn linked_ids_only_resolve_where_the_system_is_used() {
        let mut a = TokenSet::default();
        a.create(TokenKind::Color, "x", "#ffffff", None).unwrap();
        let p = Published {
            format: FORMAT.into(),
            version: VERSION,
            id: "abc".into(),
            name: "Abc".into(),
            prefix: "abc".into(),
            description: None,
            notes: None,
            release: 1,
            source: "src".into(),
            published: 0,
            tokens: a.publishable(),
            text_styles: Vec::new(),
            modes: Vec::new(),
        };
        let mut used = TokenSet::default();
        used.attach(p.link(), &p.tokens, &[]).unwrap();
        let id = crate::model::tokens::linked_id("abc", &p.tokens[0].id);
        assert!(used.get(&id).is_some());
        assert!(TokenSet::default().get(&id).is_none());
        assert_eq!(p.token_set().css_name(used.get(&id).unwrap()), "--abc-x");
    }
}
