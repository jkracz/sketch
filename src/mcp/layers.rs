//! Layer commands: lock, group, ungroup, flip, align and distribute. Each runs the editor
//! command behind the inspector and shortcuts on the given nodes instead of the selection.

use super::ToolResult;
use super::exec::{Args, node_key, page_arg, resolve_all, text};
use super::tools::tool;
use crate::editor::Editor;
use crate::geom::Rect;
use crate::model::NodeId;
use serde_json::{Value, json};

pub const TOOLS: [&str; 6] = [
    "lock_nodes",
    "group_nodes",
    "ungroup_nodes",
    "flip_nodes",
    "align_nodes",
    "distribute_nodes",
];

pub fn definitions() -> Vec<Value> {
    let ids = |d: &str| json!({"type": "array", "minItems": 1, "items": {"type": "string"}, "description": d});
    let direction =
        |d: &str| json!({"type": "string", "enum": ["horizontal", "vertical"], "description": d});
    vec![
        tool(
            "lock_nodes",
            "Lock or unlock layers. Locked layers can't be selected or dragged on the canvas, and \
             flip_nodes and align_nodes leave them in place. get_node_info shows `locked`.",
            json!({
                "nodeIds": ids("Layers to lock or unlock."),
                "locked": {"type": "boolean", "description": "false unlocks. Defaults to true."},
            }),
            &["nodeIds"],
            false,
        ),
        tool(
            "group_nodes",
            "Group layers that share a parent, like Cmd+G. The group goes where the topmost layer \
             was, keeps every layer where it is on the canvas, and is named \"Group\" (rename it \
             with rename_nodes). Returns groupId.",
            json!({"nodeIds": ids("Layers to group. They must have the same parent.")}),
            &["nodeIds"],
            false,
        ),
        tool(
            "ungroup_nodes",
            "Ungroup groups or frames, like Cmd+Shift+G: their children move to the parent in \
             their place and keep their positions, and the group or frame is deleted, with its \
             fill, stroke and auto layout. Returns the freed childIds.",
            json!({"nodeIds": ids("Groups or frames with children.")}),
            &["nodeIds"],
            false,
        ),
        tool(
            "flip_nodes",
            "Mirror layers horizontally or vertically, like Shift+H and Shift+V. Several layers \
             flip as one, within their combined bounds; one layer flips in place. Vectors, \
             lines, arrows and drawings mirror their shape and children of frames and groups swap \
             sides. Rectangles, ellipses, polygons, stars, text and images keep their shape \
             and only move. Locked layers stay put.",
            json!({
                "nodeIds": ids("Layers to flip."),
                "direction": direction("\"horizontal\" mirrors left and right, \"vertical\" top and bottom."),
            }),
            &["nodeIds", "direction"],
            false,
        ),
        tool(
            "align_nodes",
            "Align layers like the inspector's align buttons. Several layers align to their \
             combined bounds; one layer aligns within its parent frame (center it in its \
             artboard with horizontal and vertical \"center\"). Pass horizontal, vertical or \
             both. Locked layers stay put but count toward the bounds. Children of auto layout \
             frames are placed by the layout, so align those with justifyContent and alignItems \
             instead.",
            json!({
                "nodeIds": ids("Layers to align."),
                "horizontal": {"type": "string", "enum": ["left", "center", "right"]},
                "vertical": {"type": "string", "enum": ["top", "center", "bottom"]},
            }),
            &["nodeIds"],
            false,
        ),
        tool(
            "distribute_nodes",
            "Space three or more layers evenly, like the inspector's distribute buttons: the \
             first and last layers along the direction stay put and the ones between move so \
             the gaps are equal.",
            json!({
                "nodeIds": ids("At least three layers."),
                "direction": direction("\"horizontal\" equalizes the gaps along x, \"vertical\" along y."),
            }),
            &["nodeIds", "direction"],
            false,
        ),
    ]
}

