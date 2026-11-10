//! MCP layer and layout properties: constraints, removing auto layout, strokes, polygon and
//! star geometry, radial gradient size, and the lock, group, flip, align and distribute tools.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Constraint, NodeId, NodeKind, Paint, StrokeAlign};
use sketch_rs::project::Store;

fn host() -> exec::SessionHost {
    let dir = std::env::temp_dir().join(format!("sketch-rs-layers-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(dir).unwrap());
    h.session.create(&mut h.ed, "Layers").unwrap();
    h
}

fn try_call(h: &mut exec::SessionHost, name: &str, args: Value) -> Result<Value, String> {
    exec::call(h, name, &args).map(|outs| match outs.into_iter().next() {
        Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
        _ => Value::Null,
    })
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    try_call(h, name, args).unwrap_or_else(|e| panic!("{name} failed: {e}"))
}

fn num(id: &Value) -> NodeId {
    NodeId(
        id.as_str()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap(),
    )
}

fn styles(h: &mut exec::SessionHost, id: &Value) -> Value {
    let v = call(h, "get_computed_styles", json!({"nodeIds": [id]}));
    v[id.as_str().unwrap()].clone()
}

/// A free 400×300 frame on the page with three 40×40 boxes inside it.
fn board(h: &mut exec::SessionHost) -> (Value, Vec<Value>) {
    let ab = call(
        h,
        "create_artboard",
        json!({"name": "Board", "styles": {"width": "400px", "height": "300px", "left": 0, "top": 0}}),
    );
    call(
        h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ab["id"]], "styles": {"autoLayout": false}}]}),
    );
    let boxes = [(10, 10), (100, 50), (300, 20)]
        .iter()
        .map(|(x, y)| {
            let r = call(
                h,
                "write_html",
                json!({
                    "targetNodeId": ab["id"], "mode": "insert-children",
                    "html": format!("<div style=\"position:absolute; left:{x}px; top:{y}px; width:40px; height:40px; background:#f00\"></div>"),
                }),
            );
            r["createdNodeIds"][0].clone()
        })
        .collect();
    (ab["id"].clone(), boxes)
}

fn rect(h: &exec::SessionHost, id: &Value) -> (f64, f64, f64, f64) {
    let r = h.ed.doc.get(num(id)).unwrap().rect;
    (r.x, r.y, r.w, r.h)
}

#[test]
fn constraints_follow_the_parent_and_read_back() {
    let mut h = host();
    let (ab, b) = board(&mut h);
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [
            {"nodeIds": [b[2]], "styles": {"horizontalConstraint": "right", "verticalConstraint": "top-bottom"}},
            {"nodeIds": [ab], "styles": {"horizontalConstraint": "center"}},
        ]}),
    );
    // A layer on the page has no parent to follow.
    assert_eq!(
        r["ignoredStyles"][ab.as_str().unwrap()],
        json!(["horizontalConstraint"])
    );
    let c = h.ed.doc.get(num(&b[2])).unwrap().layout_child.constraints;
    assert_eq!((c.h, c.v), (Constraint::Max, Constraint::Stretch));
    let st = styles(&mut h, &b[2]);
    assert_eq!(st["horizontalConstraint"], "right");
    assert_eq!(st["verticalConstraint"], "top-bottom");
    assert!(styles(&mut h, &b[0]).get("horizontalConstraint").is_none());

    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ab], "styles": {"width": "500px", "height": "400px"}}]}),
    );
    assert_eq!(rect(&h, &b[2]), (400.0, 20.0, 40.0, 140.0));
    assert_eq!(rect(&h, &b[0]), (10.0, 10.0, 40.0, 40.0));
}

#[test]
fn auto_layout_false_keeps_children_in_place() {
    let mut h = host();
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Row", "styles": {"width": "fit-content", "height": "fit-content", "display": "flex", "flexDirection": "row", "gap": "20px", "padding": "10px"}}),
    );
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab["id"], "mode": "insert-children",
               "html": "<div style=\"width:30px;height:30px;background:#00f\"></div><div style=\"width:30px;height:30px;background:#0f0\"></div>"}),
    );
    let second = r["createdNodeIds"][1].clone();
    let before = rect(&h, &second);
    let frame_before = rect(&h, &ab["id"]);
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ab["id"]], "styles": {"autoLayout": false}}]}),
    );
    let n = h.ed.doc.get(num(&ab["id"])).unwrap();
    assert!(n.layout.is_none());
    assert_eq!(rect(&h, &second), before);
    assert_eq!(rect(&h, &ab["id"]), frame_before);
    assert_eq!(styles(&mut h, &second)["position"], "absolute");
    assert_eq!(
        call(&mut h, "get_node_info", json!({"nodeId": ab["id"]}))["autoLayout"],
        false
    );
    // Only frames have auto layout.
    let rect = call(
        &mut h,
        "create_shape",
        json!({"type": "rectangle", "x": 0, "y": 500}),
    );
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [rect["id"]], "styles": {"autoLayout": false}}]}),
    );
    assert_eq!(
        r["ignoredStyles"][rect["id"].as_str().unwrap()],
        json!(["autoLayout"])
    );
}

