//! Design systems over MCP: one project publishes its tokens, another uses them, and agents
//! load only the systems and tokens they ask for.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, NodeId, TokenProp};
use sketch_rs::project::Store;

fn store() -> Store {
    let dir = std::env::temp_dir().join(format!("sketch-rs-ds-{:016x}", fastrand::u64(..)));
    Store::open(dir).unwrap()
}

fn try_call(h: &mut exec::SessionHost, name: &str, args: Value) -> Result<String, String> {
    exec::call(h, name, &args).map(|outs| match outs.into_iter().next() {
        Some(Out::Text(t)) => t,
        _ => String::new(),
    })
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    let t = try_call(h, name, args).unwrap_or_else(|e| panic!("{name} failed: {e}"));
    serde_json::from_str(&t).unwrap_or(Value::String(t))
}

fn node(id: &Value) -> NodeId {
    let n = id.as_str().unwrap().rsplit(':').next().unwrap();
    NodeId(n.parse().unwrap())
}

/// A host with "Acme" open, publishing `--acme-primary`, `--acme-cta` (an alias) and
/// `--acme-space`.
fn published(store: &Store) -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store.clone());
    h.session.create(&mut h.ed, "Acme").unwrap();
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--primary", "value": "#2f6bff", "description": "Buttons"},
            {"type": "color", "name": "--cta", "value": "var(--primary)"},
            {"type": "spacing", "name": "--space", "value": "16px"},
        ]}),
    );
    let r = call(
        &mut h,
        "publish_design_system",
        json!({"description": "Acme's palette", "notes": "Use --acme-cta for main actions."}),
    );
    assert_eq!(r["prefix"], "--acme-");
    assert_eq!(r["version"], 1);
    assert_eq!(r["added"].as_array().unwrap().len(), 3);
    h
}

fn consumer(store: &Store) -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store.clone());
    h.session.create(&mut h.ed, "Landing").unwrap();
    h
}

#[test]
fn agents_list_systems_in_short_and_load_only_what_they_ask_for() {
    let store = store();
    published(&store);
    let mut h = consumer(&store);

    let list = call(&mut h, "list_design_systems", json!({}));
    let s = &list["systems"][0];
    assert_eq!(s["name"], "Acme");
    assert_eq!(s["tokens"], json!({"color": 2, "spacing": 1}));
    assert!(s.get("usedByOpenFile").is_none());
    assert!(
        !list.to_string().contains("2f6bff"),
        "no token values: {list}"
    );

    let full = call(&mut h, "get_design_system", json!({"id": "acme"}));
    let full = full.as_str().unwrap();
    assert!(full.contains("Use --acme-cta for main actions."), "{full}");
    assert!(
        full.contains("--acme-primary: #2f6bff; /* Buttons */"),
        "{full}"
    );
    assert!(full.contains("--acme-cta: var(--acme-primary);"), "{full}");
    assert!(full.contains("use_design_system"), "{full}");

    let colors = call(
        &mut h,
        "get_design_system",
        json!({"id": "Acme", "types": ["spacing"], "format": "json"}),
    );
    assert_eq!(colors["tokens"].as_array().unwrap().len(), 1);
    assert_eq!(colors["tokens"][0]["name"], "--acme-space");
}

