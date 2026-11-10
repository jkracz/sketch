//! Design tokens end to end: agents drive them over the MCP tools, a project on disk keeps
//! them, and the app shows them.

mod common;
use common::*;
use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, LetterSpacing, NodeId, NodeKind, TokenProp};
use sketch_rs::project::Store;
use sketch_rs::render::frame;
use sketch_rs::ui::{SidebarTab, UiState};

fn host() -> exec::SessionHost {
    let dir = std::env::temp_dir().join(format!("sketch-rs-tokens-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(dir).unwrap());
    h.session.create(&mut h.ed, "Tokens").unwrap();
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

fn node(id: &Value) -> NodeId {
    let n = id.as_str().unwrap().rsplit(':').next().unwrap();
    NodeId(n.parse().unwrap())
}

fn seed(h: &mut exec::SessionHost) {
    let r = call(
        h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--color-surface", "value": "#ffffff", "description": "Cards"},
            {"type": "color", "name": "--color-brand", "value": "#2f6bff"},
            {"type": "color", "name": "--color-action", "value": "var(--color-brand)"},
            {"type": "spacing", "name": "--space-md", "value": "16px"},
            {"type": "radius", "name": "--radius-lg", "value": "12px"},
            {"type": "opacity", "name": "--muted", "value": 0.123456},
            {"type": "fontSize", "name": "--text-title", "value": "24px"},
        ]}),
    );
    assert_eq!(r["count"], 7);
    assert!(
        r["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["result"] == "created"),
        "{r}"
    );
}

fn artboard(h: &mut exec::SessionHost) -> Value {
    let r = call(
        h,
        "create_artboard",
        json!({"name": "Card", "styles": {
            "width": "360px", "height": "240px",
            "backgroundColor": "var(--color-surface)",
            "padding": "var(--space-md)", "gap": "var(--space-md)",
            "borderRadius": "var(--radius-lg)",
        }}),
    );
    r["id"].clone()
}

#[test]
fn get_tokens_lists_filters_and_formats() {
    let mut h = host();
    seed(&mut h);
    let all = call(&mut h, "get_tokens", json!({}));
    let list = all["tokens"].as_array().unwrap();
    assert_eq!(list.len(), 7);
    assert_eq!(list[0]["name"], "--color-surface");
    assert_eq!(list[0]["description"], "Cards");
    assert_eq!(
        list[2]["value"], "var(--color-brand)",
        "aliases stay aliases"
    );

    let colors = call(&mut h, "get_tokens", json!({"types": ["color"]}));
    assert_eq!(colors["tokens"].as_array().unwrap().len(), 3);
    let by_name = call(&mut h, "get_tokens", json!({"namePattern": "--COLOR-*"}));
    assert_eq!(by_name["tokens"].as_array().unwrap().len(), 3);
    let both = call(
        &mut h,
        "get_tokens",
        json!({"namePattern": "*space*", "types": ["color"]}),
    );
    assert_eq!(both["tokens"].as_array().unwrap().len(), 0);

    let css = call(&mut h, "get_tokens", json!({"format": "css"}));
    let css = css.as_str().unwrap();
    assert!(css.starts_with(":root {"), "{css}");
    assert!(css.contains("--color-action: var(--color-brand);"), "{css}");
    let tw = call(&mut h, "get_tokens", json!({"format": "tailwind"}));
    assert!(tw.as_str().unwrap().starts_with("@theme {"), "{tw}");

    let info = call(&mut h, "get_basic_info", json!({}));
    assert_eq!(info["tokens"].as_array().unwrap().len(), 7);
}

#[test]
fn bad_entries_fail_alone() {
    let mut h = host();
    seed(&mut h);
    let r = call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--bad", "value": "not-a-color"},
            {"type": "color", "name": "--good", "value": "#000000"},
            {"type": "spacing", "name": "--x", "value": "var(--color-brand)"},
            {"type": "nonsense", "name": "--y", "value": "1"},
        ]}),
    );
    let t = r["tokens"].as_array().unwrap();
    assert_eq!(t[0]["result"], "error");
    assert_eq!(t[1]["result"], "created");
    assert_eq!(t[2]["result"], "error");
    assert_eq!(t[3]["result"], "error");

    let r = call(
        &mut h,
        "set_tokens",
        json!({"tokens": [
            {"name": "--color-brand", "value": "var(--color-action)"},
            {"name": "--missing", "value": "#000"},
        ]}),
    );
    let t = r["tokens"].as_array().unwrap();
    assert!(
        t[0]["message"].as_str().unwrap().contains("Circular"),
        "{r}"
    );
    assert!(
        t[1]["message"].as_str().unwrap().contains("not found"),
        "{r}"
    );
}

