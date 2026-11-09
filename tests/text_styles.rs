use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{NodeId, StyleField};
use sketch_rs::project::Store;
use sketch_rs::session::Session;

fn store() -> Store {
    let dir = std::env::temp_dir().join(format!("sketch-rs-styles-{:016x}", fastrand::u64(..)));
    Store::open(dir).unwrap()
}

fn host(store: &Store, name: &str) -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store.clone());
    h.session.create(&mut h.ed, name).unwrap();
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

fn font_size(h: &exec::SessionHost, id: NodeId) -> f64 {
    h.ed.doc.get(id).unwrap().kind.font_size().unwrap()
}

fn seed(h: &mut exec::SessionHost) {
    call(
        h,
        "create_tokens",
        json!({"tokens": [
            {"type": "fontSize", "name": "--size-l", "value": "32px"},
            {"type": "fontFamily", "name": "--font-display", "value": "Lora"},
        ]}),
    );
    call(
        h,
        "create_text_styles",
        json!({"styles": [
            {"name": "Heading/L", "fontFamily": "var(--font-display)", "fontSize": "var(--size-l)",
             "fontWeight": 700, "lineHeight": "120%", "letterSpacing": "-0.02em",
             "description": "Page titles"},
            {"name": "Body", "fontSize": 16, "lineHeight": 1.5},
        ]}),
    );
}

fn artboard(h: &mut exec::SessionHost) -> Value {
    call(
        h,
        "create_artboard",
        json!({"name": "Page", "styles": {"width": "600px", "height": "400px"}}),
    )["id"]
        .clone()
}

#[test]
fn get_tokens_lists_styles_in_short_form() {
    let store = store();
    let mut h = host(&store, "Styles");
    seed(&mut h);
    let all = call(&mut h, "get_tokens", json!({}));
    let styles = all["textStyles"].as_array().unwrap();
    assert_eq!(styles.len(), 2);
    assert_eq!(styles[0]["class"], "heading-l");
    assert_eq!(
        styles[0]["css"],
        "font: 700 var(--size-l)/120% var(--font-display); letter-spacing: -0.02em"
    );
    assert_eq!(styles[0]["description"], "Page titles");
    let only = call(&mut h, "get_tokens", json!({"types": ["textStyle"]}));
    assert!(only["tokens"].as_array().unwrap().is_empty());
    assert_eq!(only["textStyles"].as_array().unwrap().len(), 2);
    let none = call(&mut h, "get_tokens", json!({"types": ["fontSize"]}));
    assert!(none.get("textStyles").is_none(), "{none}");
    let pattern = call(&mut h, "get_tokens", json!({"namePattern": ".head*"}));
    assert_eq!(pattern["textStyles"].as_array().unwrap().len(), 1);
    let css = call(&mut h, "get_tokens", json!({"format": "css"}));
    let css = css.as_str().unwrap();
    assert!(
        css.contains(".heading-l {\n  font-family: var(--font-display);"),
        "{css}"
    );
    assert!(css.contains(".body {"), "{css}");
}

#[test]
fn write_html_applies_a_style_by_class_and_inline_styles_override_it() {
    let store = store();
    let mut h = host(&store, "Styles");
    seed(&mut h);
    let ab = artboard(&mut h);
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children", "html":
            "<h1 class=\"heading-l\">Title</h1>\
             <p class=\"lead body\" style=\"font-size: 20px\">Lead</p>\
             <div class=\"heading-l\" style=\"padding: 8px; background: #eee\">Boxed</div>"}),
    );
    let ids = r["createdNodeIds"].as_array().unwrap();
    let title = node(&ids[0]);
    assert_eq!(font_size(&h, title), 32.0);
    let n = h.ed.doc.get(title).unwrap();
    assert_eq!(n.text_style.family.as_deref(), Some("Lora"));
    assert_eq!(n.text_style.base_weight(), 700);
    let r = n.text_style_ref.clone().expect("the title uses the style");
    assert!(r.overrides.is_empty(), "{r:?}");

    let lead = node(&ids[1]);
    assert_eq!(font_size(&h, lead), 20.0);
    let r = h.ed.doc.get(lead).unwrap().text_style_ref.clone().unwrap();
    assert_eq!(
        r.overrides.into_iter().collect::<Vec<_>>(),
        [StyleField::Size]
    );

    let boxed = h.ed.doc.get(node(&ids[2])).unwrap();
    let inner = boxed.children[0];
    assert!(h.ed.doc.get(inner).unwrap().text_style_ref.is_some());

    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [ids[1]]}));
    let st = &st[ids[1].as_str().unwrap()];
    assert_eq!(st["textStyle"], "body");
    assert_eq!(st["textStyleOverrides"], json!(["fontSize"]));
    let jsx = call(&mut h, "get_jsx", json!({"nodeId": ids[0]}));
    assert!(
        jsx.as_str().unwrap().contains("className=\"heading-l\""),
        "{jsx}"
    );
}

