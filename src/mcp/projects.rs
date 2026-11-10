//! Project tools beyond create, open, move and rename: duplicate a project, move projects and
//! folders to the trash and back, and unpublish the open file's design system.
//!
//! Deleting forever and emptying the trash stay in the app: they can't be undone.

use super::ToolResult;
use super::exec::{
    Args, Host, check_file, find_folder, find_project, location, lookup_project, same_dir, text,
};
use super::tools::tool;
use crate::project::{Folder, Project, TrashEntry};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const NAMES: [&str; 5] = [
    "duplicate_file",
    "list_trash",
    "trash_resources",
    "restore_resources",
    "unpublish_design_system",
];

pub fn definitions() -> Vec<Value> {
    let keys = |d: &str| json!({"type": "array", "minItems": 1, "items": {"type": "string"}, "description": d});
    let mut v = vec![
        tool(
            "duplicate_file",
            "Copies a project with all its pages into the same folder and returns the copy's \
             fileId. The copy uses the original's design systems but doesn't publish the \
             original's. The open project can be copied; it is saved first. Call open_file to \
             work in the copy.",
            json!({
                "fileId": {"type": "string", "minLength": 1, "description": "Project id, name, Folder/Project location or path (see list_resources)."},
                "name": {"type": "string", "description": "Name of the copy. Defaults to \"<name> copy\"; a taken name gets the next free one."},
            }),
            &["fileId"],
            false,
        ),
        tool(
            "list_trash",
            "Lists projects in the trash, most recently trashed first: id, name, the folder it \
             came from, when it was trashed (deletedAt, Unix ms) and its path in the trash. \
             Restore one with restore_resources.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "trash_resources",
            "Moves projects and folders to the trash, as Move to Trash does in the app. Trashing a \
             folder trashes its projects, and restoring any of them brings the folder back. \
             Trashed projects vanish from list_resources, and a design system a trashed project \
             publishes stops being available. Only the user can delete forever or empty the \
             trash. Trashing the project open in the app saves and closes it first, and the app \
             shows the home screen. Every id is checked first: if any can't be trashed, nothing \
             is. Returns the trashed items.",
            json!({"resourceIds": keys("Project or folder ids, names or Folder/Project locations from list_resources.")}),
            &["resourceIds"],
            false,
        ),
        tool(
            "restore_resources",
            "Puts trashed projects back where they were, recreating their folder if it's gone. A \
             project whose name is taken there gets the next free name. Pass a trashed folder's \
             id or name to restore every project trashed from it. Every id is checked first: if \
             any isn't in the trash, nothing is restored. Returns each restored project's id, \
             name, location and path.",
            json!({"resourceIds": keys("Ids, names or trash paths from list_trash, or the id or name of a folder they came from.")}),
            &["resourceIds"],
            false,
        ),
        tool(
            "unpublish_design_system",
            "Removes the open file's design system from the list other projects pick from. \
             Files that already use it keep their copy but stop getting updates. The file keeps \
             its publish settings, so publish_design_system brings it back for those files.",
            json!({}),
            &[],
            false,
        ),
    ];
    for t in &mut v {
        let name = t["name"].as_str().unwrap_or_default().to_string();
        if matches!(
            name.as_str(),
            "list_trash" | "trash_resources" | "restore_resources"
        ) && let Some(p) = t["inputSchema"]["properties"].as_object_mut()
        {
            p.remove("fileId");
        }
        if name == "trash_resources" {
            t["annotations"]["destructiveHint"] = json!(true);
        }
    }
    v
}

pub(super) fn call(host: &mut dyn Host, name: &str, a: &Args) -> ToolResult {
    match name {
        "duplicate_file" => duplicate_file(host, a),
        "list_trash" => list_trash(host),
        "trash_resources" => trash_resources(host, a),
        "restore_resources" => restore_resources(host, a),
        "unpublish_design_system" => unpublish(host, a),
        _ => Err(format!("unknown tool `{name}`")),
    }
}

