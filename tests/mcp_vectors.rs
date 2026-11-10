//! MCP vector, text-range, arrow and pencil tools on a headless session: what an agent can do
//! to paths, styled text, arrows and freehand strokes once they exist.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{FillRule, Head, NodeId, NodeKind, Route};
use sketch_rs::project::Store;

fn host() -> exec::SessionHost {
    let s = Store::open(
        std::env::temp_dir().join(format!("sketch-rs-mcp-vec-{:016x}", fastrand::u64(..))),
    )
    .unwrap();
    let mut h = exec::SessionHost::new(s);
    h.session.create(&mut h.ed, "Vectors").unwrap();
    h
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    match exec::call(h, name, &args) {
        Ok(outs) => match outs.into_iter().next() {
            Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
            _ => Value::Null,
        },
        Err(e) => panic!("{name} failed: {e}"),
    }
}

fn err(h: &mut exec::SessionHost, name: &str, args: Value) -> String {
    exec::call(h, name, &args).unwrap_err()
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

fn rect(h: &mut exec::SessionHost, x: f64, y: f64, w: f64, page: Option<&str>) -> Value {
    let mut a = json!({"type": "rectangle", "x": x, "y": y, "width": w, "height": w,
        "styles": {"backgroundColor": "#3366ff"}});
    if let Some(p) = page {
        a["pageId"] = json!(p);
    }
    call(h, "create_shape", a)["id"].clone()
}

#[test]
fn tools_are_listed_with_schemas() {
    let names: Vec<String> = sketch_rs::mcp::tools::definitions()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for n in [
        "boolean_nodes",
        "flatten_nodes",
        "set_vector_points",
        "set_text_range_styles",
        "update_arrows",
        "create_stroke",
    ] {
        assert!(names.iter().any(|x| x == n), "{n} missing");
    }
    // The rmcp catalogue accepts every definition.
    assert_eq!(sketch_rs::mcp::server::tool_list().len(), names.len());
}

#[test]
fn booleans_combine_layers_on_any_page_and_leave_the_selection_alone() {
    let mut h = host();
    let other = call(&mut h, "create_page", json!({"name": "Shapes"}))["pageId"]
        .as_str()
        .unwrap()
        .to_string();
    let a = rect(&mut h, 0.0, 0.0, 100.0, Some(&other));
    let b = rect(&mut h, 50.0, 50.0, 100.0, Some(&other));
    let r = call(
        &mut h,
        "boolean_nodes",
        json!({"nodeIds": [a, b], "operation": "union"}),
    );
    let doc = h.ed.page_doc(1);
    assert_eq!(doc.nodes.len(), 1, "operands replaced by the result");
    let n = doc.get(num(&r["id"])).unwrap();
    assert!(matches!(n.kind, NodeKind::Path { .. }));
    assert_eq!(
        (n.rect.x, n.rect.y, n.rect.w, n.rect.h),
        (0.0, 0.0, 150.0, 150.0)
    );
    assert_eq!(h.ed.active_page, 0);

    // Subtract on the user's page keeps what they had selected.
    let keep = rect(&mut h, 500.0, 0.0, 10.0, None);
    h.ed.selection = vec![num(&keep)];
    let a = rect(&mut h, 0.0, 0.0, 100.0, None);
    let b = rect(&mut h, 50.0, 0.0, 100.0, None);
    let r = call(
        &mut h,
        "boolean_nodes",
        json!({"nodeIds": [a, b], "operation": "subtract"}),
    );
    let n = h.ed.doc.get(num(&r["id"])).unwrap();
    assert_eq!((n.rect.w, n.rect.h), (50.0, 100.0));
    assert_eq!(h.ed.selection, vec![num(&keep)]);
    // One undo brings both operands back.
    call(&mut h, "undo", json!({}));
    assert_eq!(h.ed.doc.nodes.len(), 3);

    // Disjoint intersect: nothing left.
    let a = rect(&mut h, 0.0, 300.0, 10.0, None);
    let b = rect(&mut h, 100.0, 300.0, 10.0, None);
    let r = call(
        &mut h,
        "boolean_nodes",
        json!({"nodeIds": [a, b], "operation": "intersect"}),
    );
    assert!(r["id"].is_null());

    let e = err(
        &mut h,
        "boolean_nodes",
        json!({"nodeIds": [keep], "operation": "union"}),
    );
    assert!(e.contains("two or more"), "{e}");
    let e = err(
        &mut h,
        "boolean_nodes",
        json!({"nodeIds": [keep, keep], "operation": "merge"}),
    );
    assert!(e.contains("unknown operation"), "{e}");
}

#[test]
fn flatten_then_edit_points_in_world_units() {
    let mut h = host();
    let r = rect(&mut h, 100.0, 100.0, 50.0, None);
    let frame = call(
        &mut h,
        "create_artboard",
        json!({"name": "F", "styles": {"width": "100px", "height": "100px", "left": "0px", "top": "0px"}}),
    );
    let f = call(
        &mut h,
        "flatten_nodes",
        json!({"nodeIds": [r, frame["id"]]}),
    );
    assert_eq!(f["flattened"], json!([r]));
    assert_eq!(f["skipped"], json!([frame["id"]]));
    let info = call(&mut h, "get_node_info", json!({"nodeId": r}));
    let pts = &info["vector"]["subpaths"][0]["points"];
    assert_eq!(pts[0]["x"], 100.0);
    assert_eq!(info["vector"]["subpaths"][0]["closed"], true);

    // Move one point, add a curve handle and send it back.
    let mut v = info["vector"].clone();
    v["subpaths"][0]["points"][2] = json!({"x": 200, "y": 200, "in": [210, 160]});
    call(
        &mut h,
        "set_vector_points",
        json!({"nodeId": r, "subpaths": v["subpaths"], "fillRule": "evenodd"}),
    );
    let n = h.ed.doc.get(num(&r)).unwrap();
    let NodeKind::Path {
        subpaths,
        fill_rule,
    } = &n.kind
    else {
        panic!()
    };
    assert_eq!(*fill_rule, FillRule::EvenOdd);
    assert_eq!(n.rect.x, 100.0);
    assert!(
        n.rect.w > 99.0,
        "box refit to the moved point: {:?}",
        n.rect
    );
    let wr = h.ed.doc.world_rect(num(&r)).unwrap();
    let p = subpaths[0].points[2];
    assert_eq!((p.p.x + wr.x, p.p.y + wr.y), (200.0, 200.0));
    assert!(p.h_in.is_some());
    // Reading it back gives the same world numbers.
    let info = call(&mut h, "get_node_info", json!({"nodeId": r}));
    assert_eq!(info["vector"]["subpaths"][0]["points"][2]["x"], 200.0);

    // SVG path data works too.
    call(
        &mut h,
        "set_vector_points",
        json!({"nodeId": r, "d": "M10 20 L110 20 L60 120 Z"}),
    );
    let wr = h.ed.doc.world_rect(num(&r)).unwrap();
    assert_eq!((wr.x, wr.y, wr.w, wr.h), (10.0, 20.0, 100.0, 100.0));

    // Shapes must be flattened first.
    let other = rect(&mut h, 0.0, 400.0, 10.0, None);
    let e = err(
        &mut h,
        "set_vector_points",
        json!({"nodeId": other, "d": "M0 0 L1 1"}),
    );
    assert!(e.contains("flatten_nodes"), "{e}");
    let e = err(
        &mut h,
        "set_vector_points",
        json!({"nodeId": r, "subpaths": [{"points": [{"x": 0, "y": 0}]}]}),
    );
    assert!(e.contains("two or more"), "{e}");
}

#[test]
fn text_ranges_style_matches_and_offsets_and_reset() {
    let mut h = host();
    let t = call(
        &mut h,
        "create_shape",
        json!({"type": "text", "text": "Ship it fast, ship it now", "x": 0, "y": 0}),
    )["id"]
        .clone();
    let w0 = h.ed.doc.get(num(&t)).unwrap().rect.w;
    let r = call(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [
            {"nodeId": t, "match": "ship it", "styles": {"fontWeight": 700, "color": "#ff0000"}},
            {"nodeId": t, "start": 0, "end": 4, "styles": {"fontStyle": "italic", "textDecoration": "underline"}},
        ]}),
    );
    assert_eq!(
        r["updated"].as_array().unwrap().len(),
        1,
        "one entry per layer"
    );
    let ranges = r["updated"][0]["textRanges"].as_array().unwrap();
    assert_eq!(ranges[0]["text"], "Ship");
    assert_eq!(ranges[0]["styles"]["fontStyle"], "italic");
    let n = h.ed.doc.get(num(&t)).unwrap();
    let ink = sketch_rs::model::Color::BLACK;
    let bold = n.text_style.char_style(14, 20.0, ink);
    assert_eq!(bold.weight, 700);
    assert_eq!(bold.color, sketch_rs::model::Color::rgb(255, 0, 0));
    assert_eq!(n.text_style.char_style(0, 20.0, ink).weight, 400);
    assert!(n.text_style.char_style(0, 20.0, ink).underline);
    assert!(n.rect.w > w0, "bold text is wider and the box refits");

    // get_node_info lists the ranges.
    let info = call(&mut h, "get_node_info", json!({"nodeId": t}));
    assert!(
        info["textRanges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["text"] == "ship it" && r["styles"]["fontWeight"] == 700)
    );

    // All occurrences, then reset part of one.
    call(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [{"nodeId": t, "match": "it", "occurrence": "all", "styles": {"fontSize": "30px"}}]}),
    );
    let n = h.ed.doc.get(num(&t)).unwrap();
    assert_eq!(n.text_style.char_style(5, 20.0, ink).size, 30.0);
    assert_eq!(n.text_style.char_style(19, 20.0, ink).size, 30.0);
    call(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [{"nodeId": t, "start": 14, "end": 25, "reset": true}]}),
    );
    let n = h.ed.doc.get(num(&t)).unwrap();
    let s = n.text_style.char_style(19, 20.0, ink);
    assert_eq!((s.size, s.weight), (20.0, 400));
    assert_eq!(n.text_style.char_style(5, 20.0, ink).size, 30.0);

    let e = err(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [{"nodeId": t, "match": "nope", "styles": {"fontWeight": 700}}]}),
    );
    assert!(e.contains("not in the text"), "{e}");
    let e = err(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [{"nodeId": t, "start": 3, "end": 99, "styles": {"fontWeight": 700}}]}),
    );
    assert!(e.contains("25 characters"), "{e}");
    let e = err(
        &mut h,
        "set_text_range_styles",
        json!({"updates": [{"nodeId": t, "styles": {"padding": "4px"}}]}),
    );
    assert!(e.contains("can't style a text range"), "{e}");
}

