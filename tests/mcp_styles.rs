//! An agent builds a token setup over MCP: modes, an elevation scale of effect styles, and
//! per-effect visibility on layers.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, EffectKind, NodeId, TokenProp};
use sketch_rs::project::Store;

fn store() -> Store {
    let dir = std::env::temp_dir().join(format!("sketch-rs-styles-{:016x}", fastrand::u64(..)));
    Store::open(dir).unwrap()
}

fn host(store: &Store) -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store.clone());
    h.session.create(&mut h.ed, "Tokens").unwrap();
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

fn node(id: &Value) -> NodeId {
    let n = id.as_str().unwrap().rsplit(':').next().unwrap();
    NodeId(n.parse().unwrap())
}

fn modes(h: &mut exec::SessionHost) -> Value {
    call(h, "get_tokens", json!({}))["modes"].clone()
}

fn card(h: &mut exec::SessionHost, style: &str) -> Value {
    let ab = call(
        h,
        "create_artboard",
        json!({"name": "Page", "styles": {"width": "400px", "height": "300px"}}),
    )["id"]
        .clone();
    let html = format!("<div style=\"width: 100px; height: 60px; {style}\"></div>");
    let r = call(
        h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children", "html": html}),
    );
    r["createdNodeIds"][0].clone()
}

#[test]
fn modes_are_added_renamed_and_removed() {
    let store = store();
    let mut h = host(&store);
    assert_eq!(modes(&mut h), json!(["Light", "Dark"]));
    let r = call(
        &mut h,
        "create_modes",
        json!({"names": ["Dark", "High contrast", "high-contrast", " "]}),
    );
    let res = &r["results"];
    assert_eq!(res[0]["result"], "exists");
    assert_eq!(res[1]["result"], "created");
    assert_eq!(
        res[2]["result"], "exists",
        "names match as data-theme values"
    );
    assert_eq!(res[3]["result"], "error");
    assert_eq!(r["modes"], json!(["Light", "Dark", "High contrast"]));

    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--bg", "value": "#ffffff",
                           "modes": {"Dark": "#111111", "High contrast": "#000000"}}]}),
    );
    let r = call(
        &mut h,
        "set_modes",
        json!({"modes": [
            {"name": "high contrast", "newName": "Contrast"},
            {"name": "Dusk", "newName": "x"},
            {"name": "Light", "newName": "Dark"},
            {"name": "Light", "delete": true},
        ]}),
    );
    let res = &r["results"];
    assert_eq!(res[0]["result"], "renamed");
    assert_eq!(res[1]["result"], "error");
    assert!(
        res[1]["message"]
            .as_str()
            .unwrap()
            .contains("Light, Dark, Contrast")
    );
    assert_eq!(res[2]["result"], "error", "names stay unique");
    assert_eq!(res[3]["result"], "deleted");
    assert_eq!(r["modes"], json!(["Dark", "Contrast"]));
    let t = &call(&mut h, "get_tokens", json!({}))["tokens"][0];
    assert_eq!(
        t["value"], "#111111",
        "Dark became the default and kept its value"
    );
    assert_eq!(t["modes"], json!({"Contrast": "#000000"}));

    // One undo step per call.
    call(&mut h, "undo", json!({}));
    assert_eq!(modes(&mut h), json!(["Light", "Dark", "High contrast"]));

    call(
        &mut h,
        "set_modes",
        json!({"modes": [{"name": "Dark", "delete": true}]}),
    );
    call(
        &mut h,
        "set_modes",
        json!({"modes": [{"name": "High contrast", "delete": true}]}),
    );
    let r = call(
        &mut h,
        "set_modes",
        json!({"modes": [{"name": "Light", "delete": true}]}),
    );
    assert_eq!(r["results"][0]["result"], "error", "a file keeps one mode");
    assert_eq!(
        modes(&mut h),
        json!(["Light"]),
        "listed even when there is one"
    );
}