fn file_json(dir: &std::path::Path, host: &dyn Host) -> Result<Value, String> {
    let m = Project::open(dir).map_err(|e| e.to_string())?.manifest;
    let entry = host
        .store()
        .list()
        .into_iter()
        .find(|e| same_dir(&e.dir, dir));
    let mut v = json!({
        "fileId": m.id,
        "name": m.name,
        "location": entry.as_ref().map_or(m.name.clone(), location),
        "path": dir,
    });
    if let Some(f) = entry.and_then(|e| e.folder) {
        v["parentId"] = json!(f.id);
        v["folder"] = json!(f.name);
    }
    Ok(v)
}

fn is_open(host: &dyn Host, dir: &std::path::Path) -> bool {
    host.project().is_some_and(|p| same_dir(&p.dir, dir))
}

fn duplicate_file(host: &mut dyn Host, a: &Args) -> ToolResult {
    let dir = find_project(host.store(), a.req_str("fileId")?)?;
    if is_open(host, &dir) {
        host.save()?;
    }
    let mut copy = host.store().duplicate(&dir).map_err(|e| e.to_string())?;
    if let Some(name) = a.str("name").filter(|n| !n.is_empty()) {
        copy = host
            .store()
            .rename(&copy, name)
            .map_err(|e| e.to_string())?;
    }
    let mut v = file_json(&copy, host)?;
    v["next"] = json!("call open_file with this fileId to work in it");
    text(v)
}

fn trash_json(e: &TrashEntry) -> Value {
    json!({
        "id": e.manifest.id,
        "name": e.manifest.name,
        "folder": e.folder.as_ref().map(|f| &f.name),
        "folderId": e.folder.as_ref().map(|f| &f.id),
        "deletedAt": e.deleted,
        "path": e.dir,
    })
}

fn list_trash(host: &mut dyn Host) -> ToolResult {
    let items: Vec<Value> = host.store().list_trash().iter().map(trash_json).collect();
    text(json!({"items": items}))
}

enum Doomed {
    File(PathBuf),
    Folder(Folder),
}

fn trash_resources(host: &mut dyn Host, a: &Args) -> ToolResult {
    let open = host.project().map(|p| p.dir.clone());
    let store = host.store().clone();
    let store = &store;
    let mut closes_open = false;
    let mut doomed: Vec<Doomed> = Vec::new();
    for key in a.strs("resourceIds")? {
        let item = match (lookup_project(store, &key)?, find_folder(store, &key)) {
            (Some(_), Ok(_)) => {
                return Err(format!(
                    "`{key}` is both a project and a folder. Pass its id from list_resources."
                ));
            }
            (Some(dir), Err(_)) => Doomed::File(dir),
            (None, Ok(f)) => Doomed::Folder(f),
            (None, Err(_)) => return Err(format!("`{key}` not found (see list_resources)")),
        };
        if let Some(o) = &open {
            closes_open |= match &item {
                Doomed::File(d) => same_dir(d, o),
                Doomed::Folder(f) => o.parent().is_some_and(|p| same_dir(p, &f.dir)),
            };
        }
        let dup = doomed.iter().any(|d| match (d, &item) {
            (Doomed::File(a), Doomed::File(b)) => same_dir(a, b),
            (Doomed::Folder(a), Doomed::Folder(b)) => a.id == b.id,
            _ => false,
        });
        if !dup {
            doomed.push(item);
        }
    }
    // A project inside a folder that is also listed goes with the folder.
    let folders: Vec<PathBuf> = (doomed.iter())
        .filter_map(|d| match d {
            Doomed::Folder(f) => Some(f.dir.clone()),
            Doomed::File(_) => None,
        })
        .collect();
    doomed.retain(|d| match d {
        Doomed::File(dir) => !dir
            .parent()
            .is_some_and(|p| folders.iter().any(|f| same_dir(f, p))),
        Doomed::Folder(_) => true,
    });
    // As in the app, where the user goes home before trashing the project they had open.
    if closes_open {
        host.close_project()?;
    }
    let mut trashed = Vec::new();
    for d in doomed {
        let r = match &d {
            Doomed::File(dir) => {
                let m = Project::open(dir).map_err(|e| e.to_string())?.manifest;
                store
                    .trash(dir)
                    .map(|_| json!({"type": "file", "id": m.id, "name": m.name}))
            }
            Doomed::Folder(f) => store
                .trash_folder(&f.dir)
                .map(|()| json!({"type": "folder", "id": f.id, "name": f.name})),
        };
        match r {
            Ok(v) => trashed.push(v),
            Err(e) => {
                return Err(format!(
                    "trashed {} item(s), then failed: {e}",
                    trashed.len()
                ));
            }
        }
    }
    let mut v = json!({
        "trashed": trashed,
        "hint": "restore_resources brings them back; list_trash lists the trash.",
    });
    if closes_open {
        v["closedOpenProject"] = json!(true);
        v["next"] = json!("The app shows the home screen. Call open_file to keep working.");
    }
    text(v)
}

