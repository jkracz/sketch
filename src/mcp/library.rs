//! Personal library and Excalidraw import tools.
//!
//! The library is the one the app's Library tab shows (`<projects folder>/library.json`),
//! shared by every project. Changes save at once and are not undo steps, as in the app.

use super::ToolResult;
use super::exec::{Args, Host, free_spot, node_key, page_arg, parent_arg, r2, resolve_all, text};
use super::tools::{page_id, tool};
use crate::editor::Editor;
use crate::geom::{V2, v2};
use crate::library::{LibItem, Library};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::path::PathBuf;

const GLOBAL: [&str; 2] = ["list_library_items", "update_library_items"];

pub fn definitions() -> Vec<Value> {
    let ids = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    let item_id = json!({"type": "string", "description": "A library item's id or name from list_library_items."});
    let mut v = vec![
        tool(
            "list_library_items",
            "Lists reusable shapes from the app's Library tab, shared across projects. \
             Each item has id, name, size and layer count. \
             Works without an open project.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "add_library_item",
            "Saves copies of layers to the personal library as one item. \
             The layers stay on the canvas. Instances become plain copies. \
             The name defaults to the first layer's. Returns the item.",
            json!({
                "nodeIds": ids("Layers to save, on one page."),
                "name": {"type": "string"},
                "pageId": page_id(),
            }),
            &["nodeIds"],
            false,
        ),
        tool(
            "insert_library_item",
            "Places a copy of a library item on a page, as one undo step. It is centered on \
             x/y (world units) when given, else placed in free space to the right of the \
             page's content. Returns the new top-level layer ids. Move them into a frame with \
             move_nodes.",
            json!({
                "itemId": item_id.clone(),
                "x": {"type": "number"}, "y": {"type": "number"},
                "pageId": page_id(),
            }),
            &["itemId"],
            false,
        ),
        tool(
            "update_library_items",
            "Renames or removes personal library items: {itemId, name} renames, {itemId, \
             delete: true} removes. Each update names a different item, by its id or current \
             name; if any can't be found, nothing changes. Removing an item can't be undone, so \
             confirm with the user first. Layers placed from it stay. Works without an open \
             project.",
            json!({"updates": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "itemId": item_id,
                    "name": {"type": "string", "minLength": 1},
                    "delete": {"type": "boolean"},
                },
                "required": ["itemId"],
            }}}),
            &["updates"],
            false,
        ),
        tool(
            "import_excalidraw",
            "Imports an Excalidraw file. Give \
             `path` (a .excalidraw or .excalidrawlib file) or `data` (its JSON, as a string or \
             object). A scene lands on the page as one undo step: shapes, text, arrows with \
             their bindings, freehand lines and images, with the hand-drawn look. It is \
             centered on x/y when given, else placed in free space. With createFile: true the \
             scene becomes a new project instead (name and parentId as in create_file), and \
             the result has its fileId for open_file. A library (.excalidrawlib) adds its \
             items to the personal library.",
            json!({
                "path": {"type": "string", "description": "A local file path."},
                "data": {"anyOf": [{"type": "string"}, {"type": "object"}], "description": "The file's JSON instead of a path."},
                "x": {"type": "number"}, "y": {"type": "number"},
                "createFile": {"type": "boolean", "description": "Make a new project from the scene."},
                "name": {"type": "string", "description": "The new project's name with createFile. Defaults to the file name."},
                "parentId": {"type": "string", "description": "Folder for the new project with createFile."},
                "pageId": page_id(),
            }),
            &[],
            false,
        ),
    ];
    for t in &mut v {
        if GLOBAL.contains(&t["name"].as_str().unwrap_or(""))
            && let Some(p) = t["inputSchema"]["properties"].as_object_mut()
        {
            p.remove("fileId");
        }
        if t["name"] == "update_library_items" {
            t["annotations"]["destructiveHint"] = json!(true);
        }
    }
    v
}

/// Whether a call needs no open project: the library list and edits, and an import that
/// makes a new project or fills the library.
pub(super) fn is_global(name: &str, a: &Args) -> bool {
    GLOBAL.contains(&name)
        || (name == "import_excalidraw"
            && (a.get("createFile").and_then(Value::as_bool) == Some(true)
                || source(a).is_ok_and(|(json, _)| is_library(&json))))
}

pub(super) fn call_global(host: &mut dyn Host, name: &str, a: &Args) -> ToolResult {
    match name {
        "list_library_items" => list_items(host),
        "update_library_items" => update_items(host, a),
        _ => import_as_file_or_library(host, a),
    }
}