#[test]
fn var_binds_and_set_tokens_reaches_every_page() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    let id = node(&ab);
    call(&mut h, "create_page", json!({"name": "Two"}));
    let pages = call(&mut h, "get_basic_info", json!({}));
    let second = pages["pages"][1]["id"].clone();
    let other = call(
        &mut h,
        "create_artboard",
        json!({"name": "Other", "pageId": second, "styles": {
            "width": "100px", "height": "100px", "backgroundColor": "var(--color-surface)",
        }}),
    );

    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::WHITE));
    for prop in [
        TokenProp::Fill,
        TokenProp::Padding,
        TokenProp::Gap,
        TokenProp::Radius,
    ] {
        assert!(n.tokens.contains_key(&prop), "{prop:?}: {:?}", n.tokens);
    }

    let styles = call(&mut h, "get_computed_styles", json!({"nodeIds": [ab]}));
    let s = styles.as_object().unwrap().values().next().unwrap();
    assert_eq!(s["backgroundColor"], "var(--color-surface)", "{s}");
    assert_eq!(s["borderRadius"], "var(--radius-lg)", "{s}");

    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [
            {"name": "--color-surface", "value": "#101014", "newName": "--color-card"},
            {"name": "--radius-lg", "value": "20px"},
        ]}),
    );
    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x10, 0x10, 0x14)));
    assert_eq!(n.style.corner_radius, 20.0);
    let parked = h.ed.page_doc(1).get(node(&other["id"])).unwrap();
    assert_eq!(parked.style.fill(), Some(Color::rgb(0x10, 0x10, 0x14)));

    let styles = call(&mut h, "get_computed_styles", json!({"nodeIds": [ab]}));
    let s = styles.as_object().unwrap().values().next().unwrap();
    assert_eq!(
        s["backgroundColor"], "var(--color-card)",
        "renames stay bound"
    );
}

#[test]
fn write_html_binds_text_and_boxes() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": ab, "mode": "insert-children",
            "html": "<div style=\"background-color: var(--color-action); padding: var(--space-md); border: 2px solid var(--color-brand); opacity: var(--muted)\">\
                     <div style=\"color: var(--color-surface); font-size: var(--text-title)\">Save</div></div>",
        }),
    );
    assert_eq!(r["createdNodeIds"].as_array().unwrap().len(), 1, "{r}");
    let doc = &h.ed.doc;
    let text = doc
        .nodes
        .values()
        .find(|n| matches!(n.kind, NodeKind::Text { .. }))
        .unwrap();
    assert_eq!(text.kind.font_size(), Some(24.0));
    assert!(
        text.tokens.contains_key(&TokenProp::Fill),
        "{:?}",
        text.tokens
    );
    assert!(text.tokens.contains_key(&TokenProp::FontSize));
    let boxed = doc.get(text.parent.unwrap()).unwrap();
    assert_eq!(boxed.style.fill(), Some(Color::rgb(0x2f, 0x6b, 0xff)));
    assert!(boxed.tokens.contains_key(&TokenProp::Fill));
    assert_eq!(boxed.style.stroke, Some(Color::rgb(0x2f, 0x6b, 0xff)));
    assert!(
        !boxed.tokens.contains_key(&TokenProp::Stroke),
        "part of a shorthand doesn't bind"
    );
    assert_eq!(boxed.style.opacity, 0.123456, "full precision survives CSS");
    assert!(
        boxed.tokens.contains_key(&TokenProp::Opacity),
        "{:?}",
        boxed.tokens
    );
}