#[test]
fn arrows_rebind_reroute_and_change_heads() {
    let mut h = host();
    let a = rect(&mut h, 0.0, 0.0, 100.0, None);
    let b = rect(&mut h, 400.0, 0.0, 100.0, None);
    let c = rect(&mut h, 0.0, 400.0, 100.0, None);
    let arrow = call(
        &mut h,
        "create_shape",
        json!({"type": "arrow", "startNodeId": a, "endNodeId": b}),
    )["id"]
        .clone();
    let info = call(&mut h, "get_node_info", json!({"nodeId": arrow}));
    assert_eq!(info["arrow"]["end"]["nodeId"], b);
    assert_eq!(info["arrow"]["endHead"], "arrow");

    // Rebind the end to c, elbow route, new heads.
    let r = call(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": arrow, "endNodeId": c, "route": "elbow", "startHead": "dot", "endHead": "triangle"}]}),
    );
    assert_eq!(r["updated"][0]["arrow"]["end"]["nodeId"], c);
    let n = h.ed.doc.get(num(&arrow)).unwrap();
    let NodeKind::Connector {
        end,
        route,
        start_head,
        end_head,
        ..
    } = &n.kind
    else {
        panic!()
    };
    assert_eq!(end.unwrap().node, num(&c));
    assert_eq!(
        (*route, *start_head, *end_head),
        (Route::Elbow, Head::Dot, Head::Triangle)
    );
    let last = *h.ed.connector_points(num(&arrow)).unwrap().last().unwrap();
    assert!((last.y - 400.0).abs() < 20.0, "routed to c's top: {last:?}");

    // It follows c when c moves.
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [c], "styles": {"left": "600px", "top": "600px"}}]}),
    );
    let last = *h.ed.connector_points(num(&arrow)).unwrap().last().unwrap();
    assert!(last.x > 590.0 && last.y > 590.0, "{last:?}");

    // Unbind both ends and draw it by points, curved through a middle point.
    call(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": arrow, "startNodeId": null, "endNodeId": "", "route": "curved",
            "points": [[0, 200], [100, 150], [200, 200]]}]}),
    );
    let pts = h.ed.connector_points(num(&arrow)).unwrap();
    assert_eq!(pts.len(), 3);
    assert_eq!((pts[0].x, pts[0].y, pts[2].x), (0.0, 200.0, 200.0));
    let info = call(&mut h, "get_node_info", json!({"nodeId": arrow}));
    assert!(info["arrow"]["start"].is_null() && info["arrow"]["end"].is_null());
    // Straight keeps only the ends.
    call(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": arrow, "route": "straight"}]}),
    );
    assert_eq!(h.ed.connector_points(num(&arrow)).unwrap().len(), 2);
    // An anchor aims the end at part of the shape.
    call(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": arrow, "endNodeId": b, "endAnchor": [0.5, 0.0]}]}),
    );
    let info = call(&mut h, "get_node_info", json!({"nodeId": arrow}));
    assert_eq!(info["arrow"]["end"]["anchor"], json!([0.5, 0.0]));

    let e = err(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": a, "route": "elbow"}]}),
    );
    assert!(e.contains("not an arrow"), "{e}");
    let e = err(
        &mut h,
        "update_arrows",
        json!({"updates": [{"nodeId": arrow, "endNodeId": arrow}]}),
    );
    assert!(e.contains("itself"), "{e}");
}

