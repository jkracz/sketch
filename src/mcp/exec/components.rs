//! The file's own components over MCP: make mains, place their instances from write_html,
//! detach instances and reset their overrides. Each tool runs the editor action the inspector
//! runs, on the page the nodes are on, and leaves the user's selection as it was.

use super::{Args, INSTANCE_TAG, ToolResult, node_key, page_arg, resolve, resolve_all, text};
use crate::editor::Editor;
use crate::editor::structure::Source;
use crate::mcp::html::Html;
use crate::model::{Document, Node, NodeId};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// At most this many mains in get_basic_info.
const MAX_LISTED: usize = 200;

/// Run `f` on page `page` with `ids` selected, then give the page its selection back.
fn with_selection<R>(
    ed: &mut Editor,
    page: usize,
    ids: Vec<NodeId>,
    f: impl FnOnce(&mut Editor) -> R,
) -> R {
    ed.with_page(page, |ed| {
        let saved = std::mem::replace(&mut ed.selection, ids);
        let r = f(ed);
        ed.selection = saved;
        let doc = &ed.doc;
        ed.selection.retain(|s| doc.get(*s).is_some());
        r
    })
}

/// Group node keys by the page they're on, in the order given.
fn by_page(ed: &Editor, keys: &[String], def: usize) -> Result<Vec<(usize, Vec<NodeId>)>, String> {
    let mut out: Vec<(usize, Vec<NodeId>)> = Vec::new();
    for k in keys {
        let (p, id) = resolve(ed, k, def)?;
        match out.iter_mut().find(|(q, _)| *q == p) {
            Some((_, l)) if !l.contains(&id) => l.push(id),
            Some(_) => {}
            None => out.push((p, vec![id])),
        }
    }
    Ok(out)
}

/// The page and node of the main a link points at, when that node is on a page of this file.
fn linked_node(ed: &Editor, page: usize, n: &Node) -> Option<(usize, NodeId)> {
    let l = n.instance.as_ref()?;
    if l.component.is_some() {
        return None;
    }
    let mp = match &l.page {
        None => page,
        Some(id) => ed.pages.iter().position(|p| p.id == *id)?,
    };
    ed.page_doc(mp).get(l.main).map(|m| (mp, m.id))
}

/// Whether `n` is the root of an instance: its link points at a main component, here or in a
/// design system.
fn is_root(ed: &Editor, page: usize, n: &Node) -> bool {
    let Some(l) = &n.instance else { return false };
    match &l.component {
        Some(c) => {
            (ed.tokens.component(c)).is_some_and(|c| c.doc.get(l.main).is_some_and(|m| m.component))
        }
        None => linked_node(ed, page, n)
            .is_some_and(|(mp, m)| ed.page_doc(mp).get(m).is_some_and(|m| m.component)),
    }
}

/// The instance root `id` is a layer of: itself or its nearest ancestor that is one.
fn root_of(ed: &Editor, page: usize, id: NodeId) -> Option<NodeId> {
    let doc = ed.page_doc(page);
    std::iter::once(id)
        .chain(doc.ancestors(id))
        .find(|a| doc.get(*a).is_some_and(|n| is_root(ed, page, n)))
}

// ------------------------------------------------------------------------------ reading

/// Component facts for get_node_info: what an instance comes from, the instance a layer
/// belongs to, its overrides, and how many instances a main has.
pub(super) fn describe(ed: &Editor, page: usize, id: NodeId, v: &mut Value) {
    let doc = ed.page_doc(page);
    let n = doc.get(id).unwrap();
    if n.component {
        v["instanceCount"] = json!(instance_count(ed, page, id));
    }
    if is_root(ed, page, n) {
        if let Some((mp, m)) = linked_node(ed, page, n) {
            v["instanceOf"] = json!(ed.page_doc(mp).get(m).unwrap().name);
            v["mainComponentId"] = json!(node_key(ed, mp, m));
        }
    } else if n.instance.is_some()
        && let Some(r) = root_of(ed, page, id)
    {
        v["instanceRootId"] = json!(node_key(ed, page, r));
    }
    if let Some(l) = n.instance.as_ref().filter(|l| !l.overrides.is_empty()) {
        v["overrides"] = json!(l.overrides);
    }
}

/// The name of the file's own main that instance root `n` comes from.
pub(super) fn main_name(ed: &Editor, page: usize, n: &Node) -> Option<String> {
    let (mp, m) = linked_node(ed, page, n)?;
    let m = ed.page_doc(mp).get(m)?;
    m.component.then(|| m.name.clone())
}

fn instance_count(ed: &Editor, page: usize, main: NodeId) -> usize {
    (0..ed.pages.len())
        .map(|p| {
            (ed.page_doc(p).nodes.values())
                .filter(|n| linked_node(ed, p, n) == Some((page, main)))
                .count()
        })
        .sum()
}