#[test]
fn an_agent_builds_an_elevation_scale_that_follows_modes() {
    let store = store();
    let mut h = host(&store);
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--shadow", "value": "#0000001a",
                           "modes": {"Dark": "#00000080"}}]}),
    );
    let r = call(
        &mut h,
        "create_effect_styles",
        json!({"styles": [
            {"name": "Elevation/1", "effects": [
                {"type": "drop-shadow", "y": 1, "blur": 2, "color": "var(--shadow)"}]},
            {"name": "Elevation/2", "description": "Cards", "effects": [
                {"type": "drop-shadow", "y": 2, "blur": 4, "color": "var(--shadow)"},
                {"type": "drop-shadow", "y": 4, "blur": 8, "spread": -2, "color": "#0000000d"}]},
            {"name": "Glass", "effects": [{"type": "background-blur", "blur": 12}]},
            {"name": "elevation 1", "effects": []},
            {"name": "Bad", "effects": [{"type": "layer-blur", "x": 3}]},
            {"name": "Bad", "effects": [{"type": "drop-shadow", "color": "var(--nope)"}]},
        ]}),
    );
    let res = &r["styles"];
    assert_eq!(res[0], json!({"var": "--elevation-1", "result": "created"}));
    assert_eq!(res[1]["var"], "--elevation-2");
    assert_eq!(res[2]["var"], "--glass");
    for (i, why) in [
        (3, "already uses"),
        (4, "shadows only"),
        (5, "not a color token"),
    ] {
        assert_eq!(res[i]["result"], "error");
        let m = res[i]["message"].as_str().unwrap();
        assert!(m.contains(why), "{m}");
    }

    let list = call(&mut h, "get_tokens", json!({"types": ["effectStyle"]}));
    let e2 = &list["effectStyles"][1];
    assert_eq!(e2["description"], "Cards");
    assert_eq!(
        e2["effects"][0],
        json!({"type": "drop-shadow", "x": 0.0, "y": 2.0, "blur": 4.0, "spread": 0.0,
               "color": "var(--shadow)", "visible": true})
    );
    assert_eq!(
        list["effectStyles"][2]["effects"],
        json!([{"type": "background-blur", "blur": 12.0, "visible": true}])
    );

    // A card in a Dark frame shows the Dark shadow color.
    let id = card(&mut h, "box-shadow: var(--elevation-2)");
    let parent = h.ed.doc.get(node(&id)).unwrap().parent.unwrap();
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [format!("{}", parent.0)], "styles": {"mode": "Dark"}}]}),
    );
    let shown = |h: &exec::SessionHost| h.ed.doc.get(node(&id)).unwrap().style.effects.clone();
    assert_eq!(shown(&h)[0].color, Color::rgba(0, 0, 0, 0x80));

    // Editing the style reaches the card, in one undo step.
    let mut effects = e2["effects"].clone();
    effects[0]["blur"] = json!(6);
    effects[1]["visible"] = json!(false);
    let r = call(
        &mut h,
        "set_effect_styles",
        json!({"styles": [
            {"name": "--elevation-2", "newName": "Elevation/Raised", "effects": effects},
            {"name": "Glass", "delete": true},
            {"name": "Nope", "delete": true},
        ]}),
    );
    assert_eq!(
        r["styles"][0],
        json!({"var": "--elevation-raised", "result": "updated"})
    );
    assert_eq!(r["styles"][1]["result"], "deleted");
    assert_eq!(r["styles"][2]["result"], "error");
    assert_eq!((shown(&h)[0].blur, shown(&h)[1].visible), (6.0, false));
    assert!(h.ed.doc.get(node(&id)).unwrap().effect_style.is_some());
    let css = call(&mut h, "get_tokens", json!({"types": ["effectStyle"]}));
    assert_eq!(
        css["effectStyles"][1]["css"], "box-shadow: 0px 2px 6px var(--shadow)",
        "a hidden effect leaves the CSS"
    );
    call(&mut h, "undo", json!({}));
    assert_eq!(shown(&h)[0].blur, 4.0);
    assert_eq!(
        call(&mut h, "get_tokens", json!({"types": ["effectStyle"]}))["effectStyles"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn a_single_effect_on_a_layer_can_be_hidden() {
    let store = store();
    let mut h = host(&store);
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--shade", "value": "#ff0000"}]}),
    );
    let id = card(
        &mut h,
        "box-shadow: 0 1px 2px var(--shade), 0 4px 8px #0000ff; filter: blur(3px)",
    );
    let info = call(&mut h, "get_node_info", json!({"nodeId": id}));
    let mut effects = info["effects"].clone();
    assert_eq!(effects.as_array().unwrap().len(), 3);
    assert_eq!(effects[0]["color"], "var(--shade)");
    assert_eq!(
        effects[2],
        json!({"type": "layer-blur", "blur": 3.0, "visible": true})
    );

    effects[1]["visible"] = json!(false);
    let r = call(
        &mut h,
        "set_effects",
        json!({"updates": [{"nodeIds": [id], "effects": effects}]}),
    );
    assert_eq!(r["updated"], 1);
    let n = h.ed.doc.get(node(&id)).unwrap();
    assert!(!n.style.effects[1].visible && n.style.effects[0].visible);
    assert!(
        n.tokens.contains_key(&TokenProp::EffectColor(0)),
        "the token stays bound"
    );
    let info = call(&mut h, "get_node_info", json!({"nodeId": id}));
    assert_eq!(info["effects"][1]["visible"], false);
    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [id]}));
    assert_eq!(
        st[id.as_str().unwrap()]["boxShadow"],
        "0px 1px 2px var(--shade)"
    );

    // Reorder and change kind, then clear.
    call(
        &mut h,
        "set_effects",
        json!({"updates": [{"nodeIds": [id], "effects": [
            {"type": "background-blur", "blur": 10},
            {"type": "inner-shadow", "x": 1, "y": 1, "blur": 0, "color": "var(--shade)"},
        ]}]}),
    );
    let n = h.ed.doc.get(node(&id)).unwrap();
    let kinds: Vec<EffectKind> = n.style.effects.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [EffectKind::BackgroundBlur, EffectKind::InnerShadow]);
    assert_eq!(n.style.effects[1].color, Color::rgb(255, 0, 0));
    assert!(n.tokens.contains_key(&TokenProp::EffectColor(1)));
    call(
        &mut h,
        "set_effects",
        json!({"updates": [{"nodeIds": [id], "effects": []}]}),
    );
    let info = call(&mut h, "get_node_info", json!({"nodeId": id}));
    assert!(info.get("effects").is_none());

    let bad = try_call(
        &mut h,
        "set_effects",
        json!({"updates": [{"nodeIds": [id], "effects": [{"type": "glow"}]}]}),
    );
    assert!(bad.unwrap_err().contains("effects[0]: Unknown effect type"));
}