#[test]
fn pencil_strokes_draw_recolor_and_erase() {
    let mut h = host();
    let s = call(
        &mut h,
        "create_stroke",
        json!({"points": [[0, 0, 0.2], [50, 30], [100, 0, 0.9]], "width": 4, "color": "#e03131"}),
    );
    let id = num(&s["id"]);
    let n = h.ed.doc.get(id).unwrap();
    let NodeKind::Stroke { points, width, .. } = &n.kind else {
        panic!()
    };
    assert!(points.len() > 3, "smoothed through the points");
    assert_eq!(*width, 4.0);
    assert!(n.rect.w >= 100.0);
    let info = call(&mut h, "get_node_info", json!({"nodeId": s["id"]}));
    assert_eq!(info["stroke"]["color"], "#e03131");
    assert_eq!(info["type"], "Drawing");

    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [s["id"]], "styles": {"color": "#1971c2"}}]}),
    );
    let info = call(&mut h, "get_node_info", json!({"nodeId": s["id"]}));
    assert_eq!(info["stroke"]["color"], "#1971c2");

    // Inside a frame, points stay world units.
    let f = call(
        &mut h,
        "create_artboard",
        json!({"name": "Board", "styles": {"width": "300px", "height": "300px", "left": "500px", "top": "0px"}}),
    );
    let s2 = call(
        &mut h,
        "create_stroke",
        json!({"points": [[520, 20], [560, 60]], "parentId": f["id"], "smooth": false}),
    );
    let wr = h.ed.doc.world_rect(num(&s2["id"])).unwrap();
    assert!(wr.x > 510.0 && wr.x < 520.0, "{wr:?}");
    assert_eq!(
        h.ed.doc.get(num(&s2["id"])).unwrap().parent,
        Some(num(&f["id"]))
    );

    call(&mut h, "delete_nodes", json!({"nodeIds": [s["id"]]}));
    assert!(h.ed.doc.get(id).is_none());
    let e = err(&mut h, "create_stroke", json!({"points": [[0, 0]]}));
    assert!(e.contains("two or more"), "{e}");
}
