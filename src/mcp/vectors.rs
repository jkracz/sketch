//! Vector, text-range, arrow and pencil tools: booleans, flatten, path points, styled text
//! ranges, arrow routes and bindings after creation, and freehand strokes.
//!
//! Booleans and flatten run the editor's own commands on a borrowed selection, so agents get
//! exactly what the toolbar does. Points are world coordinates throughout, because a path's
//! box refits after every edit and node-local numbers would shift under the agent.

use super::exec::{
    Args, ROOT, fit_text, head, mark_override, page_arg, r2, relayout, resolve, resolve_all, text,
};
use super::tools::{page_id, styles_schema, tool};
use super::{ToolResult, css};
use crate::editor::Editor;
use crate::editor::vector_ops::{BoolOp, normalize_path};
use crate::editor::whiteboard::{set_world_points, update_connector};
use crate::geom::{V2, v2};
use crate::history::Tx;
use crate::model::{
    Binding, Document, FillRule, Head, Node, NodeId, NodeKind, PathPoint, Route, StrokePoint,
    SubPath, TextSpan, TextStyle,
};
use serde_json::{Map, Value, json};

/// The tools this module serves, for the catalogue.
pub fn definitions() -> Vec<Value> {
    let ids = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    let point = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2});
    vec![
        tool(
            "boolean_nodes",
            "Combines two or more layers into one vector layer, like the toolbar's boolean \
             buttons: union, subtract (the bottom layer minus the ones above it), intersect or \
             exclude. Layers combine in stacking order, bottom first. The result takes the top \
             layer's style (the bottom one's for subtract), sits where the top layer was and \
             replaces the operands. It is destructive: curves become many short segments. \
             Returns the new layer's id, or null when nothing is left (an intersect of layers \
             that don't overlap).",
            json!({
                "nodeIds": ids("Two or more layers on one page."),
                "operation": {"type": "string", "enum": ["union", "subtract", "intersect", "exclude"]},
                "pageId": page_id(),
            }),
            &["nodeIds", "operation"],
            false,
        ),
        tool(
            "flatten_nodes",
            "Converts shapes, lines and text to editable vector paths in place (the Flatten \
             command, ⌘E). Ids stay the same. Frames, groups and layers that are already \
             paths are skipped and listed under `skipped`. Read the points with get_node_info \
             and edit them with set_vector_points.",
            json!({"nodeIds": ids("Layers to flatten."), "pageId": page_id()}),
            &["nodeIds"],
            false,
        ),
        tool(
            "set_vector_points",
            "Replaces the points of a vector path, which is how you move, add or delete \
             points and handles (like Edit points in the app). get_node_info returns a path's \
             `vector` in the same shape: copy it, change it, send it back. Coordinates are \
             world units. Give `subpaths` ([{closed, points: [{x, y, in?: [x, y], out?: [x, \
             y]}]}], where in/out are bezier handles and a point without them is a corner) or \
             an SVG path `d` string in world units. Each subpath needs two or more points. \
             The layer's box refits to the points. Flatten other shapes with flatten_nodes \
             first.",
            json!({
                "nodeId": {"type": "string", "description": "A vector path layer."},
                "subpaths": {"type": "array", "minItems": 1, "items": {
                    "type": "object",
                    "properties": {
                        "closed": {"type": "boolean"},
                        "points": {"type": "array", "minItems": 2, "items": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "number"}, "y": {"type": "number"},
                                "in": point.clone(), "out": point.clone(),
                            },
                            "required": ["x", "y"],
                        }},
                    },
                    "required": ["points"],
                }},
                "d": {"type": "string", "description": "SVG path data in world units, instead of subpaths, e.g. \"M0 0 L100 0 C150 0 150 100 100 100 Z\"."},
                "fillRule": {"type": "string", "enum": ["nonzero", "evenodd"]},
                "pageId": page_id(),
            }),
            &["nodeId"],
            false,
        ),
        tool(
            "set_text_range_styles",
            "Styles part of a text or sticky layer, like selecting characters in the app and \
             changing them. Pick the range with `match` (text to find, case-sensitive; \
             `occurrence` picks which one, from 1, or \"all\") or with `start`/`end` character \
             offsets (end exclusive). With neither, the whole text. `styles` takes fontWeight, \
             fontStyle (normal, italic), fontFamily, fontSize (px), color and textDecoration \
             (underline, line-through, none). `reset: true` removes range styling so the \
             characters follow the layer again (applied before `styles`). get_node_info lists \
             a layer's styled ranges as `textRanges`. Returns each layer's ranges after the \
             change.",
            json!({"updates": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "nodeId": {"type": "string"},
                    "match": {"type": "string", "description": "Text to find in the layer."},
                    "occurrence": {"anyOf": [{"type": "integer", "minimum": 1}, {"type": "string", "enum": ["all"]}], "description": "Which match to style, from 1 (default), or \"all\"."},
                    "start": {"type": "integer", "minimum": 0, "description": "First character, counting Unicode characters from 0."},
                    "end": {"type": "integer", "minimum": 0, "description": "One past the last character."},
                    "styles": styles_schema("fontWeight, fontStyle, fontFamily, fontSize, color, textDecoration."),
                    "reset": {"type": "boolean", "description": "Clear range styles first."},
                },
                "required": ["nodeId"],
            }}, "pageId": page_id()}),
            &["updates"],
            false,
        ),
        tool(
            "update_arrows",
            "Edits arrows and connector lines after they exist: what each end is bound to, \
             their points, route and heads. startNodeId/endNodeId bind an end to a layer (the \
             arrow follows it when it moves); an empty string or null unbinds it, leaving the \
             end where it is. startAnchor/endAnchor say where on the layer the end aims, as \
             [fx, fy] fractions of its box (default [0.5, 0.5], the center); the end stops at \
             the layer's outline. `points` replaces the route in world units ([[x,y],...], two \
             or more; middle points bend a curved arrow, and bound ends are recomputed). route \
             is straight, curved or elbow (elbow routes around the bound shapes by itself; \
             straight keeps only the two ends). Heads: none, arrow, triangle, dot, bar, \
             diamond. get_node_info shows an arrow's `arrow` details.",
            json!({"updates": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "nodeId": {"type": "string", "description": "An arrow or line made with create_shape or the arrow tool."},
                    "startNodeId": {"type": ["string", "null"]},
                    "endNodeId": {"type": ["string", "null"]},
                    "startAnchor": point.clone(),
                    "endAnchor": point.clone(),
                    "points": {"type": "array", "minItems": 2, "items": point.clone()},
                    "route": {"type": "string", "enum": ["straight", "curved", "elbow"]},
                    "startHead": {"type": "string", "enum": ["none", "arrow", "triangle", "dot", "bar", "diamond"]},
                    "endHead": {"type": "string", "enum": ["none", "arrow", "triangle", "dot", "bar", "diamond"]},
                },
                "required": ["nodeId"],
            }}, "pageId": page_id()}),
            &["updates"],
            false,
        ),
        tool(
            "create_stroke",
            "Draws a freehand pencil stroke, like the pencil tool (⇧P). `points` are world \
             units ([[x, y], ...] or [[x, y, pressure], ...] with pressure 0 to 1, default \
             0.5), two or more. The stroke is smoothed through the points unless smooth is \
             false. width is the line width at pressure 0.5 (default 3), thinning how much \
             pressure changes it (0 even, default 0.6), color any CSS color (default ink that \
             reads on what's underneath). Recolor later with update_styles {color}; remove \
             with delete_nodes, as the eraser does.",
            json!({
                "points": {"type": "array", "minItems": 2, "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 3}},
                "width": {"type": "number", "exclusiveMinimum": 0},
                "thinning": {"type": "number", "minimum": 0, "maximum": 1},
                "color": {"type": "string"},
                "smooth": {"type": "boolean"},
                "name": {"type": "string"},
                "parentId": {"type": "string", "description": "Frame to draw in; points stay world units."},
                "pageId": page_id(),
            }),
            &["points"],
            false,
        ),
    ]
}