#[test]
fn inline_spans_get_token_values_without_binding() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": ab, "mode": "insert-children",
            "html": "<p style=\"color: #000000\">Plain <span style=\"color: var(--color-brand); font-size: var(--text-title)\">bright</span></p>",
        }),
    );
    let text =
        h.ed.doc
            .nodes
            .values()
            .find(|n| matches!(n.kind, NodeKind::Text { .. }))
            .unwrap();
    let span = &text.text_style.spans[0];
    assert_eq!(span.color, Some(Color::rgb(0x2f, 0x6b, 0xff)), "{span:?}");
    assert_eq!(span.size, Some(24.0), "{span:?}");
    assert!(!text.tokens.contains_key(&TokenProp::FontSize));
}

#[test]
fn update_styles_with_a_plain_value_detaches() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    let id = node(&ab);
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ab], "styles": {"backgroundColor": "#123456"}}]}),
    );
    let n = h.ed.doc.get(id).unwrap();
    assert!(!n.tokens.contains_key(&TokenProp::Fill));
    assert!(
        n.tokens.contains_key(&TokenProp::Padding),
        "others stay bound"
    );
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--color-surface", "value": "#000000"}]}),
    );
    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x12, 0x34, 0x56)));
}

#[test]
fn deleting_keeps_values_and_flattens_aliases() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    let id = node(&ab);
    let r = call(
        &mut h,
        "set_tokens",
        json!({"tokens": [
            {"name": "--color-brand", "delete": true},
            {"name": "--color-surface", "delete": true},
        ]}),
    );
    assert_eq!(r["tokens"][0]["result"], "deleted");
    assert_eq!(h.ed.doc.get(id).unwrap().style.fill(), Some(Color::WHITE));
    let left = call(&mut h, "get_tokens", json!({"types": ["color"]}));
    assert_eq!(left["tokens"][0]["name"], "--color-action");
    assert_eq!(left["tokens"][0]["value"], "#2f6bff");
}

#[test]
fn one_call_is_one_undo_step() {
    let mut h = host();
    seed(&mut h);
    call(&mut h, "undo", json!({}));
    assert!(h.ed.tokens.is_empty());
    call(&mut h, "redo", json!({}));
    assert_eq!(h.ed.tokens.len(), 7);
}

#[test]
fn tokens_and_bindings_survive_closing_the_project() {
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    let id = node(&ab);
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();

    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    assert_eq!(again.ed.tokens.len(), 7);
    assert_eq!(again.ed.tokens, h.ed.tokens);
    call(
        &mut again,
        "set_tokens",
        json!({"tokens": [{"name": "--color-surface", "value": "#ff0000"}]}),
    );
    let n = again.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(255, 0, 0)), "still bound");

    // Token-only changes are saved too, including to pages that are not open.
    again.session.save(&mut again.ed).unwrap();
    let mut third = exec::SessionHost::new(h.session.store.clone());
    third.session.open(&mut third.ed, &dir).unwrap();
    let n = third.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(255, 0, 0)));
}

/// The MCP id of a 32px text node bound to `--tracking-tight`, which is -0.025em.
fn tracked_text(h: &mut exec::SessionHost) -> Value {
    call(
        h,
        "create_tokens",
        json!({"tokens": [{"name": "--tracking-tight", "type": "letterSpacing", "value": "-0.025em"}]}),
    );
    let ab = artboard(h);
    call(
        h,
        "write_html",
        json!({
            "targetNodeId": ab, "mode": "insert-children",
            "html": "<div style=\"font-size: 32px; letter-spacing: var(--tracking-tight)\">Heading</div>",
        }),
    );
    let text = (h.ed.doc.nodes.values())
        .find(|n| matches!(n.kind, NodeKind::Text { .. }))
        .unwrap();
    let (page, _) = ab.as_str().unwrap().rsplit_once(':').unwrap();
    json!(format!("{page}:{}", text.id.0))
}