#[test]
fn update_styles_applies_and_style_edits_reach_every_layer_as_one_undo() {
    let store = store();
    let mut h = host(&store, "Styles");
    seed(&mut h);
    let ab = artboard(&mut h);
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children", "html": "<p>One</p><p>Two</p>"}),
    );
    let ids = r["createdNodeIds"].clone();
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": ids, "styles": {"textStyle": "heading-l"}}]}),
    );
    assert!(r.get("ignoredStyles").is_none(), "{r}");
    let (a, b) = (node(&ids[0]), node(&ids[1]));
    assert_eq!((font_size(&h, a), font_size(&h, b)), (32.0, 32.0));

    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ids[1]], "styles": {"textStyle": "heading-l", "fontSize": "20px"}}]}),
    );
    assert_eq!(font_size(&h, b), 20.0);
    let r = h.ed.doc.get(b).unwrap().text_style_ref.clone().unwrap();
    assert!(r.overrides.contains(&StyleField::Size), "{r:?}");
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ids[1]], "styles": {"textStyle": "heading-l"}}]}),
    );

    let r = call(
        &mut h,
        "set_text_styles",
        json!({"styles": [{"name": "heading-l", "fontSize": "48px", "newName": "Display"}]}),
    );
    assert_eq!(
        r["styles"][0],
        json!({"class": "display", "result": "updated"})
    );
    assert_eq!((font_size(&h, a), font_size(&h, b)), (48.0, 48.0));
    call(&mut h, "undo", json!({}));
    assert_eq!((font_size(&h, a), font_size(&h, b)), (32.0, 32.0));
    call(&mut h, "redo", json!({}));

    let r = call(
        &mut h,
        "set_text_styles",
        json!({"styles": [{"name": "display", "delete": true}]}),
    );
    assert_eq!(r["styles"][0]["result"], "deleted");
    assert_eq!(font_size(&h, a), 48.0);
}

#[test]
fn styles_save_with_the_project() {
    let store = store();
    let mut h = host(&store, "Styles");
    seed(&mut h);
    let ab = artboard(&mut h);
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children", "html": "<p class=\"body\">Hi</p>"}),
    );
    let id = node(&r["createdNodeIds"][0]);
    call(&mut h, "finish_working_on_nodes", json!({}));
    let dir = h.session.project.as_ref().unwrap().dir.clone();

    let mut s = Session::new(store.clone());
    let mut ed = sketch_rs::editor::Editor::default();
    s.open(&mut ed, &dir).unwrap();
    assert_eq!(ed.tokens.styles().len(), 2);
    let n = ed.doc.get(id).unwrap();
    assert!(ed.bound_style(n).is_some());
}

#[test]
fn design_systems_carry_styles_with_the_prefix() {
    let store = store();
    let mut src = host(&store, "Acme");
    seed(&mut src);
    let r = call(&mut src, "publish_design_system", json!({}));
    assert!(r.to_string().contains(".acme-heading-l"), "{r}");

    let mut h = host(&store, "Landing");
    let list = call(&mut h, "list_design_systems", json!({}));
    assert_eq!(list["systems"][0]["textStyles"], 2);
    let text = call(&mut h, "get_design_system", json!({"id": "acme"}));
    let text = text.as_str().unwrap();
    assert!(text.contains(".acme-heading-l"), "{text}");
    assert!(text.contains("var(--acme-size-l)"), "{text}");
    let json = call(
        &mut h,
        "get_design_system",
        json!({"id": "acme", "format": "json", "types": ["textStyle"]}),
    );
    assert!(json["tokens"].as_array().unwrap().is_empty());
    assert_eq!(json["textStyles"].as_array().unwrap().len(), 2);

    call(&mut h, "use_design_system", json!({"id": "acme"}));
    let ab = artboard(&mut h);
    let r = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children",
               "html": "<p class=\"acme-heading-l\">Hi</p>"}),
    );
    let id = node(&r["createdNodeIds"][0]);
    assert_eq!(font_size(&h, id), 32.0);
    let r = call(
        &mut h,
        "set_text_styles",
        json!({"styles": [{"name": "acme-heading-l", "fontSize": "10px"}]}),
    );
    assert_eq!(r["styles"][0]["result"], "error", "{r}");
    let own = call(&mut h, "get_tokens", json!({}));
    assert_eq!(own["designSystems"][0]["textStyles"], 2);
}