#[test]
fn stroke_align_caps_and_joins_round_trip() {
    let mut h = host();
    let (_, b) = board(&mut h);
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [b[0]], "styles": {
            "strokeAlign": "outside", "border": "4px solid #000",
            "strokeLinecap": "round", "strokeLinejoin": "bevel",
        }}]}),
    );
    let s = &h.ed.doc.get(num(&b[0])).unwrap().style;
    assert_eq!(
        s.stroke_align,
        StrokeAlign::Outside,
        "strokeAlign wins over border's inside"
    );
    let st = styles(&mut h, &b[0]);
    assert_eq!(
        (
            &st["strokeAlign"],
            &st["strokeLinecap"],
            &st["strokeLinejoin"]
        ),
        (&json!("outside"), &json!("round"), &json!("bevel"))
    );
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [b[0]], "styles": {"strokeAlign": "inside"}}]}),
    );
    assert!(
        styles(&mut h, &b[0]).get("strokeAlign").is_none(),
        "inside is what border means"
    );
}

#[test]
fn polygon_sides_and_star_ratio_change_after_creation() {
    let mut h = host();
    let poly = call(
        &mut h,
        "create_shape",
        json!({"type": "polygon", "x": 0, "y": 0, "width": 80, "height": 80}),
    );
    let star = call(
        &mut h,
        "create_shape",
        json!({"type": "star", "x": 100, "y": 0, "width": 80, "height": 80, "styles": {"innerRadius": 0.5}}),
    );
    let (poly, star) = (poly["id"].clone(), star["id"].clone());
    call(
        &mut h,
        "update_styles",
        json!({"updates": [
            {"nodeIds": [poly], "styles": {"sides": 8}},
            {"nodeIds": [star], "styles": {"sides": 7, "innerRadius": "60%"}},
        ]}),
    );
    assert_eq!(
        h.ed.doc.get(num(&poly)).unwrap().kind,
        NodeKind::Polygon { sides: 8 }
    );
    assert_eq!(
        h.ed.doc.get(num(&star)).unwrap().kind,
        NodeKind::Star {
            points: 7,
            inner: 0.6
        }
    );
    let st = styles(&mut h, &star);
    assert_eq!(
        (&st["sides"], &st["innerRadius"]),
        (&json!("7"), &json!("0.6"))
    );
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [poly], "styles": {"innerRadius": 0.5}}]}),
    );
    assert_eq!(
        r["ignoredStyles"][poly.as_str().unwrap()],
        json!(["innerRadius"])
    );
}

#[test]
fn radial_gradients_take_a_size_and_center() {
    let mut h = host();
    let (_, b) = board(&mut h);
    let set = |h: &mut exec::SessionHost, bg: &str| {
        call(
            h,
            "update_styles",
            json!({"updates": [{"nodeIds": [b[0]], "styles": {"width": "200px", "height": "100px", "background": bg}}]}),
        );
        match h
            .ed
            .doc
            .get(num(&b[0]))
            .unwrap()
            .style
            .fills
            .last()
            .unwrap()
        {
            Paint::Radial { center, radius, .. } => (center.x, center.y, radius.x, radius.y),
            p => panic!("{p:?}"),
        }
    };
    // Measured against the 200×100 box the same update makes, not the 40×40 one before it.
    assert_eq!(
        set(
            &mut h,
            "radial-gradient(circle 50px at left top, #fff, #000)"
        ),
        (0.0, 0.0, 0.25, 0.5)
    );
    assert_eq!(
        set(&mut h, "radial-gradient(#fff, #000)"),
        (0.5, 0.5, 0.5, 0.5)
    );
    assert_eq!(
        set(
            &mut h,
            "radial-gradient(ellipse 30% 60% at 25% 75%, #fff, #000)"
        ),
        (0.25, 0.75, 0.3, 0.6)
    );
    assert_eq!(
        set(&mut h, "radial-gradient(circle closest-side, #fff, #000)"),
        (0.5, 0.5, 0.25, 0.5)
    );
    let (_, _, rx, ry) = set(
        &mut h,
        "radial-gradient(farthest-corner at 0% 0%, #fff, #000)",
    );
    assert!((rx - 2f64.sqrt()).abs() < 1e-9 && (ry - 2f64.sqrt()).abs() < 1e-9);

    set(
        &mut h,
        "radial-gradient(ellipse 30% 60% at 25% 75%, #fff, #000)",
    );
    let css = styles(&mut h, &b[0])["background"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        css,
        "radial-gradient(ellipse 30% 60% at 25% 75%, #ffffff 0%, #000000 100%)"
    );
    assert_eq!(set(&mut h, &css), (0.25, 0.75, 0.3, 0.6));
}

