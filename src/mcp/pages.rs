//! Page tools: delete, duplicate and reorder pages, and move or copy layers to another page.
//!
//! Every call leaves the user on the page they are viewing, except deleting that page, which
//! shows its neighbor as the Pages list does.

use super::ToolResult;
use super::exec::{Args, ROOT, find_page, node_key, page_arg, relayout, resolve, text};
use super::tools::tool;
use crate::editor::Editor;
use crate::geom::{Rect, V2};
use crate::history::Tx;
use crate::model::{Document, Node, NodeId};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const NAMES: [&str; 4] = [
    "delete_pages",
    "duplicate_page",
    "arrange_pages",
    "move_to_page",
];

pub fn definitions() -> Vec<Value> {
    let mut v = vec![
        tool(
            "delete_pages",
            "Deletes pages and everything on them. This can't be undone, so only delete pages the \
             user asked to remove. A project keeps at least one page. Deleting the page the user \
             is viewing shows a neighboring page. Every id is checked first: if any is unknown, \
             nothing is deleted. Returns the deleted pages and the pages left.",
            json!({"pageIds": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Page ids or names from get_basic_info."}}),
            &["pageIds"],
            false,
        ),
        tool(
            "duplicate_page",
            "Copies a page with all its layers to just after it, without switching the user's \
             page, and returns the new pageId. Layers on the copy keep their numbers: \"3fa9c1:14\" \
             becomes \"<idPrefix>:14\". Main components on the page are copied as new mains.",
            json!({
                "pageId": {"type": "string", "minLength": 1, "description": "The page to copy."},
                "name": {"type": "string", "description": "Name of the copy. Defaults to \"<name> copy\"."},
            }),
            &["pageId"],
            false,
        ),
        tool(
            "arrange_pages",
            "Reorders the page list. Pass every page id from get_basic_info exactly once, in the \
             new order; a missing, unknown or repeated page is an error and changes nothing. Does \
             not switch the user's page. Returns the new page list.",
            json!({"order": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Every page id (or name), in the new order."}}),
            &["order"],
            false,
        ),
        tool(
            "move_to_page",
            "Moves layers to another page, or copies them with copy: true, like cut or copy and \
             paste in the app. Layers keep their position on the canvas, or their x/y inside \
             parentId when it's given. Ids change, because each page numbers its own layers: the \
             result maps every old id to its new one. A main component moved to another page \
             stays a main, and its instances follow it; a copied main becomes an instance of the \
             original. A move is one undo step on each page it changes.",
            json!({
                "nodeIds": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Layers to move, all on one page. Children of a listed layer go with it."},
                "toPageId": {"type": "string", "minLength": 1, "description": "The destination page id or name."},
                "parentId": {"type": "string", "description": "Optional frame or group on the destination page to put the layers in. \"root\" or omitted puts them on the canvas."},
                "copy": {"type": "boolean", "description": "Copy instead of move, leaving the originals in place. Default false."},
                "pageId": {"type": "string", "description": "The page the layers are on, for bare node numbers. Defaults to the page the user is viewing."},
            }),
            &["nodeIds", "toPageId"],
            false,
        ),
    ];
    v[0]["annotations"]["destructiveHint"] = json!(true);
    v
}

pub(super) fn call(ed: &mut Editor, name: &str, a: &Args) -> ToolResult {
    match name {
        "delete_pages" => delete_pages(ed, a),
        "duplicate_page" => duplicate_page(ed, a),
        "arrange_pages" => arrange_pages(ed, a),
        "move_to_page" => move_to_page(ed, a),
        _ => Err(format!("unknown tool `{name}`")),
    }
}

fn page_list(ed: &Editor) -> Value {
    let list: Vec<Value> = (ed.pages.iter().enumerate())
        .map(|(i, p)| json!({"pageId": p.id, "name": p.name, "isActive": i == ed.active_page}))
        .collect();
    Value::Array(list)
}

/// Page indices for `keys`, each once, in the order given.
fn pages_of(ed: &Editor, keys: &[String]) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for k in keys {
        let i = find_page(ed, k)?;
        if out.contains(&i) {
            return Err(format!("page `{k}` is listed twice"));
        }
        out.push(i);
    }
    Ok(out)
}

fn delete_pages(ed: &mut Editor, a: &Args) -> ToolResult {
    let mut doomed = pages_of(ed, &a.strs("pageIds")?)?;
    if doomed.is_empty() {
        return Err("pageIds is empty".into());
    }
    if doomed.len() >= ed.pages.len() {
        return Err("A project needs at least one page, so not every page can be deleted.".into());
    }
    let deleted: Vec<Value> = (doomed.iter())
        .map(|i| json!({"pageId": ed.pages[*i].id, "name": ed.pages[*i].name}))
        .collect();
    // From the end, so the indices left to delete stay valid.
    doomed.sort_unstable_by(|a, b| b.cmp(a));
    ed.commit_pending();
    for i in doomed {
        ed.delete_page(i);
    }
    ed.status = match &deleted[..] {
        [one] => format!("Deleted {}", one["name"].as_str().unwrap_or_default()),
        many => format!("Deleted {} pages", many.len()),
    };
    text(json!({"deleted": deleted, "pages": page_list(ed)}))
}

fn duplicate_page(ed: &mut Editor, a: &Args) -> ToolResult {
    let i = find_page(ed, a.req_str("pageId")?)?;
    let at = ed.duplicate_page_in_background(i, a.str("name"));
    let p = &ed.pages[at];
    text(json!({
        "pageId": p.id,
        "name": p.name,
        "idPrefix": &p.id[..p.id.len().min(6)],
        "pages": page_list(ed),
    }))
}

fn arrange_pages(ed: &mut Editor, a: &Args) -> ToolResult {
    let order = pages_of(ed, &a.strs("order")?)?;
    if order.len() != ed.pages.len() {
        let missing: Vec<String> = (0..ed.pages.len())
            .filter(|i| !order.contains(i))
            .map(|i| format!("{} ({})", ed.pages[i].id, ed.pages[i].name))
            .collect();
        return Err(format!(
            "order must list every page once; missing {}",
            missing.join(", ")
        ));
    }
    let ids: Vec<String> = order.iter().map(|i| ed.pages[*i].id.clone()).collect();
    for (to, id) in ids.iter().enumerate() {
        let from = ed.pages.iter().position(|p| &p.id == id).unwrap();
        ed.move_page(from, to);
    }
    text(json!({"pages": page_list(ed)}))
}

/// The listed layers without those inside another listed layer, in input order.
fn top_level(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut roots: Vec<NodeId> = Vec::new();
    for id in ids {
        if !roots.contains(id) && !ids.iter().any(|o| o != id && doc.is_ancestor(*o, *id)) {
            roots.push(*id);
        }
    }
    roots
}

fn move_to_page(ed: &mut Editor, a: &Args) -> ToolResult {
    let def = page_arg(ed, a)?;
    let mut src = None;
    let mut ids = Vec::new();
    for k in a.strs("nodeIds")? {
        let (p, id) = resolve(ed, &k, def)?;
        if src.is_some_and(|q| q != p) {
            return Err("all nodeIds must be on one page".into());
        }
        src = Some(p);
        ids.push(id);
    }
    let src = src.ok_or("nodeIds is empty")?;
    let dst = find_page(ed, a.req_str("toPageId")?)?;
    if dst == src {
        return Err(
            "the layers are already on that page; use move_nodes or duplicate_nodes".into(),
        );
    }
    let parent = match a.str("parentId").filter(|p| !p.is_empty() && *p != ROOT) {
        None => None,
        Some(p) => {
            let (pp, pid) = resolve(ed, p, dst)?;
            if pp != dst {
                return Err("parentId must be on the destination page".into());
            }
            if !ed.page_doc(dst).get(pid).unwrap().is_container() {
                return Err("parentId must be a frame or group".into());
            }
            Some(pid)
        }
    };
    let copy = a.get("copy").and_then(Value::as_bool) == Some(true);

    let doc = ed.page_doc(src);
    let roots = top_level(doc, &ids);
    let old_order: Vec<Vec<NodeId>> = roots.iter().map(|r| doc.subtree(*r)).collect();
    let nodes: Vec<Node> = old_order
        .iter()
        .flatten()
        .filter_map(|id| doc.get(*id).cloned())
        .collect();
    let world: Vec<Rect> = match parent {
        // Same x/y inside the new parent as inside the old one.
        Some(p) => {
            let origin = ed.page_doc(dst).world_rect(p).unwrap().min();
            (roots.iter())
                .map(|r| doc.get(*r).unwrap().rect.translate(origin))
                .collect()
        }
        None => (roots.iter())
            .map(|r| doc.world_rect(*r).unwrap_or_default())
            .collect(),
    };
    let src_id = ed.pages[src].id.clone();
    let dst_id = ed.pages[dst].id.clone();
    let label = if copy { "Copy to page" } else { "Move to page" };

    // The destination goes first, keeping moved mains as mains. Each page's own links to a
    // moved main change in the same undo step as the move on that page.
    let mut pairs: Vec<(NodeId, NodeId)> = Vec::new();
    let mut moved: HashMap<NodeId, NodeId> = HashMap::new();
    let mut relinked = 0;
    let new_roots = ed.with_page(dst, |ed| {
        ed.insert_subtrees_with(
            label,
            &nodes,
            &roots,
            &world,
            V2::ZERO,
            Some(&src_id),
            !copy,
            |tx, new| {
                if let Some(p) = parent {
                    for id in new {
                        tx.reparent(*id, Some(p), None);
                    }
                    relayout(tx, Some(p));
                }
                for (old, new) in old_order.iter().zip(new) {
                    pairs.extend(old.iter().copied().zip(tx.doc().subtree(*new)));
                }
                if !copy {
                    moved = moved_mains(tx.doc(), &pairs);
                    relinked += relink(tx, &dst_id, &src_id, &dst_id, &moved);
                }
            },
        )
    });
    if !copy {
        ed.with_page(src, |ed| {
            ed.edit(label, |tx| {
                relinked += relink(tx, &src_id, &src_id, &dst_id, &moved);
                for r in &roots {
                    tx.remove(*r);
                }
            });
            ed.selection.retain(|s| ed.doc.get(*s).is_some());
        });
        for i in 0..ed.pages.len() {
            let here = ed.pages[i].id.clone();
            if i == src || i == dst || stale(ed.page_doc(i), &here, &src_id, &moved).is_empty() {
                continue;
            }
            ed.with_page(i, |ed| {
                ed.edit("Relink instances", |tx| {
                    relinked += relink(tx, &here, &src_id, &dst_id, &moved);
                })
            });
        }
    }
    let map: serde_json::Map<String, Value> = (pairs.iter())
        .map(|(o, n)| (node_key(ed, src, *o), json!(node_key(ed, dst, *n))))
        .collect();
    let mut v = json!({
        "result": if copy { "copied" } else { "moved" },
        "toPageId": dst_id,
        "newIds": new_roots.iter().map(|n| node_key(ed, dst, *n)).collect::<Vec<_>>(),
        "idMap": map,
    });
    if relinked > 0 {
        v["instancesRelinked"] = json!(relinked);
    }
    text(v)
}

/// Old to new ids of the moved layers that are, or are inside, a main component.
fn moved_mains(doc: &Document, pairs: &[(NodeId, NodeId)]) -> HashMap<NodeId, NodeId> {
    let in_main = |id: NodeId| {
        std::iter::once(id)
            .chain(doc.ancestors(id))
            .any(|a| doc.get(a).is_some_and(|n| n.component))
    };
    pairs.iter().filter(|(_, n)| in_main(*n)).copied().collect()
}

/// Layers on page `here` linked to a main that moved off page `src`.
fn stale(doc: &Document, here: &str, src: &str, moved: &HashMap<NodeId, NodeId>) -> Vec<NodeId> {
    (doc.nodes.values())
        .filter(|n| {
            n.instance.as_ref().is_some_and(|l| {
                l.component.is_none()
                    && moved.contains_key(&l.main)
                    && l.page.as_deref().unwrap_or(here) == src
            })
        })
        .map(|n| n.id)
        .collect()
}

/// Point page `here`'s links to moved mains at their new place on page `dst`. Returns how
/// many layers changed.
fn relink(tx: &mut Tx, here: &str, src: &str, dst: &str, moved: &HashMap<NodeId, NodeId>) -> usize {
    let ids = stale(tx.doc(), here, src, moved);
    for id in &ids {
        let l = tx.node_mut(*id).unwrap().instance.as_mut().unwrap();
        l.main = moved[&l.main];
        l.page = (here != dst).then(|| dst.to_string());
    }
    ids.len()
}