/// Trash entries matching `key`: its id, path or name, or the id or name of its old folder.
fn trashed(list: &[TrashEntry], key: &str) -> Result<Vec<PathBuf>, String> {
    let key = key.trim();
    let tiers: [&dyn Fn(&TrashEntry) -> bool; 4] = [
        &|e| e.manifest.id == key || e.dir == std::path::Path::new(key),
        &|e| e.manifest.name == key || e.original == key,
        &|e| e.manifest.name.eq_ignore_ascii_case(key),
        &|e| {
            e.folder
                .as_ref()
                .is_some_and(|f| f.id == key || f.name.eq_ignore_ascii_case(key))
        },
    ];
    for (i, matches) in tiers.iter().enumerate() {
        let found: Vec<&TrashEntry> = list.iter().filter(|e| matches(e)).collect();
        match found[..] {
            [] => continue,
            [one] => return Ok(vec![one.dir.clone()]),
            // A folder name means every project trashed from it.
            _ if i == 3 => return Ok(found.iter().map(|e| e.dir.clone()).collect()),
            _ => {
                let options: Vec<String> = (found.iter())
                    .map(|e| format!("{} (id {})", e.manifest.name, e.manifest.id))
                    .collect();
                return Err(format!(
                    "`{key}` matches {} trashed projects: {}. Pass an id from list_trash.",
                    found.len(),
                    options.join(", ")
                ));
            }
        }
    }
    Err(format!("`{key}` is not in the trash (see list_trash)"))
}

fn restore_resources(host: &mut dyn Host, a: &Args) -> ToolResult {
    let list = host.store().list_trash();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for key in a.strs("resourceIds")? {
        for d in trashed(&list, &key)? {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    if dirs.is_empty() {
        return Err("resourceIds is empty".into());
    }
    let mut restored = Vec::new();
    for d in dirs {
        match host.store().restore(&d) {
            Ok(dir) => restored.push(file_json(&dir, host)?),
            Err(e) => {
                return Err(format!(
                    "restored {} project(s), then failed on {}: {e}",
                    restored.len(),
                    d.display()
                ));
            }
        }
    }
    text(json!({"restored": restored}))
}

fn unpublish(host: &mut dyn Host, a: &Args) -> ToolResult {
    check_file(host, a)?;
    let store = host.store().clone();
    let ed = host.editor();
    let name = ed
        .published
        .as_ref()
        .map(|l| l.name.clone())
        .ok_or("The open file hasn't published a design system.")?;
    if !crate::systems::Registry::in_store(&store)
        .list()
        .iter()
        .any(|p| ed.published.as_ref().is_some_and(|l| l.id == p.id))
    {
        return Err(format!("{name} isn't published right now."));
    }
    ed.status = crate::systems::unpublish(&store, ed)?;
    text(json!({
        "name": name,
        "result": "unpublished",
        "hint": "Files that use it keep their copy. publish_design_system publishes it again.",
    }))
}