#[test]
fn lock_group_and_ungroup() {
    let mut h = host();
    let (ab, b) = board(&mut h);
    h.ed.selection = vec![num(&b[2])];

    call(&mut h, "lock_nodes", json!({"nodeIds": [b[0]]}));
    assert_eq!(
        call(&mut h, "get_node_info", json!({"nodeId": b[0]}))["locked"],
        true
    );
    call(
        &mut h,
        "lock_nodes",
        json!({"nodeIds": [b[0]], "locked": false}),
    );
    assert!(!h.ed.doc.get(num(&b[0])).unwrap().locked);

    let before = (rect(&h, &b[0]), rect(&h, &b[1]));
    let g = call(&mut h, "group_nodes", json!({"nodeIds": [b[0], b[1]]}));
    let gid = num(&g["groupId"]);
    let group = h.ed.doc.get(gid).unwrap();
    assert!(group.is_group() && group.parent == Some(num(&ab)));
    assert_eq!(g["childIds"], json!([b[0], b[1]]));
    assert_eq!(h.ed.doc.world_rect(num(&b[0])).unwrap().x, before.0.0);
    assert_eq!(
        h.ed.selection,
        vec![num(&b[2])],
        "the user's selection is kept"
    );

    let e = try_call(&mut h, "group_nodes", json!({"nodeIds": [b[0], b[2]]})).unwrap_err();
    assert!(e.contains("same parent"), "{e}");

    let r = call(
        &mut h,
        "ungroup_nodes",
        json!({"nodeIds": [g["groupId"], g["groupId"]]}),
    );
    assert_eq!(r["childIds"], json!([b[0], b[1]]));
    assert!(h.ed.doc.get(gid).is_none());
    assert_eq!((rect(&h, &b[0]), rect(&h, &b[1])), before);
    assert!(try_call(&mut h, "ungroup_nodes", json!({"nodeIds": [b[0]]})).is_err());
}

#[test]
fn flip_align_and_distribute() {
    let mut h = host();
    let (ab, b) = board(&mut h);

    // One layer centers in its parent, both axes in one undo step.
    let r = call(
        &mut h,
        "align_nodes",
        json!({"nodeIds": [b[1]], "horizontal": "center", "vertical": "center"}),
    );
    assert_eq!(r["moved"], json!([b[1]]));
    assert_eq!(rect(&h, &b[1]), (180.0, 130.0, 40.0, 40.0));
    call(&mut h, "undo", json!({}));
    assert_eq!(rect(&h, &b[1]), (100.0, 50.0, 40.0, 40.0));

    // Several align to their bounds; a locked one stays but counts.
    call(&mut h, "lock_nodes", json!({"nodeIds": [b[2]]}));
    let r = call(
        &mut h,
        "align_nodes",
        json!({"nodeIds": b, "vertical": "top"}),
    );
    assert_eq!(r["moved"], json!([b[1]]));
    assert_eq!(r["locked"], json!([b[2]]));
    assert_eq!(rect(&h, &b[1]).1, 10.0);
    call(
        &mut h,
        "lock_nodes",
        json!({"nodeIds": [b[2]], "locked": false}),
    );

    call(
        &mut h,
        "distribute_nodes",
        json!({"nodeIds": b, "direction": "horizontal"}),
    );
    assert_eq!(rect(&h, &b[1]).0, 155.0);
    assert!(
        try_call(
            &mut h,
            "distribute_nodes",
            json!({"nodeIds": [b[0], b[1]], "direction": "vertical"})
        )
        .is_err()
    );

    // Several flip as one within their bounds.
    let r = call(
        &mut h,
        "flip_nodes",
        json!({"nodeIds": [b[0], b[2]], "direction": "horizontal"}),
    );
    assert_eq!(r["flipped"], json!([b[0], b[2]]));
    assert_eq!((rect(&h, &b[0]).0, rect(&h, &b[2]).0), (300.0, 10.0));

    let e = try_call(
        &mut h,
        "align_nodes",
        json!({"nodeIds": [ab], "horizontal": "left"}),
    )
    .unwrap_err();
    assert!(e.contains("parent"), "{e}");
}

#[test]
fn flip_mirrors_a_vector() {
    let mut h = host();
    let line = call(
        &mut h,
        "create_shape",
        json!({"type": "line", "points": [[0, 0], [100, 40]]}),
    );
    call(
        &mut h,
        "flip_nodes",
        json!({"nodeIds": [line["id"]], "direction": "vertical"}),
    );
    let NodeKind::Line { a, b } = h.ed.doc.get(num(&line["id"])).unwrap().kind else {
        panic!()
    };
    assert_eq!((a.y, b.y), (40.0, 0.0));
}

#[test]
fn constraints_only_apply_where_the_layer_is_free() {
    let mut h = host();
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Flex", "styles": {"width": "300px", "height": "200px"}}),
    );
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab["id"], "mode": "insert-children",
               "html": "<div style=\"width:40px;height:40px\"></div><div style=\"position:absolute;left:0;top:0;width:40px;height:40px\"></div>"}),
    );
    let (flow, abs) = (&r["createdNodeIds"][0], &r["createdNodeIds"][1]);
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [flow, abs], "styles": {"horizontalConstraint": "right"}}]}),
    );
    // The auto layout places the first one; the absolute one follows constraints.
    assert_eq!(
        r["ignoredStyles"],
        json!({flow.as_str().unwrap(): ["horizontalConstraint"]})
    );
    assert_eq!(styles(&mut h, abs)["horizontalConstraint"], "right");
}