#[test]
fn em_tracking_tokens_stay_bound_when_the_font_size_changes() {
    let mut h = host();
    let text = tracked_text(&mut h);
    let id = node(&text);

    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [text], "styles": {"fontSize": "16px"}}]}),
    );
    let n = h.ed.doc.get(id).unwrap();
    assert!(n.tokens.contains_key(&TokenProp::LetterSpacing));
    assert_eq!(
        n.text_style.letter_spacing,
        LetterSpacing::Em { em: -0.025 }
    );

    let listed = call(&mut h, "get_tokens", json!({}));
    assert!(listed.to_string().contains("-0.025em"), "{listed}");
    let styles = call(&mut h, "get_computed_styles", json!({"nodeIds": [text]}));
    let s = styles.as_object().unwrap().values().next().unwrap();
    assert_eq!(s["letterSpacing"], "var(--tracking-tight)", "{s}");

    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--tracking-tight", "value": "2px"}]}),
    );
    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(n.text_style.letter_spacing, LetterSpacing::Px(2.0));
    assert!(n.tokens.contains_key(&TokenProp::LetterSpacing));
}

#[test]
fn em_tracking_survives_closing_the_project() {
    let mut h = host();
    let id = node(&tracked_text(&mut h));
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();

    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    assert_eq!(again.ed.tokens, h.ed.tokens);
    let n = again.ed.doc.get(id).unwrap();
    assert_eq!(
        n.text_style.letter_spacing,
        LetterSpacing::Em { em: -0.025 }
    );
    assert!(n.tokens.contains_key(&TokenProp::LetterSpacing));
}

#[test]
fn projects_without_tokens_keep_their_file_shape() {
    let mut h = host();
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();
    let manifest = std::fs::read_to_string(dir.join("project.json")).unwrap();
    assert!(!manifest.contains("tokens"), "{manifest}");
}

#[test]
fn an_oversized_token_cannot_make_a_project_unreadable() {
    let mut h = host();
    let r = call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "spacing", "name": "--huge", "value": "1e308rem"}]}),
    );
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();
    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    assert_eq!(r["tokens"][0]["result"], "error", "{r}");
}

#[test]
fn undoing_a_delete_keeps_a_rename_made_on_another_page() {
    let mut h = host();
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "spacing", "name": "--a", "value": "4px"},
            {"type": "spacing", "name": "--b", "value": "8px"},
        ]}),
    );
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--a", "delete": true}]}),
    );
    h.ed.add_page();
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--b", "newName": "--renamed"}]}),
    );
    h.ed.switch_page(0);
    h.ed.undo();
    assert!(
        h.ed.tokens.by_css_name("--a").is_some(),
        "the delete is undone"
    );
    assert!(
        h.ed.tokens.by_css_name("--renamed").is_some(),
        "the other page's rename stays"
    );
}

#[test]
fn sticky_fills_bind_through_the_background() {
    let mut h = host();
    seed(&mut h);
    let r = call(
        &mut h,
        "create_shape",
        json!({"type": "sticky", "text": "Note", "x": 0, "y": 0, "styles": {
            "backgroundColor": "var(--color-brand)",
        }}),
    );
    let id = node(&r["id"]);
    let n = h.ed.doc.get(id).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x2f, 0x6b, 0xff)));
    assert!(n.tokens.contains_key(&TokenProp::Fill), "{:?}", n.tokens);
    let styles = call(&mut h, "get_computed_styles", json!({"nodeIds": [r["id"]]}));
    let s = styles.as_object().unwrap().values().next().unwrap();
    assert_eq!(s["backgroundColor"], "var(--color-brand)", "{s}");

    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--color-brand", "value": "#000000"}]}),
    );
    assert_eq!(h.ed.doc.get(id).unwrap().style.fill(), Some(Color::BLACK));
}