/// Run one of this module's tools, or None when `name` isn't one.
pub(super) fn call(ed: &mut Editor, name: &str, a: &Args) -> Option<ToolResult> {
    Some(match name {
        "boolean_nodes" => boolean_nodes(ed, a),
        "flatten_nodes" => flatten_nodes(ed, a),
        "set_vector_points" => set_vector_points(ed, a),
        "set_text_range_styles" => set_text_range_styles(ed, a),
        "update_arrows" => update_arrows(ed, a),
        "create_stroke" => create_stroke(ed, a),
        _ => return None,
    })
}

/// Details get_node_info adds for paths, arrows, strokes and styled text.
pub(super) fn node_details(ed: &Editor, page: usize, n: &Node, out: &mut Value) {
    let doc = ed.page_doc(page);
    let Some(wr) = doc.world_rect(n.id) else {
        return;
    };
    let o = wr.min();
    let pt = |p: V2| json!([r2(p.x), r2(p.y)]);
    match &n.kind {
        NodeKind::Path {
            subpaths,
            fill_rule,
        } => {
            out["vector"] = json!({
                "fillRule": fill_rule_name(*fill_rule),
                "subpaths": subpaths.iter().map(|sp| json!({
                    "closed": sp.closed,
                    "points": sp.points.iter().map(|q| {
                        let mut v = json!({"x": r2(q.p.x + o.x), "y": r2(q.p.y + o.y)});
                        if let Some(h) = q.h_in {
                            v["in"] = pt(h + o);
                        }
                        if let Some(h) = q.h_out {
                            v["out"] = pt(h + o);
                        }
                        v
                    }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
        }
        NodeKind::Connector {
            points,
            start,
            end,
            route,
            start_head,
            end_head,
        } => {
            let bound = |b: &Option<Binding>| match b {
                Some(b) if doc.get(b.node).is_some() => json!({
                    "nodeId": super::exec::node_key(ed, page, b.node),
                    "anchor": [r2(b.fixed.x), r2(b.fixed.y)],
                }),
                _ => Value::Null,
            };
            out["arrow"] = json!({
                "route": route_name(*route),
                "startHead": head_name(*start_head),
                "endHead": head_name(*end_head),
                "start": bound(start),
                "end": bound(end),
                "points": points.iter().map(|p| pt(*p + o)).collect::<Vec<_>>(),
            });
        }
        NodeKind::Stroke {
            points,
            width,
            thinning,
        } => {
            out["stroke"] = json!({
                "width": r2(*width),
                "thinning": r2(*thinning),
                "pointCount": points.len(),
            });
            if let Some(c) = n.style.fills.last() {
                out["stroke"]["color"] = json!(css::color_css(c.preview_color()));
            }
        }
        _ => {}
    }
    if let Some(t) = n.kind.text()
        && !n.text_style.spans.is_empty()
    {
        out["textRanges"] = json!(text_ranges(t, &n.text_style));
    }
}

fn fill_rule_name(r: FillRule) -> &'static str {
    match r {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

fn route_name(r: Route) -> &'static str {
    match r {
        Route::Straight => "straight",
        Route::Curved => "curved",
        Route::Elbow => "elbow",
    }
}

fn head_name(h: Head) -> String {
    h.label().to_ascii_lowercase()
}

// ------------------------------------------------------------------------------ booleans

/// Ids with any id nested inside another one in the list dropped (like `top_selection`).
fn outermost(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = Vec::new();
    for id in ids {
        if !out.contains(id) && !doc.ancestors(*id).iter().any(|a| ids.contains(a)) {
            out.push(*id);
        }
    }
    out
}

/// Run an editor command on `ids` as if the user had selected them, then give the user's
/// selection back. Returns the selection the command left.
fn on_selection(ed: &mut Editor, ids: Vec<NodeId>, f: impl FnOnce(&mut Editor)) -> Vec<NodeId> {
    let keep = std::mem::replace(&mut ed.selection, ids);
    f(ed);
    let after = std::mem::take(&mut ed.selection);
    ed.selection = keep
        .into_iter()
        .filter(|i| ed.doc.nodes.contains_key(i))
        .collect();
    after
}

fn boolean_nodes(ed: &mut Editor, a: &Args) -> ToolResult {
    let op = match a.req_str("operation")?.to_ascii_lowercase().as_str() {
        "union" => BoolOp::Union,
        "subtract" => BoolOp::Subtract,
        "intersect" => BoolOp::Intersect,
        "exclude" => BoolOp::Exclude,
        o => {
            return Err(format!(
                "unknown operation `{o}`; use union, subtract, intersect or exclude"
            ));
        }
    };
    let (page, ids) = resolve_all(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    let ids = outermost(ed.page_doc(page), &ids);
    if ids.len() < 2 {
        return Err("boolean_nodes needs two or more layers, none inside another".into());
    }
    let removed = ids.len();
    let out = ed.with_page(page, |ed| on_selection(ed, ids, |ed| ed.boolean(op)));
    let id = out.first().map(|id| super::exec::node_key(ed, page, *id));
    text(json!({
        "id": id,
        "operation": op.label().to_ascii_lowercase(),
        "replaced": removed,
        "pageId": ed.pages[page].id,
    }))
}

fn flatten_nodes(ed: &mut Editor, a: &Args) -> ToolResult {
    let (page, ids) = resolve_all(ed, &a.strs("nodeIds")?, page_arg(ed, a)?)?;
    let doc = ed.page_doc(page);
    let ids = outermost(doc, &ids);
    let (todo, skipped): (Vec<NodeId>, Vec<NodeId>) = ids.into_iter().partition(|id| {
        doc.get(*id)
            .is_some_and(|n| !n.is_container() && !matches!(n.kind, NodeKind::Path { .. }))
    });
    if !todo.is_empty() {
        ed.with_page(page, |ed| {
            on_selection(ed, todo.clone(), |ed| ed.flatten_selection())
        });
    }
    let key = |id: &NodeId| super::exec::node_key(ed, page, *id);
    text(json!({
        "flattened": todo.iter().map(key).collect::<Vec<_>>(),
        "skipped": skipped.iter().map(key).collect::<Vec<_>>(),
    }))
}

// ------------------------------------------------------------------------------ points

fn xy(v: &Value) -> Option<V2> {
    let p = v.as_array()?;
    Some(v2(p.first()?.as_f64()?, p.get(1)?.as_f64()?))
}

/// World-space subpaths from the `subpaths` argument.
fn subpaths_arg(v: &[Value]) -> Result<Vec<SubPath>, String> {
    let mut out = Vec::new();
    for (i, sp) in v.iter().enumerate() {
        let pts = sp
            .get("points")
            .and_then(Value::as_array)
            .ok_or(format!("subpaths[{i}] needs `points`"))?;
        let mut points = Vec::new();
        for (j, p) in pts.iter().enumerate() {
            let at = |k: &str| p.get(k).and_then(Value::as_f64);
            let (Some(x), Some(y)) = (at("x"), at("y")) else {
                return Err(format!("subpaths[{i}].points[{j}] needs numbers x and y"));
            };
            let handle = |k: &str| -> Result<Option<V2>, String> {
                match p.get(k).filter(|h| !h.is_null()) {
                    None => Ok(None),
                    Some(h) => xy(h)
                        .map(Some)
                        .ok_or(format!("subpaths[{i}].points[{j}].{k} must be [x, y]")),
                }
            };
            points.push(PathPoint {
                p: v2(x, y),
                h_in: handle("in")?,
                h_out: handle("out")?,
            });
        }
        if points.len() < 2 {
            return Err(format!("subpaths[{i}] needs two or more points"));
        }
        let closed = sp.get("closed").and_then(Value::as_bool).unwrap_or(false);
        out.push(SubPath { points, closed });
    }
    Ok(out)
}

/// World-space subpaths from SVG path data.
fn subpaths_from_d(d: &str) -> Result<Vec<SubPath>, String> {
    let esc = d
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path d="{esc}" fill="black"/></svg>"#
    );
    let nodes =
        crate::io::svg_import::parse(&svg).map_err(|_| format!("`d` is not SVG path data: {d}"))?;
    let mut out = Vec::new();
    for n in nodes {
        if let NodeKind::Path { subpaths, .. } = n.kind {
            let o = n.rect.min();
            for mut sp in subpaths {
                for q in &mut sp.points {
                    q.p = q.p + o;
                    q.h_in = q.h_in.map(|h| h + o);
                    q.h_out = q.h_out.map(|h| h + o);
                }
                out.push(sp);
            }
        }
    }
    out.retain(|sp| sp.points.len() >= 2);
    if out.is_empty() {
        return Err(format!("`d` draws no subpath with two or more points: {d}"));
    }
    Ok(out)
}

fn set_vector_points(ed: &mut Editor, a: &Args) -> ToolResult {
    let (page, id) = resolve(ed, a.req_str("nodeId")?, page_arg(ed, a)?)?;
    let n = ed.page_doc(page).get(id).unwrap();
    if !matches!(n.kind, NodeKind::Path { .. }) {
        return Err(format!(
            "{} is a {}, not a vector path; call flatten_nodes on it first",
            n.name,
            n.kind.type_name()
        ));
    }
    let world = match (a.get("subpaths").and_then(Value::as_array), a.str("d")) {
        (Some(s), _) => subpaths_arg(s)?,
        (None, Some(d)) if !d.is_empty() => subpaths_from_d(d)?,
        _ => return Err("pass `subpaths` or `d`".into()),
    };
    let rule = match a.str("fillRule").map(str::to_ascii_lowercase).as_deref() {
        None => None,
        Some("nonzero") => Some(FillRule::NonZero),
        Some("evenodd") => Some(FillRule::EvenOdd),
        Some(r) => return Err(format!("unknown fillRule `{r}`; use nonzero or evenodd")),
    };
    let points: usize = world.iter().map(|s| s.points.len()).sum();
    ed.with_page(page, |ed| {
        ed.edit("Edit points", |tx| {
            let o = tx.doc().world_rect(id).unwrap().min();
            let node = tx.node_mut(id).unwrap();
            if let NodeKind::Path {
                subpaths,
                fill_rule,
            } = &mut node.kind
            {
                *subpaths = world
                    .into_iter()
                    .map(|mut sp| {
                        for q in &mut sp.points {
                            q.p = q.p - o;
                            q.h_in = q.h_in.map(|h| h - o);
                            q.h_out = q.h_out.map(|h| h - o);
                        }
                        sp
                    })
                    .collect();
                if let Some(r) = rule {
                    *fill_rule = r;
                }
            }
            normalize_path(tx, id);
            relayout(tx, Some(id));
        })
    });
    let r = ed.page_doc(page).world_rect(id).unwrap_or_default();
    text(json!({
        "id": super::exec::node_key(ed, page, id),
        "points": points,
        "worldX": r2(r.x), "worldY": r2(r.y), "width": r2(r.w), "height": r2(r.h),
    }))
}

// ------------------------------------------------------------------------------ text ranges

fn span_styles(sp: &TextSpan) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(f) = &sp.family {
        m.insert("fontFamily".into(), json!(f));
    }
    match (sp.weight, sp.bold) {
        (Some(w), _) => {
            m.insert("fontWeight".into(), json!(w));
        }
        (None, Some(b)) => {
            m.insert("fontWeight".into(), json!(if b { 700 } else { 400 }));
        }
        _ => {}
    }
    if let Some(i) = sp.italic {
        m.insert(
            "fontStyle".into(),
            json!(if i { "italic" } else { "normal" }),
        );
    }
    if let Some(z) = sp.size {
        m.insert("fontSize".into(), json!(format!("{}px", r2(z))));
    }
    if let Some(c) = sp.color {
        m.insert("color".into(), json!(css::color_css(c)));
    }
    let deco: Vec<&str> = [
        (sp.underline == Some(true)).then_some("underline"),
        (sp.strike == Some(true)).then_some("line-through"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !deco.is_empty() {
        m.insert("textDecoration".into(), json!(deco.join(" ")));
    } else if sp.underline == Some(false) || sp.strike == Some(false) {
        m.insert("textDecoration".into(), json!("none"));
    }
    m
}

fn text_ranges(t: &str, ts: &TextStyle) -> Vec<Value> {
    let chars: Vec<char> = t.chars().collect();
    ts.spans
        .iter()
        .map(|sp| {
            let (a, b) = (sp.start.min(chars.len()), sp.end.min(chars.len()));
            json!({
                "start": sp.start,
                "end": sp.end,
                "text": chars[a..b].iter().collect::<String>(),
                "styles": span_styles(sp),
            })
        })
        .collect()
}

/// One parsed range edit: character ranges, what to set, and whether to clear first.
#[derive(Clone)]
struct RangeEdit {
    ranges: Vec<(usize, usize)>,
    span: TextSpan,
    reset: bool,
    /// Instance overrides the edit makes.
    fields: Vec<&'static str>,
}

fn range_span(
    styles: &Value,
    tokens: &crate::model::TokenSet,
) -> Result<(TextSpan, Vec<&'static str>), String> {
    let mut list = css::from_json(styles)?;
    tokens.expand_in(&mut list, None);
    let mut sp = TextSpan::default();
    let mut fields = Vec::new();
    for (k, v) in &list {
        let v = v.trim();
        match k.as_str() {
            "fontWeight" => {
                sp.weight = Some(css::font_weight(v));
                sp.bold = None;
            }
            "fontStyle" => {
                sp.italic =
                    Some(v.eq_ignore_ascii_case("italic") || v.eq_ignore_ascii_case("oblique"))
            }
            "fontFamily" => {
                sp.family = Some(css::font_family(v).ok_or(format!("bad fontFamily `{v}`"))?)
            }
            "fontSize" => {
                sp.size = Some(
                    css::length(v, 16.0)
                        .filter(|z| *z > 0.0)
                        .ok_or(format!("bad fontSize `{v}`; use px"))?,
                )
            }
            "color" => {
                sp.color = Some(css::color(v).ok_or(format!("bad color `{v}`"))?);
                fields.push("fills");
                continue;
            }
            "textDecoration" | "textDecorationLine" => {
                let v = v.to_ascii_lowercase();
                sp.underline = Some(v.contains("underline"));
                sp.strike = Some(v.contains("line-through"));
            }
            other => {
                return Err(format!(
                    "`{other}` can't style a text range; use fontWeight, fontStyle, fontFamily, fontSize, color or textDecoration"
                ));
            }
        }
        fields.push("font");
    }
    fields.dedup();
    Ok((sp, fields))
}

/// Character ranges an update picks in `t`.
fn pick_ranges(u: &Args, t: &str) -> Result<Vec<(usize, usize)>, String> {
    let len = t.chars().count();
    if let Some(m) = u.get("match").and_then(Value::as_str) {
        if m.is_empty() {
            return Err("`match` is empty".into());
        }
        let found: Vec<(usize, usize)> = t
            .match_indices(m)
            .map(|(b, s)| {
                let a = t[..b].chars().count();
                (a, a + s.chars().count())
            })
            .collect();
        if found.is_empty() {
            return Err(format!("`{m}` is not in the text"));
        }
        return match u.get("occurrence") {
            Some(Value::String(s)) if s == "all" => Ok(found),
            None => Ok(vec![found[0]]),
            Some(v) => {
                let i = v
                    .as_u64()
                    .filter(|i| *i >= 1)
                    .ok_or("occurrence is a number from 1, or \"all\"")?;
                found
                    .get(i as usize - 1)
                    .map(|r| vec![*r])
                    .ok_or(format!("`{m}` occurs {} time(s), not {i}", found.len()))
            }
        };
    }
    let start = u.num("start").map_or(0, |s| s.max(0.0) as usize);
    let end = u.num("end").map_or(len, |e| e.max(0.0) as usize);
    if start >= end || end > len {
        return Err(format!(
            "range {start}..{end} is outside the text, which has {len} characters"
        ));
    }
    Ok(vec![(start, end)])
}

/// Remove range styling inside `[a, b)`, splitting spans that cross its edges.
fn clear_range(ts: &mut TextStyle, a: usize, b: usize) {
    let mut out = Vec::new();
    for sp in ts.spans.drain(..) {
        if sp.end <= a || sp.start >= b {
            out.push(sp);
            continue;
        }
        if sp.start < a {
            out.push(TextSpan {
                end: a,
                ..sp.clone()
            });
        }
        if sp.end > b {
            out.push(TextSpan { start: b, ..sp });
        }
    }
    ts.spans = out;
    ts.normalize();
}

fn set_text_range_styles(ed: &mut Editor, a: &Args) -> ToolResult {
    let def = page_arg(ed, a)?;
    let mut by_page: Vec<(usize, Vec<(NodeId, RangeEdit)>)> = Vec::new();
    for (i, u) in a.arr("updates")?.iter().enumerate() {
        let u = Args(u.as_object().ok_or("each update must be an object")?);
        let (page, id) = resolve(ed, u.req_str("nodeId")?, def)?;
        let n = ed.page_doc(page).get(id).unwrap();
        let Some(t) = n.kind.text() else {
            return Err(format!(
                "updates[{i}]: {} is a {}, not a text or sticky layer",
                n.name,
                n.kind.type_name()
            ));
        };
        let ranges = pick_ranges(&u, t).map_err(|e| format!("updates[{i}]: {e}"))?;
        let (span, fields) = match u.get("styles") {
            Some(s) => range_span(s, &ed.tokens).map_err(|e| format!("updates[{i}]: {e}"))?,
            None => (TextSpan::default(), Vec::new()),
        };
        let reset = u.get("reset").and_then(Value::as_bool).unwrap_or(false);
        if !reset && fields.is_empty() {
            return Err(format!(
                "updates[{i}]: pass `styles`, `reset: true` or both"
            ));
        }
        let mut fields = fields;
        if reset {
            fields.extend(["font", "fills"]);
        }
        let e = RangeEdit {
            ranges,
            span,
            reset,
            fields,
        };
        match by_page.iter_mut().find(|(p, _)| *p == page) {
            Some((_, l)) => l.push((id, e)),
            None => by_page.push((page, vec![(id, e)])),
        }
    }
    let mut results = Vec::new();
    for (page, list) in by_page {
        ed.with_page(page, |ed| {
            ed.edit("Style text", |tx| {
                for (id, e) in &list {
                    let node = tx.node_mut(*id).unwrap();
                    for &(s, t) in &e.ranges {
                        if e.reset {
                            clear_range(&mut node.text_style, s, t);
                        }
                        node.text_style.apply_range(s, t, |sp| {
                            let keep = (sp.start, sp.end);
                            *sp = e.span.clone();
                            (sp.start, sp.end) = keep;
                        });
                    }
                    node.text_style.normalize();
                    for f in &e.fields {
                        mark_override(node, f);
                    }
                    refit(tx, *id);
                }
            })
        });
        for (id, _) in &list {
            let n = ed.page_doc(page).get(*id).unwrap();
            results.push(json!({
                "id": super::exec::node_key(ed, page, *id),
                "textRanges": text_ranges(n.kind.text().map_or("", |s| s), &n.text_style),
            }));
        }
    }
    text(json!({"updated": results}))
}

/// Re-measure a text layer after a style change and re-run the auto layout around it.
fn refit(tx: &mut Tx, id: NodeId) {
    let size = tx.doc().get(id).map(|n| (n.rect.w, n.rect.h));
    fit_text(tx, id, false);
    if let Some(node) = tx.node_mut(id)
        && Some((node.rect.w, node.rect.h)) != size
    {
        mark_override(node, "size");
    }
    relayout(tx, Some(id));
}

// ------------------------------------------------------------------------------ arrows

#[derive(Clone)]
struct ArrowEdit {
    /// None: leave alone. Some(None): unbind. Some(Some(id)): bind.
    start: Option<Option<NodeId>>,
    end: Option<Option<NodeId>>,
    start_anchor: Option<V2>,
    end_anchor: Option<V2>,
    points: Option<Vec<V2>>,
    route: Option<Route>,
    start_head: Option<Head>,
    end_head: Option<Head>,
}

fn route_arg(s: &str) -> Result<Route, String> {
    match s.to_ascii_lowercase().as_str() {
        "straight" => Ok(Route::Straight),
        "curved" => Ok(Route::Curved),
        "elbow" => Ok(Route::Elbow),
        r => Err(format!(
            "unknown route `{r}`; use straight, curved or elbow"
        )),
    }
}

fn arrow_edit(ed: &Editor, page: usize, id: NodeId, u: &Args) -> Result<ArrowEdit, String> {
    let bind = |k: &str| -> Result<Option<Option<NodeId>>, String> {
        match u.0.get(k) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
            Some(Value::String(s)) => {
                let (pg, t) = resolve(ed, s, page)?;
                if pg != page {
                    return Err(format!("{k} is on another page"));
                }
                if t == id || ed.page_doc(page).is_ancestor(id, t) {
                    return Err(format!("{k} can't be the arrow itself"));
                }
                Ok(Some(Some(t)))
            }
            Some(_) => Err(format!("{k} must be a node id, \"\" or null")),
        }
    };
    let anchor = |k: &str| -> Result<Option<V2>, String> {
        match u.get(k) {
            None => Ok(None),
            Some(v) => xy(v).map(Some).ok_or(format!("{k} must be [fx, fy]")),
        }
    };
    let points = match u.get("points") {
        None => None,
        Some(v) => {
            let pts: Option<Vec<V2>> = v
                .as_array()
                .map(|l| l.iter().map(xy).collect())
                .unwrap_or(None);
            match pts {
                Some(p) if p.len() >= 2 => Some(p),
                _ => return Err("points must be [[x, y], ...] with two or more points".into()),
            }
        }
    };
    let head_arg = |k: &str| u.str(k).map(|s| head(Some(s), Head::None)).transpose();
    Ok(ArrowEdit {
        start: bind("startNodeId")?,
        end: bind("endNodeId")?,
        start_anchor: anchor("startAnchor")?,
        end_anchor: anchor("endAnchor")?,
        points,
        route: u.str("route").map(route_arg).transpose()?,
        start_head: head_arg("startHead")?,
        end_head: head_arg("endHead")?,
    })
}

fn apply_arrow(tx: &mut Tx, id: NodeId, e: &ArrowEdit) {
    // World points before the change: unbound ends stay where they are drawn.
    let Some(wr) = tx.doc().world_rect(id) else {
        return;
    };
    let mut pts: Vec<V2> = match &tx.doc().get(id).unwrap().kind {
        NodeKind::Connector { points, .. } => points.iter().map(|p| *p + wr.min()).collect(),
        _ => return,
    };
    if let Some(p) = &e.points {
        pts = p.clone();
    }
    let binding = |tx: &Tx, b: Option<NodeId>, anchor: Option<V2>, old: Option<Binding>| {
        b.map(|node| Binding {
            node,
            fixed: anchor
                .or(old.filter(|o| o.node == node).map(|o| o.fixed))
                .unwrap_or(v2(0.5, 0.5)),
            inside: false,
        })
        .filter(|b| tx.doc().get(b.node).is_some())
    };
    let NodeKind::Connector { start, end, .. } = tx.doc().get(id).unwrap().kind.clone() else {
        return;
    };
    let new_start = match e.start {
        Some(b) => binding(tx, b, e.start_anchor, start),
        None => start.map(|s| Binding {
            fixed: e.start_anchor.unwrap_or(s.fixed),
            ..s
        }),
    };
    let new_end = match e.end {
        Some(b) => binding(tx, b, e.end_anchor, end),
        None => end.map(|s| Binding {
            fixed: e.end_anchor.unwrap_or(s.fixed),
            ..s
        }),
    };
    let node = tx.node_mut(id).unwrap();
    if let NodeKind::Connector {
        start,
        end,
        route,
        start_head,
        end_head,
        ..
    } = &mut node.kind
    {
        *start = new_start;
        *end = new_end;
        if let Some(r) = e.route {
            *route = r;
            // Like the inspector: a straight arrow keeps only its two ends.
            if r == Route::Straight && pts.len() > 2 {
                pts = vec![pts[0], *pts.last().unwrap()];
            }
        }
        if let Some(h) = e.start_head {
            *start_head = h;
        }
        if let Some(h) = e.end_head {
            *end_head = h;
        }
    }
    set_world_points(tx, id, &pts);
    update_connector(tx, id);
}

fn update_arrows(ed: &mut Editor, a: &Args) -> ToolResult {
    let def = page_arg(ed, a)?;
    let mut by_page: Vec<(usize, Vec<(NodeId, ArrowEdit)>)> = Vec::new();
    for (i, u) in a.arr("updates")?.iter().enumerate() {
        let u = Args(u.as_object().ok_or("each update must be an object")?);
        let (page, id) = resolve(ed, u.req_str("nodeId")?, def)?;
        let n = ed.page_doc(page).get(id).unwrap();
        if !matches!(n.kind, NodeKind::Connector { .. }) {
            return Err(format!(
                "updates[{i}]: {} is a {}, not an arrow; create arrows with create_shape type \"arrow\"",
                n.name,
                n.kind.type_name()
            ));
        }
        let e = arrow_edit(ed, page, id, &u).map_err(|e| format!("updates[{i}]: {e}"))?;
        match by_page.iter_mut().find(|(p, _)| *p == page) {
            Some((_, l)) => l.push((id, e)),
            None => by_page.push((page, vec![(id, e)])),
        }
    }
    let mut results = Vec::new();
    for (page, list) in by_page {
        ed.with_page(page, |ed| {
            ed.edit("Edit arrow", |tx| {
                for (id, e) in &list {
                    apply_arrow(tx, *id, e);
                }
            })
        });
        for (id, _) in &list {
            let n = ed.page_doc(page).get(*id).unwrap();
            let mut v = json!({"id": super::exec::node_key(ed, page, *id)});
            node_details(ed, page, n, &mut v);
            results.push(v);
        }
    }
    text(json!({"updated": results}))
}

// ------------------------------------------------------------------------------ pencil

/// Point spacing of a finished stroke, as the pencil uses at 100% zoom.
const STROKE_SPACING: f64 = 1.5;

fn create_stroke(ed: &mut Editor, a: &Args) -> ToolResult {
    let def = page_arg(ed, a)?;
    let mut world = Vec::new();
    for (i, p) in a.arr("points")?.iter().enumerate() {
        let l = p.as_array().filter(|l| (2..=3).contains(&l.len()));
        let nums: Option<Vec<f64>> = l
            .map(|l| l.iter().map(Value::as_f64).collect())
            .unwrap_or(None);
        let Some(nums) = nums else {
            return Err(format!("points[{i}] must be [x, y] or [x, y, pressure]"));
        };
        world.push(StrokePoint {
            x: nums[0],
            y: nums[1],
            p: nums.get(2).copied().unwrap_or(0.5).clamp(0.0, 1.0),
        });
    }
    if world.len() < 2 {
        return Err("a stroke needs two or more points".into());
    }
    if a.get("smooth").and_then(Value::as_bool).unwrap_or(true) {
        world = crate::ink::resample_catmull_rom(&world, STROKE_SPACING);
    }
    let width = a.num("width").filter(|w| *w > 0.0).unwrap_or(ed.ink_width);
    let thinning = a.num("thinning").map_or(0.6, |t| t.clamp(0.0, 1.0));
    let (mut page, mut parent) = (def, None);
    if let Some(p) = a.str("parentId").filter(|p| !p.is_empty() && *p != ROOT) {
        let (pg, id) = resolve(ed, p, def)?;
        if !ed.page_doc(pg).get(id).unwrap().is_container() {
            return Err("parentId must be a frame or group".into());
        }
        (page, parent) = (pg, Some(id));
    }
    let color = match a.str("color") {
        Some(c) => Some(css::color(c).ok_or(format!("bad color `{c}`"))?),
        None => None,
    };
    let name = a.str("name").filter(|n| !n.is_empty()).map(String::from);
    let id = ed.with_page(page, |ed| {
        let start = v2(world[0].x, world[0].y);
        let color = color.unwrap_or_else(|| ed.ink_at(ed.ink_color, start));
        ed.edit("Draw", |tx| {
            let b = crate::ink::bounds(&world, width, thinning);
            let o = tx.doc().parent_origin(parent);
            let local: Vec<StrokePoint> = world
                .iter()
                .map(|p| StrokePoint {
                    x: p.x - b.x,
                    y: p.y - b.y,
                    p: p.p,
                })
                .collect();
            let mut n = Node::new(
                tx.alloc_id(),
                NodeKind::Stroke {
                    points: local,
                    width,
                    thinning,
                },
                b.translate(V2::ZERO - o),
            );
            n.style.set_fill(Some(color));
            if let Some(nm) = &name {
                n.name = nm.chars().take(50).collect();
            }
            if parent.is_some_and(|p| tx.doc().get(p).is_some_and(|f| f.layout.is_some())) {
                n.layout_child.absolute = true;
            }
            tx.insert(n, parent, None)
        })
    });
    let r = ed.page_doc(page).world_rect(id).unwrap_or_default();
    text(json!({
        "id": super::exec::node_key(ed, page, id),
        "pageId": ed.pages[page].id,
        "points": world.len(),
        "worldX": r2(r.x), "worldY": r2(r.y), "width": r2(r.w), "height": r2(r.h),
    }))
}
