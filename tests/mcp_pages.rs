//! Pages, projects, trash and design-system tools over MCP: everything a person does from the
//! Pages list, a project card's menu and the Trash, done by an agent.

use serde_json::{Value, json};
use sketch_rs::geom::v2;
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::NodeId;
use sketch_rs::project::Store;

fn store() -> Store {
    Store::open(std::env::temp_dir().join(format!("sketch-rs-pages-{:016x}", fastrand::u64(..))))
        .unwrap()
}

fn host_in(s: &Store, name: &str) -> exec::SessionHost {
    let mut h = exec::SessionHost::new(s.clone());
    h.session.create(&mut h.ed, name).unwrap();
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

fn err(h: &mut exec::SessionHost, name: &str, args: Value) -> String {
    try_call(h, name, args).expect_err(name)
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

fn names(h: &exec::SessionHost) -> Vec<String> {
    h.ed.pages.iter().map(|p| p.name.clone()).collect()
}

/// A 100x100 card at (40, 60) on the page the user sees, with a text child.
fn card(h: &mut exec::SessionHost) -> (Value, Value) {
    let r = call(
        h,
        "write_html",
        json!({"targetNodeId": "root", "mode": "insert-children",
               "html": "<div style=\"position:absolute;left:40px;top:60px;width:100px;height:100px;background:#ff0000\"><p>Hi</p></div>"}),
    );
    let id = r["createdNodeIds"][0].clone();
    let kids = call(h, "get_children", json!({"nodeId": id}));
    (id, kids["children"][0]["id"].clone())
}

#[test]
fn agents_duplicate_reorder_and_delete_pages_without_moving_the_user() {
    let s = store();
    let mut h = host_in(&s, "Pages");
    card(&mut h);
    let first = h.ed.pages[0].id.clone();
    let b = call(&mut h, "create_page", json!({"name": "B"}))["pageId"].clone();

    let dup = call(&mut h, "duplicate_page", json!({"pageId": first}));
    assert_eq!(names(&h), ["Page 1", "Page 1 copy", "B"]);
    assert_eq!(h.ed.active_page, 0, "the user stays on their page");
    let prefix = dup["idPrefix"].as_str().unwrap();
    let copy = h.ed.page_doc(1);
    assert_eq!(copy.nodes.len(), h.ed.doc.nodes.len());
    let info = call(
        &mut h,
        "get_tree_summary",
        json!({"nodeId": "root", "pageId": dup["pageId"]}),
    );
    assert!(info.as_str().unwrap().contains(prefix), "{info}");
    let named = call(
        &mut h,
        "duplicate_page",
        json!({"pageId": "B", "name": "Draft"}),
    );
    assert_eq!(named["name"], "Draft");
    assert_eq!(names(&h), ["Page 1", "Page 1 copy", "B", "Draft"]);

    // A partial order changes nothing.
    let e = err(&mut h, "arrange_pages", json!({"order": [b, first]}));
    assert!(e.contains("missing"), "{e}");
    assert_eq!(names(&h), ["Page 1", "Page 1 copy", "B", "Draft"]);
    let e = err(
        &mut h,
        "arrange_pages",
        json!({"order": [b, b, first, "Draft"]}),
    );
    assert!(e.contains("twice"), "{e}");
    let r = call(
        &mut h,
        "arrange_pages",
        json!({"order": ["Draft", b, first, dup["pageId"]]}),
    );
    assert_eq!(names(&h), ["Draft", "B", "Page 1", "Page 1 copy"]);
    assert_eq!(r["pages"][2]["isActive"], true);
    assert_eq!(h.ed.active_page_id(), first, "the user is still on Page 1");

    // Every page can't go, and a bad id deletes nothing.
    let all: Vec<String> = h.ed.pages.iter().map(|p| p.id.clone()).collect();
    assert!(err(&mut h, "delete_pages", json!({"pageIds": all})).contains("at least one"));
    assert!(err(&mut h, "delete_pages", json!({"pageIds": ["B", "nope"]})).contains("nope"));
    assert_eq!(h.ed.pages.len(), 4);
    call(&mut h, "delete_pages", json!({"pageIds": ["Draft", "B"]}));
    assert_eq!(names(&h), ["Page 1", "Page 1 copy"]);
    assert_eq!(h.ed.active_page_id(), first);
    // Deleting the user's page shows the next one, and the files go on save.
    call(&mut h, "delete_pages", json!({"pageIds": [first]}));
    assert_eq!(names(&h), ["Page 1 copy"]);
    assert_eq!(h.ed.active_page, 0);
    call(&mut h, "finish_working_on_nodes", json!({}));
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    let files = std::fs::read_dir(dir.join("pages")).unwrap().count();
    assert_eq!(files, 1);
    let reopened = sketch_rs::project::Project::open(&dir).unwrap();
    assert_eq!(reopened.manifest.pages.len(), 1);
}

#[test]
fn layers_move_and_copy_to_another_page_keeping_their_place() {
    let s = store();
    let mut h = host_in(&s, "Move");
    let (id, child) = card(&mut h);
    let src = h.ed.pages[0].id.clone();
    let dst = call(&mut h, "create_page", json!({"name": "Two"}))["pageId"].clone();

    let e = err(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [id], "toPageId": src}),
    );
    assert!(e.contains("move_nodes"), "{e}");
    let e = err(
        &mut h,
        "move_nodes",
        json!({"moves": [{"nodeId": id, "parentId": format!("{}:0", &dst.as_str().unwrap()[..6])}]}),
    );
    assert!(
        e.contains("move_to_page") || e.contains("does not exist"),
        "{e}"
    );

    // Copy: the original stays.
    let c = call(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [id, child], "toPageId": "Two", "copy": true}),
    );
    assert_eq!(c["result"], "copied");
    assert_eq!(
        c["newIds"].as_array().unwrap().len(),
        1,
        "the child goes with its parent"
    );
    assert!(c["idMap"][child.as_str().unwrap()].is_string(), "{c}");
    assert!(h.ed.doc.get(num(&id)).is_some());
    let info = call(&mut h, "get_node_info", json!({"nodeId": c["newIds"][0]}));
    assert_eq!(
        (info["worldX"].clone(), info["worldY"].clone()),
        (json!(40.0), json!(60.0))
    );
    assert_eq!(h.ed.active_page, 0);

    // Move into a frame on the destination: same x/y inside it.
    let frame = call(
        &mut h,
        "create_artboard",
        json!({"name": "Holder", "pageId": dst, "styles": {"left": "500px", "top": "500px", "width": "400px", "height": "400px", "display": "block"}}),
    )["id"]
        .clone();
    let m = call(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [id], "toPageId": dst, "parentId": frame}),
    );
    assert_eq!(m["result"], "moved");
    assert!(
        h.ed.doc.get(num(&id)).is_none(),
        "gone from the source page"
    );
    let moved = call(&mut h, "get_node_info", json!({"nodeId": m["newIds"][0]}));
    assert_eq!(moved["parentId"], frame, "{moved}");
    assert_eq!(
        (moved["x"].clone(), moved["y"].clone()),
        (json!(40.0), json!(60.0))
    );
    // The move is an undo step on each page.
    call(&mut h, "undo", json!({"pageId": src}));
    assert!(h.ed.doc.get(num(&id)).is_some());
}