pub fn call(ed: &mut Editor, name: &str, a: &Args) -> ToolResult {
    let (page, ids) = resolve_all(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    if ids.is_empty() {
        return Err("nodeIds is empty".into());
    }
    match name {
        "lock_nodes" => {
            let locked = match a.get("locked") {
                None => true,
                Some(v) => v.as_bool().ok_or("`locked` must be true or false")?,
            };
            ed.with_page(page, |ed| {
                ed.edit(if locked { "Lock" } else { "Unlock" }, |tx| {
                    for id in &ids {
                        tx.node_mut(*id).unwrap().locked = locked;
                    }
                })
            });
            text(json!({"updated": ids.len(), "locked": locked}))
        }
        "group_nodes" => {
            let doc = ed.page_doc(page);
            let parent = doc.get(ids[0]).unwrap().parent;
            if ids.iter().any(|id| doc.get(*id).unwrap().parent != parent) {
                return Err(
                    "the layers must have the same parent; move_nodes can gather them".into(),
                );
            }
            let (_, left) = on_nodes(ed, page, ids.clone(), Editor::group_selection);
            let doc = ed.page_doc(page);
            let group = match left[..] {
                [g] if !ids.contains(&g) && doc.get(g).is_some_and(|n| n.is_group()) => g,
                _ => return Err("the layers could not be grouped".into()),
            };
            text(json!({
                "groupId": node_key(ed, page, group),
                "childIds": keys(ed, page, &doc.get(group).unwrap().children),
            }))
        }
        "ungroup_nodes" => {
            let doc = ed.page_doc(page);
            if let Some(id) = ids.iter().find(|id| {
                let n = doc.get(**id).unwrap();
                !n.is_container() || n.children.is_empty()
            }) {
                return Err(format!(
                    "{} is not a group or frame with children",
                    node_key(ed, page, *id)
                ));
            }
            let (_, freed) = on_nodes(ed, page, ids, Editor::ungroup_selection);
            text(json!({"childIds": keys(ed, page, &freed)}))
        }
        "flip_nodes" => {
            let axis = axis(a)?;
            let (_, locked) = moved(ed, page, ids.clone(), |ed| ed.flip(axis));
            // A layer flipped in place keeps its box, so report every unlocked one.
            let flipped: Vec<NodeId> = ids.into_iter().filter(|id| !locked.contains(id)).collect();
            report(ed, page, "flipped", &flipped, &locked)
        }
        "align_nodes" => {
            let h = match a.str("horizontal") {
                None => None,
                Some("left") => Some(0.0),
                Some("center") => Some(0.5),
                Some("right") => Some(1.0),
                Some(v) => return Err(format!("horizontal is left, center or right, not `{v}`")),
            };
            let v = match a.str("vertical") {
                None => None,
                Some("top") => Some(0.0),
                Some("center") => Some(0.5),
                Some("bottom") => Some(1.0),
                Some(v) => return Err(format!("vertical is top, center or bottom, not `{v}`")),
            };
            if h.is_none() && v.is_none() {
                return Err("pass horizontal, vertical or both".into());
            }
            if let [id] = ids[..]
                && ed.page_doc(page).get(id).unwrap().parent.is_none()
            {
                return Err(
                    "one layer aligns within its parent frame, and this one is on the page; \
                     pass several layers to align them to each other"
                        .into(),
                );
            }
            let (moved, locked) = moved(ed, page, ids, |ed| {
                // Both axes as one undo step. A move along x leaves y unchanged.
                let mut moves = h.map_or(Vec::new(), |to| ed.align_moves(0, to));
                moves.extend(v.map_or(Vec::new(), |to| ed.align_moves(1, to)));
                ed.edit("Align", |tx| {
                    for (id, d) in moves {
                        let n = tx.node_mut(id).unwrap();
                        n.rect = n.rect.translate(d);
                    }
                });
            });
            report(ed, page, "moved", &moved, &locked)
        }
        "distribute_nodes" => {
            let axis = axis(a)?;
            if ids.len() < 3 {
                return Err("distributing needs at least three layers".into());
            }
            let (moved, _) = moved(ed, page, ids, |ed| ed.distribute(axis));
            report(ed, page, "moved", &moved, &[])
        }
        _ => Err(format!("unknown tool `{name}`")),
    }
}

fn axis(a: &Args) -> Result<usize, String> {
    match a.str("direction") {
        Some("horizontal") => Ok(0),
        Some("vertical") => Ok(1),
        _ => Err("direction is \"horizontal\" or \"vertical\"".into()),
    }
}

fn keys(ed: &Editor, page: usize, ids: &[NodeId]) -> Vec<String> {
    ids.iter().map(|id| node_key(ed, page, *id)).collect()
}

/// Run a selection command on `ids` as if the user had selected them, then give the user's
/// selection back. Returns the command's result and the selection it left.
fn on_nodes<R>(
    ed: &mut Editor,
    page: usize,
    ids: Vec<NodeId>,
    f: impl FnOnce(&mut Editor) -> R,
) -> (R, Vec<NodeId>) {
    ed.with_page(page, |ed| {
        let users = std::mem::replace(&mut ed.selection, ids);
        let r = f(ed);
        let left = std::mem::replace(&mut ed.selection, users);
        let doc = &ed.doc;
        ed.selection.retain(|id| doc.nodes.contains_key(id));
        (r, left)
    })
}

/// Run a command that moves layers. Returns the layers whose box changed and the locked
/// ones, which the command leaves alone.
fn moved(
    ed: &mut Editor,
    page: usize,
    ids: Vec<NodeId>,
    f: impl FnOnce(&mut Editor),
) -> (Vec<NodeId>, Vec<NodeId>) {
    let rects = |ed: &Editor| -> Vec<Option<Rect>> {
        let doc = ed.page_doc(page);
        ids.iter().map(|id| doc.get(*id).map(|n| n.rect)).collect()
    };
    let before = rects(ed);
    on_nodes(ed, page, ids.clone(), f);
    let after = rects(ed);
    let doc = ed.page_doc(page);
    let moved = (ids.iter().zip(before.iter().zip(&after)))
        .filter(|(_, (b, a))| b != a)
        .map(|(id, _)| *id)
        .collect();
    let locked = (ids.iter().copied())
        .filter(|id| doc.is_effectively_locked(*id))
        .collect();
    (moved, locked)
}

fn report(ed: &Editor, page: usize, key: &str, done: &[NodeId], locked: &[NodeId]) -> ToolResult {
    let mut v = json!({key: keys(ed, page, done)});
    if !locked.is_empty() {
        v["locked"] = json!(keys(ed, page, locked));
    }
    text(v)
}