#[test]
fn a_file_binds_a_system_only_after_using_it() {
    let store = store();
    published(&store);
    let mut h = consumer(&store);
    let styles = json!({"name": "Card", "styles": {
        "width": "200px", "height": "100px", "backgroundColor": "var(--acme-cta)",
    }});
    let err = try_call(&mut h, "create_artboard", styles.clone()).unwrap_err();
    assert!(err.contains("use_design_system"), "{err}");

    let used = call(&mut h, "use_design_system", json!({"id": "--acme-"}));
    assert_eq!(used["prefix"], "--acme-");
    let r = call(&mut h, "create_artboard", styles);
    let n = h.ed.doc.get(node(&r["id"])).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x2f, 0x6b, 0xff)));
    assert!(n.tokens.contains_key(&TokenProp::Fill));

    // get_tokens keeps the system's tokens out unless asked.
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--hero", "value": "var(--acme-primary)"}]}),
    );
    let own = call(&mut h, "get_tokens", json!({}));
    assert_eq!(own["tokens"].as_array().unwrap().len(), 1);
    assert_eq!(own["designSystems"][0]["tokens"], 3);
    assert_eq!(own["designSystems"][0]["sourceAvailable"], true);
    let sys = call(&mut h, "get_tokens", json!({"system": "Acme"}));
    assert_eq!(sys["tokens"].as_array().unwrap().len(), 3);
    let info = call(&mut h, "get_basic_info", json!({}));
    assert_eq!(info["tokens"].as_array().unwrap().len(), 1);
    assert_eq!(info["designSystems"][0]["name"], "Acme");

    let err = call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--acme-primary", "delete": true}]}),
    );
    assert_eq!(err["tokens"][0]["result"], "error", "{err}");

    let detached = call(
        &mut h,
        "remove_design_system",
        json!({"id": "acme", "detach": true}),
    );
    assert_eq!(detached["result"], "detached");
    let all = call(&mut h, "get_tokens", json!({}));
    assert_eq!(all["tokens"].as_array().unwrap().len(), 4);
    assert!(all.get("designSystems").is_none());
}