#[test]
fn m6_tokens_panel_screenshot() {
    let Some(g) = gpu() else { return };
    let mut h = host();
    seed(&mut h);
    let ab = artboard(&mut h);
    call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": ab, "mode": "insert-children",
            "html": "<div style=\"font-size: var(--text-title); color: #1c1c1e\">Monthly report</div>\
                     <div style=\"background-color: var(--color-action); padding: var(--space-md); border-radius: var(--radius-lg)\">\
                     <span style=\"color: var(--color-surface)\">Download</span></div>",
        }),
    );
    let id = node(&ab);
    let button =
        h.ed.doc
            .nodes
            .values()
            .find(|n| n.parent == Some(id) && n.is_frame())
            .map(|n| n.id)
            .unwrap();
    h.ed.selection = vec![button];
    let mut st = UiState {
        project_name: "Tokens".into(),
        save_state: "Saved".into(),
        sidebar: SidebarTab::Tokens,
        ..Default::default()
    };
    let action =
        h.ed.tokens
            .by_css_name("--color-action")
            .unwrap()
            .id
            .clone();
    st.tokens.open(&h.ed, &action);
    let img = frame::capture(g, &mut h.ed, &mut st, [1280.0, 800.0], 2.0, |ed| {
        let b = ed.doc.content_bounds().unwrap();
        ed.camera.fit(&b, ed.viewport, 120.0, 1.0);
    });
    img.save(shot("milestone6_tokens.png")).unwrap();
}

#[test]
fn modes_reach_layers_stylesheets_and_agents() {
    let mut h = host();
    let r = call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--bg", "value": "#ffffff", "modes": {"Dark": "#111111"}},
            {"type": "color", "name": "--card", "value": "var(--bg)"},
            {"type": "color", "name": "--x", "value": "#000000", "modes": {"Dusk": "#000000"}},
        ]}),
    );
    assert_eq!(r["tokens"][1]["result"], "created", "{r}");
    assert!(
        r["tokens"][2]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown mode"),
        "{r}"
    );

    let list = call(&mut h, "get_tokens", json!({}));
    assert_eq!(list["modes"], json!(["Light", "Dark"]));
    assert_eq!(list["tokens"][0]["modes"], json!({"Dark": "#111111"}));
    assert!(list["tokens"][1].get("modes").is_none(), "{list}");
    let css = call(&mut h, "get_tokens", json!({"format": "css"}));
    let dark = css
        .as_str()
        .unwrap()
        .split_once("[data-theme=\"dark\"]")
        .unwrap()
        .1;
    assert!(
        dark.contains("--bg: #111111;") && dark.contains("--card: var(--bg);"),
        "{css}"
    );

    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": "root", "mode": "insert-children",
            "html": "<div data-theme=\"dark\" style=\"background-color: var(--bg); width: 100px; height: 100px\">\
                     <div style=\"background-color: var(--card); border: 1px solid var(--bg); width: 10px; height: 10px\"></div></div>",
        }),
    );
    let frame = node(&r["createdNodeIds"][0]);
    let child = h.ed.doc.get(frame).unwrap().children[0];
    let dark = Color::rgb(0x11, 0x11, 0x11);
    let n = h.ed.doc.get(child).unwrap();
    assert_eq!(n.style.fill(), Some(dark));
    assert_eq!(
        n.style.stroke,
        Some(dark),
        "unbound var() takes the mode too"
    );
    assert!(n.tokens.contains_key(&TokenProp::Fill));

    let set_mode = |h: &mut exec::SessionHost, mode: &str| {
        call(
            h,
            "update_styles",
            json!({"updates": [{"nodeIds": [r["createdNodeIds"][0]], "styles": {"mode": mode}}]}),
        )
    };
    set_mode(&mut h, "Light");
    assert_eq!(
        h.ed.doc.get(child).unwrap().style.fill(),
        Some(Color::WHITE)
    );
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--bg", "modes": {"Dark": "#222222"}}]}),
    );
    set_mode(&mut h, "dark");
    let n = h.ed.doc.get(child).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x22, 0x22, 0x22)));
    assert!(
        n.tokens.contains_key(&TokenProp::Fill),
        "switching modes keeps bindings"
    );
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    h.session.save(&mut h.ed).unwrap();
    let mut reopened = exec::SessionHost::new(h.session.store.clone());
    reopened.session.open(&mut reopened.ed, &dir).unwrap();
    h = reopened;
    let bg = h.ed.tokens.by_css_name("--bg").unwrap();
    assert_eq!(
        h.ed.tokens.resolve_in(&bg.id, Some("Dark")),
        Some(sketch_rs::model::TokenValue::Color(Color::rgb(
            0x22, 0x22, 0x22
        )))
    );
    let jsx = call(&mut h, "get_jsx", json!({"nodeId": r["createdNodeIds"][0]}));
    assert!(jsx.to_string().contains("data-theme=\\\"dark\\\""), "{jsx}");
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--bg", "modes": {"Dark": null}}]}),
    );
    assert_eq!(
        h.ed.doc.get(child).unwrap().style.fill(),
        Some(Color::WHITE)
    );
}