#[test]
fn none_detaches_text_and_effect_styles() {
    let store = store();
    let mut h = host(&store);
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--shade", "value": "#00000040"}]}),
    );
    call(
        &mut h,
        "create_text_styles",
        json!({"styles": [{"name": "Heading/L", "fontSize": "32px", "fontWeight": 700}]}),
    );
    call(
        &mut h,
        "create_effect_styles",
        json!({"styles": [{"name": "Raised", "effects": [
            {"type": "drop-shadow", "y": 2, "blur": 4, "color": "var(--shade)"}]}]}),
    );
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Page", "styles": {"width": "400px", "height": "300px"}}),
    )["id"]
        .clone();
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children",
               "html": "<p class=\"heading-l\">Title</p><div style=\"width: 50px; height: 50px; box-shadow: var(--raised)\"></div>"}),
    );
    let (text, boxed) = (
        r["createdNodeIds"][0].clone(),
        r["createdNodeIds"][1].clone(),
    );
    assert!(h.ed.doc.get(node(&text)).unwrap().text_style_ref.is_some());

    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [
            {"nodeIds": [text], "styles": {"textStyle": "none"}},
            {"nodeIds": [boxed], "styles": {"effectStyle": "none", "textStyle": "none"}},
        ]}),
    );
    assert_eq!(
        r["ignoredStyles"],
        json!({boxed.as_str().unwrap(): ["textStyle"]})
    );
    let t = h.ed.doc.get(node(&text)).unwrap();
    assert!(t.text_style_ref.is_none());
    assert_eq!(
        t.kind.font_size(),
        Some(32.0),
        "the text keeps what it shows"
    );
    let b = h.ed.doc.get(node(&boxed)).unwrap();
    assert!(b.effect_style.is_none());
    assert_eq!(b.style.effects[0].y, 2.0);
    assert!(
        b.tokens.contains_key(&TokenProp::EffectColor(0)),
        "the color stays bound"
    );
    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [text]}));
    assert!(st[text.as_str().unwrap()].get("textStyle").is_none());
}

#[test]
fn the_new_tools_are_listed_with_schemas() {
    let defs = sketch_rs::mcp::tools::definitions();
    for name in [
        "create_effect_styles",
        "set_effect_styles",
        "set_effects",
        "create_modes",
        "set_modes",
    ] {
        let d = defs.iter().find(|d| d["name"] == name).expect(name);
        assert!(
            d["inputSchema"]["properties"]["fileId"].is_object(),
            "{name}"
        );
    }
    assert!(sketch_rs::mcp::tools::INSTRUCTIONS.contains("create_modes"));
    assert!(sketch_rs::mcp::tools::GUIDE_INSTRUCTIONS.contains("set_effects"));
}
