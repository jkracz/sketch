//! An agent builds a component library over MCP: mains, local instances on two pages,
//! overrides, reset, detach, then publish.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, NodeId};
use sketch_rs::project::Store;

fn host() -> exec::SessionHost {
    let s = Store::open(
        std::env::temp_dir().join(format!("sketch-rs-mcp-comp-{:016x}", fastrand::u64(..))),
    )
    .unwrap();
    let mut h = exec::SessionHost::new(s);
    h.session.create(&mut h.ed, "Kit").unwrap();
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

fn write(h: &mut exec::SessionHost, target: &str, html: &str) -> Value {
    call(
        h,
        "write_html",
        json!({"targetNodeId": target, "mode": "insert-children", "html": html}),
    )
}

const BUTTON: &str = "<div layer-name=\"Button\" style=\"display:flex;padding:8px 16px;\
    background:#2563eb;border-radius:8px\"><p style=\"color:#ffffff\">Label</p></div>";

/// A Button/Primary main and an artboard to place instances in.
fn kit(h: &mut exec::SessionHost) -> (String, String) {
    let b = write(h, "root", BUTTON);
    let frame = b["createdNodeIds"][0].as_str().unwrap().to_string();
    let c = call(
        h,
        "create_component",
        json!({"nodeIds": [frame], "name": "Button/Primary"}),
    );
    assert_eq!(
        c["componentId"],
        frame.as_str(),
        "a frame converts in place"
    );
    assert_eq!(
        (c["name"].as_str(), c["wrapped"].as_bool()),
        (Some("Button/Primary"), Some(false))
    );
    let ab = call(
        h,
        "create_artboard",
        json!({"name": "Screen", "styles": {"width": "400px", "height": "300px"}}),
    );
    (frame, ab["id"].as_str().unwrap().to_string())
}

fn text_of(h: &exec::SessionHost, page: usize, id: &Value) -> String {
    h.ed.page_doc(page)
        .get(num(id))
        .unwrap()
        .kind
        .text()
        .unwrap()
        .to_string()
}

#[test]
fn an_agent_builds_and_publishes_a_component_library() {
    let mut h = host();
    let (main, artboard) = kit(&mut h);

    let info = call(&mut h, "get_basic_info", json!({}));
    assert_eq!(info["components"][0]["id"], main.as_str(), "{info}");
    assert_eq!(info["components"][0]["name"], "Button/Primary");

    // A local instance, by name, inside a flex artboard.
    let w = write(
        &mut h,
        &artboard,
        "<x-sketch-instance component=\"button/primary\"/>",
    );
    let inst = w["instances"][0].clone();
    assert_eq!(inst["component"], "Button/Primary", "{w}");
    let label = inst["textLayers"][0]["id"].clone();
    let node = call(&mut h, "get_node_info", json!({"nodeId": inst["id"]}));
    assert_eq!(node["instanceOf"], "Button/Primary");
    assert_eq!(node["mainComponentId"], main.as_str());
    let m = call(&mut h, "get_node_info", json!({"nodeId": main}));
    assert_eq!(
        (m["component"].as_bool(), m["instanceCount"].as_u64()),
        (Some(true), Some(1))
    );

    // Overrides survive edits to the main; reset brings the main's text back.
    call(
        &mut h,
        "set_text_content",
        json!({"updates": [{"nodeId": label, "textContent": "Buy"}]}),
    );
    let l = call(&mut h, "get_node_info", json!({"nodeId": label}));
    assert_eq!(l["overrides"][0], "text", "{l}");
    assert_eq!(l["instanceRootId"], inst["id"]);
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [main], "styles": {"backgroundColor": "#dc2626"}}]}),
    );
    let red = Some(Color::rgb(0xdc, 0x26, 0x26));
    assert_eq!(h.ed.doc.get(num(&inst["id"])).unwrap().style.fill(), red);
    assert_eq!(text_of(&h, 0, &label), "Buy");
    let r = call(&mut h, "reset_overrides", json!({"nodeIds": [inst["id"]]}));
    assert_eq!(r["reset"][0]["cleared"], l["overrides"], "{r}");
    assert_eq!(text_of(&h, 0, &label), "Label");

    // An instance on another page links back to the main.
    let p = call(&mut h, "create_page", json!({"name": "Screens"}));
    let page = p["pageId"].as_str().unwrap();
    let w2 = call(
        &mut h,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children", "pageId": page,
               "html": format!("<x-sketch-instance component=\"{main}\"/>")}),
    );
    let far = w2["instances"][0]["id"].clone();
    let n = call(&mut h, "get_node_info", json!({"nodeId": far}));
    assert_eq!(n["mainComponentId"], main.as_str(), "{n}");
    assert_eq!(h.ed.active_page, 0, "the user's page doesn't change");
    let m = call(&mut h, "get_node_info", json!({"nodeId": main}));
    assert_eq!(m["instanceCount"], 2);

    // Detach: the instance keeps its look and stops following the main.
    let d = call(&mut h, "detach_instances", json!({"nodeIds": [far, label]}));
    assert_eq!(d["detached"], json!([far]));
    assert!(
        d["skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("inside instance"),
        "{d}"
    );
    let n = call(&mut h, "get_node_info", json!({"nodeId": far}));
    assert!(
        n.get("instanceOf").is_none() && n.get("instance").is_none(),
        "{n}"
    );
    assert_eq!(h.ed.page_doc(1).get(num(&far)).unwrap().style.fill(), red);

    let pub_ = call(&mut h, "publish_design_system", json!({"prefix": "kit"}));
    assert!(
        pub_["added"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "Button/Primary component"),
        "{pub_}"
    );
    let sys = call(
        &mut h,
        "get_design_system",
        json!({"id": "kit", "types": ["component"]}),
    );
    assert!(sys.to_string().contains("Button/Primary"), "{sys}");
}