#[test]
fn full_documents_without_data_theme_write_without_warnings() {
    let mut h = host();
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": "root", "mode": "insert-children",
            "html": "<html><body><div style=\"width: 10px; height: 10px\"></div></body></html>",
        }),
    );
    assert!(r.get("warnings").is_none(), "{r}");
}

#[test]
fn shapes_and_instances_keep_their_frames_mode() {
    let mut h = host();
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--bg", "value": "#ffffff", "modes": {"Dark": "#111111"}}]}),
    );
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": "root", "mode": "insert-children",
            "html": "<div data-theme=\"dark\" style=\"width: 200px; height: 200px\"></div>\
                     <div style=\"background-color: var(--bg); width: 50px; height: 50px\"></div>",
        }),
    );
    let key = |id: NodeId| {
        let page = r["createdNodeIds"][0]
            .as_str()
            .unwrap()
            .rsplit_once(':')
            .unwrap()
            .0;
        format!("{page}:{}", id.0)
    };
    let shape = call(
        &mut h,
        "create_shape",
        json!({"type": "rectangle", "parentId": r["createdNodeIds"][0], "width": 10, "height": 10,
               "styles": {"border": "1px solid var(--bg)"}}),
    );
    let dark = Color::rgb(0x11, 0x11, 0x11);
    let stroke = h.ed.doc.get(node(&shape["id"])).unwrap().style.stroke;
    assert_eq!(stroke, Some(dark), "{shape}");

    h.ed.selection = vec![node(&r["createdNodeIds"][1])];
    h.ed.create_component();
    let main = h.ed.selection[0];
    h.ed.instance_of_selection();
    let inst = h.ed.selection[0];
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(inst)], "styles": {"mode": "Dark"}}]}),
    );
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(main)], "styles": {"opacity": 0.5}}]}),
    );
    assert_eq!(h.ed.doc.get(inst).unwrap().mode.as_deref(), Some("dark"));
    assert_eq!(h.ed.doc.get(inst).unwrap().style.fill(), Some(dark));
    // The main shows white in Light; setting white by hand still detaches.
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [key(inst)], "styles": {"backgroundColor": "#ffffff"}}]}),
    );
    let n = h.ed.doc.get(inst).unwrap();
    assert_eq!(n.style.fill(), Some(Color::WHITE));
    assert!(n.tokens.is_empty());
}