#[test]
fn a_moved_main_component_keeps_its_instances() {
    let s = store();
    let mut h = host_in(&s, "Components");
    let (main, _) = card(&mut h);
    let main = num(&main);
    h.ed.selection = vec![main];
    h.ed.create_component();
    let here = h.ed.create_instance(main, v2(400.0, 0.0)).unwrap();
    let two = call(&mut h, "create_page", json!({"name": "Two"}))["pageId"].clone();
    let lib = call(&mut h, "create_page", json!({"name": "Components"}))["pageId"].clone();
    let key = |h: &exec::SessionHost, id: NodeId| exec::node_key(&h.ed, 0, id);
    // An instance on another page, placed the way paste does it.
    let (here_key, main_key) = (key(&h, here), key(&h, main));
    let there = call(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [here_key], "toPageId": two, "copy": true}),
    )["newIds"][0]
        .clone();

    // A copied main becomes an instance of the original.
    let copied = call(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [main_key], "toPageId": two, "copy": true}),
    )["newIds"][0]
        .clone();
    let info = call(&mut h, "get_node_info", json!({"nodeId": copied}));
    assert_eq!(info["instance"], true, "{info}");

    let r = call(
        &mut h,
        "move_to_page",
        json!({"nodeIds": [main_key], "toPageId": lib}),
    );
    let new_main = num(&r["newIds"][0]);
    assert!(r["instancesRelinked"].as_u64().unwrap() >= 3, "{r}");
    assert!(h.ed.page_doc(2).get(new_main).unwrap().component);
    assert_eq!(h.ed.instance_main(here), Some((2, new_main)));
    let (p, there_id) = (1, num(&there));
    h.ed.switch_page(p);
    assert_eq!(h.ed.instance_main(there_id), Some((2, new_main)));
    assert_eq!(h.ed.instance_main(num(&copied)), Some((2, new_main)));

    // One undo on the source page brings the main back, linked to the instance left there.
    h.ed.switch_page(0);
    let first = h.ed.pages[0].id.clone();
    call(&mut h, "undo", json!({"pageId": first}));
    assert!(h.ed.doc.get(main).unwrap().component);
    assert_eq!(h.ed.instance_main(here), Some((0, main)));
}