#[test]
fn several_layers_wrap_into_one_component_in_one_undo_step() {
    let mut h = host();
    let a = write(
        &mut h,
        "root",
        "<div style=\"width:20px;height:20px;background:red\"></div>",
    );
    let b = write(
        &mut h,
        "root",
        "<div style=\"width:20px;height:20px;background:blue\"></div>",
    );
    let (a, b) = (
        a["createdNodeIds"][0].clone(),
        b["createdNodeIds"][0].clone(),
    );
    h.ed.selection = vec![num(&a)];
    let c = call(&mut h, "create_component", json!({"nodeIds": [a, b]}));
    assert_eq!(c["wrapped"], true);
    let comp = num(&c["componentId"]);
    assert_eq!(h.ed.doc.get(comp).unwrap().children, vec![num(&a), num(&b)]);
    assert_eq!(h.ed.selection, vec![num(&a)], "the user's selection stays");
    call(&mut h, "undo", json!({}));
    assert!(h.ed.doc.get(comp).is_none());
    assert_eq!(h.ed.doc.root, vec![num(&a), num(&b)], "one undo step");
}

#[test]
fn component_mistakes_tell_the_agent_what_to_do() {
    let mut h = host();
    let (main, artboard) = kit(&mut h);

    // A main can't hold an instance of itself.
    let e = try_call(
        &mut h,
        "write_html",
        json!({"targetNodeId": main, "mode": "insert-children",
               "html": "<x-sketch-instance component=\"Button/Primary\"/>"}),
    )
    .unwrap_err();
    assert!(e.contains("itself"), "{e}");

    // Nor through an instance in another main.
    let w = write(
        &mut h,
        &artboard,
        "<x-sketch-instance component=\"Button/Primary\"/>",
    );
    let inst = w["instances"][0]["id"].clone();
    let label = w["instances"][0]["textLayers"][0]["id"].clone();
    call(
        &mut h,
        "create_component",
        json!({"nodeIds": [artboard], "name": "Screen"}),
    );
    let e = try_call(
        &mut h,
        "write_html",
        json!({"targetNodeId": main, "mode": "insert-children",
               "html": "<x-sketch-instance component=\"Screen\"/>"}),
    )
    .unwrap_err();
    assert!(e.contains("itself"), "{e}");

    // A layer inside an instance can't become a component.
    let e = try_call(&mut h, "create_component", json!({"nodeIds": [label]})).unwrap_err();
    assert!(
        e.contains("detach_instances") && e.contains(inst.as_str().unwrap()),
        "{e}"
    );

    // Two mains with one name need an id.
    let dup = write(&mut h, "root", BUTTON);
    call(
        &mut h,
        "create_component",
        json!({"nodeIds": [dup["createdNodeIds"][0]], "name": "Button/Primary"}),
    );
    let e = try_call(
        &mut h,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children",
               "html": "<x-sketch-instance component=\"Button/Primary\"/>"}),
    )
    .unwrap_err();
    assert!(
        e.contains("2 components are named") && e.contains(&main),
        "{e}"
    );

    // Plain layers have nothing to detach or reset.
    let e = try_call(&mut h, "reset_overrides", json!({"nodeIds": [main]})).unwrap_err();
    assert!(e.contains("not in an instance"), "{e}");
    let e = try_call(&mut h, "detach_instances", json!({"nodeIds": [main]})).unwrap_err();
    assert!(e.contains("not an instance"), "{e}");

    // An unknown name says where components are listed.
    let w = try_call(
        &mut h,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children",
               "html": "<x-sketch-instance component=\"Nope\"/>"}),
    )
    .unwrap_err();
    assert!(w.contains("get_basic_info"), "{w}");
}