/// Tools that work on the open project, or None.
pub(super) fn call(host: &mut dyn Host, name: &str, a: &Args) -> Option<ToolResult> {
    Some(match name {
        "add_library_item" => add_item(host, a),
        "insert_library_item" => insert_item(host, a),
        "import_excalidraw" => import_into_page(host, a),
        _ => return None,
    })
}

fn item_json(it: &LibItem) -> Value {
    let b = it.bounds().unwrap_or_default();
    json!({
        "id": it.id,
        "name": it.name,
        "width": r2(b.w),
        "height": r2(b.h),
        "layers": it.nodes.len(),
    })
}

fn save(lib: &Library) -> Result<(), String> {
    lib.save()
        .map_err(|e| format!("could not save the library: {e}"))
}

/// The index of an item by id, else by a name only one item has (ignoring case).
fn find_item(lib: &Library, key: &str) -> Result<usize, String> {
    let key = key.trim();
    if let Some(i) = lib.items.iter().position(|x| x.id == key) {
        return Ok(i);
    }
    let mut named = lib
        .items
        .iter()
        .enumerate()
        .filter(|(_, x)| x.name.eq_ignore_ascii_case(key))
        .map(|(i, _)| i);
    let Some(index) = named.next() else {
        return Err(format!(
            "no library item `{key}`; call list_library_items for ids"
        ));
    };
    let count = 1 + named.count();
    if count > 1 {
        Err(format!(
            "{count} library items are named `{key}`; pass an id from list_library_items"
        ))
    } else {
        Ok(index)
    }
}

fn list_items(host: &mut dyn Host) -> ToolResult {
    let lib = host.library();
    text(json!({
        "items": lib.items.iter().map(item_json).collect::<Vec<_>>(),
    }))
}

fn update_items(host: &mut dyn Host, a: &Args) -> ToolResult {
    let lib = host.library();
    // Check every update first, so a bad one changes nothing.
    let mut plan: Vec<(String, Option<&str>, bool)> = Vec::new();
    for (i, u) in a.arr("updates")?.iter().enumerate() {
        let u = Args(u.as_object().ok_or("each update must be an object")?);
        let at = find_item(lib, u.req_str("itemId")?).map_err(|e| format!("updates[{i}]: {e}"))?;
        let id = &lib.items[at].id;
        if plan.iter().any(|(planned_id, _, _)| planned_id == id) {
            return Err(format!("updates[{i}]: the item is already in this call"));
        }
        let delete = u.get("delete").and_then(Value::as_bool) == Some(true);
        let name = u.str("name").filter(|n| !n.is_empty());
        if !delete && name.is_none() {
            return Err(format!("updates[{i}]: pass `name` or `delete: true`"));
        }
        plan.push((id.clone(), name, delete));
    }
    let (mut renamed, mut removed) = (Vec::new(), Vec::new());
    for (id, name, delete) in plan {
        if delete {
            lib.items.retain(|x| x.id != id);
            removed.push(id);
        } else if let (Some(n), Some(it)) = (name, lib.items.iter_mut().find(|x| x.id == id)) {
            it.name = n.into();
            renamed.push(item_json(it));
        }
    }
    save(lib)?;
    text(json!({"renamed": renamed, "removed": removed}))
}

fn add_item(host: &mut dyn Host, a: &Args) -> ToolResult {
    let ed = host.editor();
    let (page, ids) = resolve_all(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    if ids.is_empty() {
        return Err("nodeIds is empty".into());
    }
    let doc = ed.page_doc(page);
    let ids: Vec<_> = (ids.iter().copied())
        .filter(|id| !doc.ancestors(*id).iter().any(|p| ids.contains(p)))
        .collect();
    let name = a
        .str("name")
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| doc.get(ids[0]).map_or("Item", |n| n.name.as_str()));
    let item = LibItem::from_selection(doc, &ids, name).ok_or("those layers have no size")?;
    let v = item_json(&item);
    let lib = host.library();
    lib.items.push(item);
    save(lib)?;
    text(v)
}

fn target(ed: &Editor, page: usize, a: &Args, w: f64, h: f64) -> V2 {
    match (a.num("x"), a.num("y")) {
        (Some(x), Some(y)) => v2(x, y),
        _ => free_spot(ed.page_doc(page), w) + v2(w * 0.5, h * 0.5),
    }
}

fn insert_item(host: &mut dyn Host, a: &Args) -> ToolResult {
    let lib = host.library();
    let item = lib.items[find_item(lib, a.req_str("itemId")?)?].clone();
    let ed = host.editor();
    let page = page_arg(ed, a)?;
    let b = item.bounds().unwrap_or_default();
    let at = target(ed, page, a, b.w, b.h);
    let world: Vec<_> = (item.roots.iter())
        .filter_map(|r| item.nodes.iter().find(|n| n.id == *r).map(|n| n.rect))
        .collect();
    let offset = (at - b.center()).round();
    let ids = ed.with_page(page, |ed| {
        ed.insert_subtrees(
            &format!("Insert {}", item.name),
            &item.nodes,
            &item.roots,
            &world,
            offset,
            None,
        )
    });
    text(json!({
        "ids": ids.iter().map(|id| node_key(ed, page, *id)).collect::<Vec<_>>(),
        "pageId": ed.pages[page].id,
        "item": item.name,
    }))
}