#[test]
fn projects_duplicate_trash_and_restore() {
    let s = store();
    let mut h = host_in(&s, "Open one");
    card(&mut h);
    let open_id = h.session.project.as_ref().unwrap().manifest.id.clone();

    // The open project is saved before it's copied.
    let d = call(
        &mut h,
        "duplicate_file",
        json!({"fileId": open_id, "name": "Copy A"}),
    );
    assert_eq!(d["name"], "Copy A");
    let copy = sketch_rs::project::Project::open(std::path::Path::new(d["path"].as_str().unwrap()))
        .unwrap();
    assert_eq!(
        copy.load_pages().unwrap()[0].doc.nodes.len(),
        h.ed.doc.nodes.len()
    );
    let plain = call(&mut h, "duplicate_file", json!({"fileId": "Copy A"}));
    assert_eq!(plain["name"], "Copy A copy");

    let f = call(&mut h, "create_folder", json!({"name": "Old work"}));
    let inside = call(
        &mut h,
        "create_file",
        json!({"name": "Inside", "parentId": f["folderId"]}),
    );

    // A bad id trashes nothing.
    assert!(
        err(
            &mut h,
            "trash_resources",
            json!({"resourceIds": ["Copy A", "nope"]})
        )
        .contains("nope")
    );
    assert_eq!(s.list().len(), 4);

    let t = call(
        &mut h,
        "trash_resources",
        json!({"resourceIds": [d["fileId"], "Old work", inside["fileId"]]}),
    );
    assert_eq!(t["trashed"].as_array().unwrap().len(), 2, "{t}");
    assert_eq!(s.list().len(), 2);
    assert!(s.folders().is_empty());
    let trash = call(&mut h, "list_trash", json!({}));
    assert_eq!(trash["items"].as_array().unwrap().len(), 2);
    let res = call(&mut h, "list_resources", json!({}));
    assert!(!res.to_string().contains("Copy A\""), "{res}");

    // Restore by id, and by the folder the project came from.
    assert!(
        err(
            &mut h,
            "restore_resources",
            json!({"resourceIds": [d["fileId"], "nope"]})
        )
        .contains("nope")
    );
    assert_eq!(s.list_trash().len(), 2);
    let r = call(
        &mut h,
        "restore_resources",
        json!({"resourceIds": [d["fileId"]]}),
    );
    assert_eq!(r["restored"][0]["name"], "Copy A");
    let r = call(
        &mut h,
        "restore_resources",
        json!({"resourceIds": ["old work"]}),
    );
    assert_eq!(r["restored"][0]["location"], "Old work/Inside", "{r}");
    assert!(s.list_trash().is_empty());
    assert_eq!(s.folders().len(), 1);

    // The open project is saved, closed and trashed, and comes back with its work.
    let t = call(&mut h, "trash_resources", json!({"resourceIds": [open_id]}));
    assert_eq!(t["closedOpenProject"], true, "{t}");
    assert!(h.session.project.is_none());
    let back = call(
        &mut h,
        "restore_resources",
        json!({"resourceIds": [open_id]}),
    );
    let dir = std::path::Path::new(back["restored"][0]["path"].as_str().unwrap());
    let pages = sketch_rs::project::Project::open(dir)
        .unwrap()
        .load_pages()
        .unwrap();
    assert_eq!(pages[0].doc.nodes.len(), 2, "the card survived the trip");

    // Deleting forever stays with the user.
    let tools: Vec<String> = sketch_rs::mcp::tools::definitions()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        !tools
            .iter()
            .any(|n| n.contains("empty") || n.contains("forever"))
    );
}