/// Every main component in the file, on every page, for get_basic_info.
pub(super) fn list(ed: &Editor) -> Value {
    let all = mains(ed);
    let listed: Vec<Value> = (all.iter().take(MAX_LISTED))
        .map(|(p, id)| {
            let n = ed.page_doc(*p).get(*id).unwrap();
            json!({
                "id": node_key(ed, *p, *id), "name": n.name, "pageId": ed.pages[*p].id,
                "width": super::r2(n.rect.w), "height": super::r2(n.rect.h),
            })
        })
        .collect();
    if all.len() > MAX_LISTED {
        json!({"total": all.len(), "first": listed})
    } else {
        json!(listed)
    }
}

/// Every main component, page by page, in name order within a page.
fn mains(ed: &Editor) -> Vec<(usize, NodeId)> {
    let mut out = Vec::new();
    for p in 0..ed.pages.len() {
        let doc = ed.page_doc(p);
        let mut here: Vec<&Node> = doc.nodes.values().filter(|n| n.component).collect();
        here.sort_by(|a, b| {
            (a.name.to_lowercase().cmp(&b.name.to_lowercase())).then(a.id.0.cmp(&b.id.0))
        });
        out.extend(here.into_iter().map(|n| (p, n.id)));
    }
    out
}

/// The file's own main named by `key`: a main's node id, or its exact name ignoring case.
/// `Ok(None)` when the file has no such main; an error when several mains share the name.
pub(super) fn find_main(ed: &Editor, key: &str) -> Result<Option<(usize, NodeId)>, String> {
    let key = key.trim();
    if key.contains(':')
        && let Ok((p, id)) = resolve(ed, key, ed.active_page)
        && ed.page_doc(p).get(id).is_some_and(|n| n.component)
    {
        return Ok(Some((p, id)));
    }
    let named: Vec<(usize, NodeId)> = (mains(ed).into_iter())
        .filter(|(p, id)| {
            (ed.page_doc(*p).get(*id)).is_some_and(|n| n.name.trim().eq_ignore_ascii_case(key))
        })
        .collect();
    match named[..] {
        [] => Ok(None),
        [one] => Ok(Some(one)),
        _ => Err(format!(
            "{} components are named `{key}`: {}. Pass the main's id as component=\"…\", or \
             rename one with rename_nodes.",
            named.len(),
            (named.iter())
                .map(|(p, id)| format!("{} on page {}", node_key(ed, *p, *id), ed.pages[*p].name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// ------------------------------------------------------------------------------ write_html

/// The file's own mains that one write_html call places, resolved before the edit starts so
/// mains on other pages can be read while the target page is being edited.
#[derive(Default)]
pub(super) struct LocalMains {
    /// By the `component` attribute as written (trimmed).
    by_key: HashMap<String, (Option<String>, NodeId)>,
    /// Documents of other pages that hold mains, by page id.
    docs: HashMap<String, Document>,
}

impl LocalMains {
    /// Resolve every `<x-sketch-instance component>` in `tree` that names one of the file's
    /// own mains. `target` is where the HTML goes on `page`: a main can't be placed inside
    /// itself, directly or through its instances.
    pub(super) fn resolve(
        ed: &Editor,
        tree: &[Html],
        page: usize,
        target: Option<NodeId>,
    ) -> Result<LocalMains, String> {
        let mut keys = Vec::new();
        wanted(tree, &mut keys);
        let mut out = LocalMains::default();
        if keys.is_empty() {
            return Ok(out);
        }
        let doc = ed.page_doc(page);
        let around: HashSet<(usize, NodeId)> = (target.into_iter())
            .flat_map(|t| std::iter::once(t).chain(doc.ancestors(t)))
            .filter(|a| doc.get(*a).is_some_and(|n| n.component))
            .map(|a| (page, a))
            .collect();
        for key in keys {
            if out.by_key.contains_key(&key) {
                continue;
            }
            let Some((mp, main)) = find_main(ed, &key)? else {
                continue;
            };
            if holds_any(ed, (mp, main), &around, 0) {
                return Err(format!(
                    "{INSTANCE_TAG} `{key}`: a component can't hold an instance of itself. \
                     Place it outside the main component."
                ));
            }
            let page_id = (mp != page).then(|| ed.pages[mp].id.clone());
            if let Some(id) = &page_id {
                (out.docs)
                    .entry(id.clone())
                    .or_insert_with(|| ed.page_doc(mp).clone());
            }
            out.by_key.insert(key, (page_id, main));
        }
        Ok(out)
    }

    /// Where to instantiate the main `key` names from, and the main's id.
    pub(super) fn get(&self, key: &str) -> Option<(Source<'_>, NodeId)> {
        let (page, main) = self.by_key.get(key.trim())?;
        let src = Source {
            doc: page.as_ref().and_then(|p| self.docs.get(p)),
            page: page.as_deref(),
            component: None,
        };
        Some((src, *main))
    }
}

fn wanted(nodes: &[Html], out: &mut Vec<String>) {
    for n in nodes {
        if let Html::Element(el) = n {
            match el.attr("component") {
                Some(c) if el.tag == INSTANCE_TAG => out.push(c.trim().to_string()),
                _ => wanted(&el.children, out),
            }
        }
    }
}

/// Whether main `m` is one of `set`, or holds an instance of one of them at any depth.
fn holds_any(
    ed: &Editor,
    m: (usize, NodeId),
    set: &HashSet<(usize, NodeId)>,
    depth: usize,
) -> bool {
    if set.contains(&m) {
        return true;
    }
    if depth > 16 {
        return false;
    }
    let doc = ed.page_doc(m.0);
    (doc.subtree(m.1).into_iter()).any(|id| {
        let n = doc.get(id).unwrap();
        id != m.1
            && is_root(ed, m.0, n)
            && linked_node(ed, m.0, n).is_some_and(|inner| holds_any(ed, inner, set, depth + 1))
    })
}

// ------------------------------------------------------------------------------ tools

pub(super) fn create_component(ed: &mut Editor, a: &Args) -> ToolResult {
    let (page, ids) = resolve_all(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    if ids.is_empty() {
        return Err("nodeIds is empty: pass the layers to make into a component".into());
    }
    for id in &ids {
        let n = ed.page_doc(page).get(*id).unwrap();
        if n.instance.is_some() && !is_root(ed, page, n) {
            let root = root_of(ed, page, *id).map_or(String::new(), |r| node_key(ed, page, r));
            return Err(format!(
                "`{}` is a layer inside instance {root}. Detach the instance with \
                 detach_instances first, or make a component from the whole instance.",
                node_key(ed, page, *id)
            ));
        }
    }
    let already = match ids[..] {
        [id] => ed.page_doc(page).get(id).is_some_and(|n| n.component),
        _ => false,
    };
    let name = a.str("name");
    let made = with_selection(ed, page, ids.clone(), |ed| {
        let made = ed.create_component_named(name);
        (made, ed.status.clone())
    });
    let id = match made {
        (Some(id), _) => id,
        (None, status) if !status.is_empty() => return Err(status),
        (None, _) => return Err("those layers can't make a component".into()),
    };
    let n = ed.page_doc(page).get(id).unwrap();
    let mut v = json!({
        "componentId": node_key(ed, page, id),
        "name": n.name,
        "pageId": ed.pages[page].id,
        "wrapped": !ids.contains(&id),
    });
    if already {
        v["alreadyComponent"] = json!(true);
    }
    v["next"] = json!(format!(
        "Place instances with <{INSTANCE_TAG} component=\"{}\"/> in write_html.",
        n.name.replace('"', "'")
    ));
    text(v)
}

pub(super) fn detach_instances(ed: &mut Editor, a: &Args) -> ToolResult {
    let groups = by_page(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    let mut detached = Vec::new();
    let mut skipped = Vec::new();
    for (page, ids) in groups {
        let mut roots = Vec::new();
        for id in ids {
            let n = ed.page_doc(page).get(id).unwrap();
            if is_root(ed, page, n) {
                roots.push(id);
                continue;
            }
            let reason = match root_of(ed, page, id) {
                Some(r) => format!(
                    "a layer inside instance {}; detach that instead",
                    node_key(ed, page, r)
                ),
                None => "not an instance".into(),
            };
            skipped.push(json!({"id": node_key(ed, page, id), "reason": reason}));
        }
        if roots.is_empty() {
            continue;
        }
        with_selection(ed, page, roots.clone(), Editor::detach_instance);
        detached.extend(roots.iter().map(|r| node_key(ed, page, *r)));
    }
    if detached.is_empty() {
        return Err(format!(
            "nothing detached: {}",
            serde_json::to_string(&skipped).unwrap_or_default()
        ));
    }
    let mut v = json!({"detached": detached});
    if !skipped.is_empty() {
        v["skipped"] = json!(skipped);
    }
    text(v)
}

pub(super) fn reset_overrides(ed: &mut Editor, a: &Args) -> ToolResult {
    let groups = by_page(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    let mut reset = Vec::new();
    let mut skipped = Vec::new();
    for (page, ids) in groups {
        let mut todo = Vec::new();
        for id in ids {
            let doc = ed.page_doc(page);
            if root_of(ed, page, id).is_none() {
                skipped.push(json!({"id": node_key(ed, page, id), "reason": "not in an instance"}));
                continue;
            }
            let mut cleared: Vec<String> = Vec::new();
            for s in doc.subtree(id) {
                for o in (doc.get(s).and_then(|n| n.instance.as_ref()))
                    .map_or(&[][..], |l| &l.overrides[..])
                {
                    if !cleared.contains(o) {
                        cleared.push(o.clone());
                    }
                }
            }
            reset.push(json!({"id": node_key(ed, page, id), "cleared": cleared}));
            todo.push(id);
        }
        if !todo.is_empty() {
            with_selection(ed, page, todo, Editor::reset_overrides);
        }
    }
    if reset.is_empty() {
        return Err(format!(
            "nothing reset: {}",
            serde_json::to_string(&skipped).unwrap_or_default()
        ));
    }
    let mut v = json!({"reset": reset});
    if !skipped.is_empty() {
        v["skipped"] = json!(skipped);
    }
    text(v)
}