/// The Excalidraw JSON from `path` or `data`, and a name for it.
fn source<'a>(a: &Args<'a>) -> Result<(Cow<'a, str>, String), String> {
    if let Some(d) = a.get("data") {
        let s = match d {
            Value::String(s) => Cow::Borrowed(s.as_str()),
            v => Cow::Owned(v.to_string()),
        };
        return Ok((s, "Excalidraw".into()));
    }
    let p = a
        .str("path")
        .filter(|p| !p.is_empty())
        .ok_or("pass `path` or `data`")?;
    let path = match p.strip_prefix("~/") {
        Some(rest) => {
            std::env::var_os("HOME").map_or(PathBuf::from(p), |h| PathBuf::from(h).join(rest))
        }
        None => PathBuf::from(p),
    };
    let s = std::fs::read_to_string(&path).map_err(|e| format!("could not read {p}: {e}"))?;
    let name = path
        .file_stem()
        .map_or("Excalidraw".into(), |s| s.to_string_lossy().to_string());
    Ok((Cow::Owned(s), name))
}

fn is_library(json: &str) -> bool {
    serde_json::from_str::<Value>(json)
        .ok()
        .is_some_and(|v| v.get("type").and_then(Value::as_str) == Some("excalidrawlib"))
}

fn import_library(host: &mut dyn Host, json: &str) -> ToolResult {
    let items = crate::io::excalidraw::import_library(json)?;
    let added: Vec<Value> = items.iter().map(item_json).collect();
    let lib = host.library();
    lib.items.extend(items);
    save(lib)?;
    text(json!({"library": true, "added": added}))
}

fn import_into_page(host: &mut dyn Host, a: &Args) -> ToolResult {
    let (json, name) = source(a)?;
    let mut doc = crate::io::excalidraw::import_scene(&json)?;
    let nodes: Vec<_> = (doc.paint_order().into_iter())
        .filter_map(|id| doc.nodes.remove(&id))
        .collect();
    if nodes.is_empty() {
        return Err("the scene has no elements".into());
    }
    let (roots, world) = crate::io::clipboard::roots_of(&nodes);
    let b = crate::io::clipboard::bounds(&world).unwrap_or_default();
    let ed = host.editor();
    let page = page_arg(ed, a)?;
    let at = target(ed, page, a, b.w, b.h);
    let off = crate::io::clipboard::center_offset(&world, at);
    let ids = ed.with_page(page, |ed| {
        ed.insert_subtrees(&format!("Import {name}"), &nodes, &roots, &world, off, None)
    });
    let r = b.translate(off);
    text(json!({
        "ids": ids.iter().map(|id| node_key(ed, page, *id)).collect::<Vec<_>>(),
        "pageId": ed.pages[page].id,
        "layers": nodes.len(),
        "worldX": r2(r.x), "worldY": r2(r.y), "width": r2(r.w), "height": r2(r.h),
    }))
}

fn import_as_file_or_library(host: &mut dyn Host, a: &Args) -> ToolResult {
    let (json, stem) = source(a)?;
    if is_library(&json) {
        if a.get("createFile").and_then(Value::as_bool) != Some(true) {
            return import_library(host, &json);
        }
        return Err(
            "that is an Excalidraw library; import it without createFile to add its items to \
             the personal library"
                .into(),
        );
    }
    let doc = crate::io::excalidraw::import_scene(&json)?;
    let folder = match a.str("parentId").filter(|k| !k.is_empty()) {
        Some(k) => parent_arg(host.store(), k)?,
        None => None,
    };
    let name = a.str("name").filter(|n| !n.is_empty()).unwrap_or(&stem);
    let page = crate::project::PageData {
        id: crate::project::new_id(),
        name: "Page 1".into(),
        doc,
        camera: None,
    };
    let page_id = page.id.clone();
    let p = host
        .store()
        .create_with_pages_in(name, vec![page], folder.as_ref())
        .map_err(|e| e.to_string())?;
    let mut v = json!({
        "fileId": p.manifest.id,
        "name": p.manifest.name,
        "path": p.dir,
        "pageId": page_id,
        "next": "call open_file with this fileId to work in it",
    });
    if let Some(f) = folder {
        v["parentId"] = json!(f.id);
        v["folder"] = json!(f.name);
    }
    text(v)
}