#[test]
fn a_file_unpublishes_its_design_system() {
    let s = store();
    let mut h = host_in(&s, "Acme");
    assert!(err(&mut h, "unpublish_design_system", json!({})).contains("hasn't published"));
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--primary", "value": "#2f6bff"}]}),
    );
    call(&mut h, "publish_design_system", json!({}));
    let mut other = host_in(&s, "Landing");
    assert_eq!(
        call(&mut other, "list_design_systems", json!({}))["systems"]
            .as_array()
            .map_or(0, Vec::len),
        1
    );
    let r = call(&mut h, "unpublish_design_system", json!({}));
    assert_eq!(r["result"], "unpublished");
    let list = call(&mut other, "list_design_systems", json!({}));
    assert!(!list.to_string().contains("Acme"), "{list}");
    assert!(err(&mut h, "unpublish_design_system", json!({})).contains("isn't published"));
    // Publishing again brings it back.
    call(&mut h, "publish_design_system", json!({}));
    let list = call(&mut other, "list_design_systems", json!({}));
    assert!(list.to_string().contains("Acme"), "{list}");
}

#[test]
fn exports_can_leave_the_background_transparent() {
    let s = store();
    let mut h = host_in(&s, "Export");
    call(
        &mut h,
        "create_shape",
        json!({"type": "ellipse", "x": 0, "y": 0, "width": 100, "height": 100, "styles": {"backgroundColor": "#ff0000"}}),
    );
    let dir = std::env::temp_dir().join(format!("sketch-rs-xp-{:016x}", fastrand::u64(..)));
    let svg = |h: &mut exec::SessionHost, transparent: bool| {
        let r = call(
            h,
            "export",
            json!({"directory": dir, "nodes": {"root": [{"format": "svg", "transparent": transparent}]}}),
        );
        std::fs::read_to_string(r["files"][0].as_str().unwrap()).unwrap()
    };
    let (white, clear) = (svg(&mut h, false), svg(&mut h, true));
    assert!(white.to_lowercase().contains("#ffffff"), "{white}");
    assert!(!clear.to_lowercase().contains("#ffffff"), "{clear}");
    if exec::Host::gpu(&mut h).is_some() {
        let png = |h: &mut exec::SessionHost, transparent: bool| {
            let r = call(
                h,
                "export",
                json!({"directory": dir, "nodes": {"root": [{"format": "png", "scale": "1x", "transparent": transparent}]}}),
            );
            image::open(r["files"][0].as_str().unwrap())
                .unwrap()
                .to_rgba8()
        };
        assert_eq!(png(&mut h, false).get_pixel(1, 1).0[3], 255);
        let img = png(&mut h, true);
        assert_eq!(img.get_pixel(1, 1).0[3], 0);
        assert_eq!(img.get_pixel(50, 50).0, [255, 0, 0, 255]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