#[test]
fn publishing_again_reports_changes_and_files_update_when_opened() {
    let store = store();
    let mut src = published(&store);
    let mut app = consumer(&store);
    call(&mut app, "use_design_system", json!({"id": "acme"}));
    call(&mut app, "finish_working_on_nodes", json!({}));
    let dir = app.session.project.as_ref().unwrap().dir.clone();

    call(
        &mut src,
        "set_tokens",
        json!({"tokens": [
            {"name": "--primary", "value": "#000000"},
            {"name": "--space", "delete": true},
        ]}),
    );
    let err = try_call(
        &mut src,
        "publish_design_system",
        json!({"prefix": "other"}),
    )
    .unwrap_err();
    assert!(err.contains("fixed"), "{err}");
    let r = call(&mut src, "publish_design_system", json!({}));
    assert_eq!(r["version"], 2);
    assert_eq!(r["changed"], json!(["--acme-primary"]));
    assert_eq!(r["removed"], json!(["--acme-space"]));
    assert_eq!(r["filesToUpdate"], 1);

    let mut reopened = exec::SessionHost::new(store.clone());
    reopened.session.open(&mut reopened.ed, &dir).unwrap();
    let t = call(&mut reopened, "get_tokens", json!({"system": "acme"}));
    let names: Vec<&str> = t["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["--acme-primary", "--acme-cta"]);
    assert_eq!(t["tokens"][0]["value"], "#000000");
}

#[test]
fn large_systems_return_counts_until_filtered() {
    let store = store();
    let mut h = exec::SessionHost::new(store.clone());
    h.session.create(&mut h.ed, "Big").unwrap();
    let tokens: Vec<Value> = (0..200)
        .map(|i| json!({"type": "spacing", "name": format!("--s-{i}"), "value": format!("{i}px")}))
        .collect();
    call(&mut h, "create_tokens", json!({ "tokens": tokens }));
    call(&mut h, "publish_design_system", json!({}));
    let full = call(&mut h, "get_design_system", json!({"id": "big"}));
    let full = full.as_str().unwrap();
    assert!(full.contains("200 tokens"), "{full}");
    assert!(!full.contains("--big-s-1:"), "{full}");
    for format in ["css", "tailwind"] {
        let css = call(
            &mut h,
            "get_design_system",
            json!({"id": "big", "format": format}),
        );
        let css = css.as_str().unwrap();
        assert!(css.contains("200 tokens"), "{css}");
        assert!(!css.contains("--big-s-1:"), "{css}");
    }
    let some = call(
        &mut h,
        "get_design_system",
        json!({"id": "big", "namePattern": "--big-s-1*"}),
    );
    assert_eq!(some.as_str().unwrap().matches("px;").count(), 111);
}

#[test]
fn border_width_tokens_publish_and_bind_in_other_files() {
    let store = store();
    let mut src = exec::SessionHost::new(store.clone());
    src.session.create(&mut src.ed, "Acme").unwrap();
    call(
        &mut src,
        "create_tokens",
        json!({"tokens": [{"type": "borderWidth", "name": "--hairline", "value": "1px"}]}),
    );
    call(&mut src, "publish_design_system", json!({}));

    let mut app = consumer(&store);
    let list = call(&mut app, "list_design_systems", json!({}));
    assert_eq!(list["systems"][0]["tokens"], json!({"borderWidth": 1}));
    call(&mut app, "use_design_system", json!({"id": "acme"}));
    let css = call(&mut app, "get_design_system", json!({"id": "acme"}));
    assert!(
        css.as_str().unwrap().contains("--acme-hairline: 1px;"),
        "{css}"
    );
    let r = call(
        &mut app,
        "create_artboard",
        json!({"name": "Card", "styles": {
            "width": "200px", "height": "100px",
            "border": "4px solid #000000", "borderWidth": "var(--acme-hairline)",
        }}),
    );
    let id = node(&r["id"]);
    let n = app.ed.doc.get(id).unwrap();
    assert_eq!(n.style.stroke_width, 1.0);
    assert!(n.tokens.contains_key(&TokenProp::StrokeWidth));
    call(&mut app, "finish_working_on_nodes", json!({}));
    let dir = app.session.project.as_ref().unwrap().dir.clone();

    call(
        &mut src,
        "set_tokens",
        json!({"tokens": [{"name": "--hairline", "value": "0.5px"}]}),
    );
    let r = call(&mut src, "publish_design_system", json!({}));
    assert_eq!(r["changed"], json!(["--acme-hairline"]));

    let mut reopened = exec::SessionHost::new(store.clone());
    reopened.session.open(&mut reopened.ed, &dir).unwrap();
    let n = reopened.ed.doc.get(id).unwrap();
    assert_eq!(
        n.style.stroke_width, 0.5,
        "the new version reaches the layer"
    );
    assert!(n.tokens.contains_key(&TokenProp::StrokeWidth));
}

#[test]
fn modes_publish_and_frames_pick_them_by_name() {
    let store = store();
    let mut src = exec::SessionHost::new(store.clone());
    src.session.create(&mut src.ed, "Acme").unwrap();
    call(
        &mut src,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--primary", "value": "#2f6bff", "modes": {"Dark": "#9ab8ff"}}]}),
    );
    call(&mut src, "publish_design_system", json!({}));

    let mut h = consumer(&store);
    call(&mut h, "use_design_system", json!({"id": "acme"}));
    let text = call(&mut h, "get_design_system", json!({"id": "acme"}));
    assert!(
        text.as_str().unwrap().contains("/* Dark: #9ab8ff */"),
        "{text}"
    );
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": "root", "mode": "insert-children",
            "html": "<div data-theme=\"dark\" style=\"background-color: var(--acme-primary); width: 10px; height: 10px\"></div>",
        }),
    );
    let n = h.ed.doc.get(node(&r["createdNodeIds"][0])).unwrap();
    assert_eq!(n.style.fill(), Some(Color::rgb(0x9a, 0xb8, 0xff)));
    assert!(n.tokens.contains_key(&TokenProp::Fill));
}

#[test]
fn system_css_starts_with_the_systems_first_mode() {
    let store = store();
    let mut src = exec::SessionHost::new(store.clone());
    src.session.create(&mut src.ed, "Acme").unwrap();
    src.ed.edit_tokens("Dark first", None, |t| {
        t.delete_mode("light").unwrap();
        let light = t.add_mode();
        t.rename_mode(&light, "Light").unwrap();
    });
    call(
        &mut src,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--bg", "value": "#000000", "modes": {"Light": "#ffffff"}}]}),
    );
    call(&mut src, "publish_design_system", json!({}));
    let css = call(
        &mut src,
        "get_design_system",
        json!({"id": "acme", "format": "css"}),
    );
    let root = css.as_str().unwrap().split("}").next().unwrap();
    assert!(root.contains("--acme-bg: #000000;"), "{css}");
}
