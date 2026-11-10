//! MCP personal library and Excalidraw import tools on a headless session.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::NodeKind;
use sketch_rs::project::Store;

const SCENE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/excalidraw/basic.excalidraw"
);
const LIB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/excalidraw/library.excalidrawlib"
);

fn store() -> Store {
    Store::open(std::env::temp_dir().join(format!("sketch-rs-mcp-lib-{:016x}", fastrand::u64(..))))
        .unwrap()
}

fn host() -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store());
    h.session.create(&mut h.ed, "Library").unwrap();
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

fn names(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn library_add_insert_rename_and_remove() {
    let mut h = host();
    let starter = names(&call(&mut h, "list_library_items", json!({}))).len();
    assert!(starter > 0, "a new library has the starter items");

    let a = call(
        &mut h,
        "create_shape",
        json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 60}),
    )["id"]
        .clone();
    let b = call(
        &mut h,
        "create_shape",
        json!({"type": "text", "x": 10, "y": 10, "text": "Go"}),
    )["id"]
        .clone();
    let item = call(
        &mut h,
        "add_library_item",
        json!({"nodeIds": [a, b], "name": "Go button"}),
    );
    assert_eq!(item["name"], "Go button");
    assert_eq!(item["width"], 100.0);
    assert_eq!(item["layers"], 2);
    // Saved right away, where the app's Library tab reads it.
    let path = sketch_rs::library::Library::path_in(&h.session.store.root);
    let on_disk = sketch_rs::library::Library::load(&path);
    assert!(on_disk.items.iter().any(|i| i.name == "Go button"));

    // Insert by name on another page, centered on a point.
    let page = call(&mut h, "create_page", json!({"name": "Two"}))["pageId"].clone();
    let ins = call(
        &mut h,
        "insert_library_item",
        json!({"itemId": "go button", "x": 500, "y": 500, "pageId": page}),
    );
    assert_eq!(ins["ids"].as_array().unwrap().len(), 2);
    let doc = h.ed.page_doc(1);
    assert_eq!(doc.nodes.len(), 2);
    let r = doc
        .nodes
        .values()
        .find(|n| matches!(n.kind, NodeKind::Rect))
        .unwrap();
    assert_eq!((r.rect.x, r.rect.y), (450.0, 470.0));
    assert_eq!(h.ed.active_page, 0);
    // One undo step.
    call(&mut h, "undo", json!({"pageId": page}));
    assert!(h.ed.page_doc(1).nodes.is_empty());

    // Without x/y it goes into free space, clear of what's there.
    let ins = call(&mut h, "insert_library_item", json!({"itemId": item["id"]}));
    let first = ins["ids"][0].as_str().unwrap();
    let info = call(&mut h, "get_node_info", json!({"nodeId": first}));
    assert!(info["worldX"].as_f64().unwrap() >= 100.0, "{info}");

    // A bad id in the batch changes nothing.
    let e = err(
        &mut h,
        "update_library_items",
        json!({"updates": [{"itemId": item["id"], "name": "Kept"}, {"itemId": "nope", "delete": true}]}),
    );
    assert!(e.contains("no library item"), "{e}");
    assert!(names(&call(&mut h, "list_library_items", json!({}))).contains(&"Go button".into()));
    let r = call(
        &mut h,
        "update_library_items",
        json!({"updates": [{"itemId": item["id"], "name": "Start button"}]}),
    );
    assert_eq!(r["renamed"][0]["name"], "Start button");
    call(
        &mut h,
        "update_library_items",
        json!({"updates": [{"itemId": "Start button", "delete": true}]}),
    );
    assert_eq!(
        names(&call(&mut h, "list_library_items", json!({}))).len(),
        starter
    );
    assert_eq!(
        sketch_rs::library::Library::load(&path).items.len(),
        starter
    );
}

/// The app showing its home screen: no canvas.
struct Home(exec::SessionHost);

impl exec::Host for Home {
    fn editor(&mut self) -> &mut sketch_rs::editor::Editor {
        self.0.editor()
    }
    fn store(&self) -> &Store {
        self.0.store()
    }
    fn project(&self) -> Option<&sketch_rs::project::Project> {
        None
    }
    fn canvas_open(&self) -> bool {
        false
    }
    fn open_project(&mut self, dir: &std::path::Path) -> Result<(), String> {
        self.0.open_project(dir)
    }
    fn project_moved(&mut self, dir: &std::path::Path) {
        self.0.project_moved(dir)
    }
    fn save(&mut self) -> Result<(), String> {
        self.0.save()
    }
    fn close_project(&mut self) -> Result<(), String> {
        self.0.close_project()
    }
    fn gpu(&mut self) -> Option<sketch_rs::render::gpu::Headless> {
        None
    }
    fn library(&mut self) -> &mut sketch_rs::library::Library {
        self.0.library()
    }
}

#[test]
fn library_tools_work_from_the_home_screen() {
    let mut h = Home(exec::SessionHost::new(store()));
    let run = |h: &mut Home, name: &str, args: Value| exec::call(h, name, &args);
    let list = |h: &mut Home| match run(h, "list_library_items", json!({})).unwrap().remove(0) {
        Out::Text(t) => serde_json::from_str::<Value>(&t).unwrap(),
        _ => panic!(),
    };
    let before = names(&list(&mut h)).len();
    assert!(before > 0);
    // A library file imports without a project; a scene onto a page needs one.
    run(&mut h, "import_excalidraw", json!({"path": LIB})).unwrap();
    assert_eq!(names(&list(&mut h)).len(), before + 2);
    let e = run(&mut h, "import_excalidraw", json!({"path": SCENE})).unwrap_err();
    assert!(e.contains("No project is open"), "{e}");
    run(
        &mut h,
        "update_library_items",
        json!({"updates": [{"itemId": "Button", "delete": true}]}),
    )
    .unwrap();
    assert_eq!(names(&list(&mut h)).len(), before + 1);
    let defs = sketch_rs::mcp::tools::definitions();
    let def = |n: &str| defs.iter().find(|t| t["name"] == n).unwrap().clone();
    assert!(def("list_library_items")["inputSchema"]["properties"]["fileId"].is_null());
    assert!(def("add_library_item")["inputSchema"]["properties"]["fileId"].is_object());
    assert_eq!(
        def("update_library_items")["annotations"]["destructiveHint"],
        true
    );
}

#[test]
fn excalidraw_scene_lands_on_the_page_as_one_undo_step() {
    let mut h = host();
    call(
        &mut h,
        "create_shape",
        json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100}),
    );
    let r = call(&mut h, "import_excalidraw", json!({"path": SCENE}));
    assert!(r["layers"].as_u64().unwrap() >= 9, "{r}");
    assert!(
        r["worldX"].as_f64().unwrap() >= 100.0,
        "placed clear of content: {r}"
    );
    let doc = &h.ed.doc;
    doc.validate().unwrap();
    // Arrows keep their bindings; freehand lines stay strokes.
    assert!(doc.nodes.values().any(|n| matches!(
        &n.kind,
        NodeKind::Connector { start, end, .. } if start.is_some() || end.is_some()
    )));
    assert!(
        doc.nodes
            .values()
            .any(|n| matches!(n.kind, NodeKind::Stroke { .. }))
    );
    call(&mut h, "undo", json!({}));
    assert_eq!(h.ed.doc.nodes.len(), 1);

    // JSON data works too, centered on a point.
    let data = std::fs::read_to_string(SCENE).unwrap();
    let r = call(
        &mut h,
        "import_excalidraw",
        json!({"data": data, "x": 0, "y": 1000}),
    );
    let (y, hgt) = (r["worldY"].as_f64().unwrap(), r["height"].as_f64().unwrap());
    assert!((y + hgt / 2.0 - 1000.0).abs() < 1.0, "{r}");
    // ... and as an object.
    let obj: Value = serde_json::from_str(&data).unwrap();
    call(&mut h, "import_excalidraw", json!({"data": obj}));

    let e = err(
        &mut h,
        "import_excalidraw",
        json!({"data": {"type": "nope"}}),
    );
    assert!(e.contains("not an Excalidraw"), "{e}");
    let e = err(&mut h, "import_excalidraw", json!({}));
    assert!(e.contains("path"), "{e}");
}

#[test]
fn excalidraw_library_and_new_project_imports() {
    let mut h = host();
    let before = names(&call(&mut h, "list_library_items", json!({}))).len();
    let r = call(&mut h, "import_excalidraw", json!({"path": LIB}));
    assert_eq!(r["library"], true);
    let added: Vec<&str> = r["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(added, ["Button", "Decision"]);
    assert_eq!(
        names(&call(&mut h, "list_library_items", json!({}))).len(),
        before + 2
    );
    assert!(h.ed.doc.nodes.is_empty(), "nothing lands on the page");

    // A scene as a new project in a folder.
    let folder = call(&mut h, "create_folder", json!({"name": "Boards"}));
    let made = call(
        &mut h,
        "import_excalidraw",
        json!({"path": SCENE, "createFile": true, "parentId": folder["folderId"]}),
    );
    assert_eq!(made["name"], "basic");
    assert_eq!(made["folder"], "Boards");
    let info = call(&mut h, "open_file", json!({"fileId": made["fileId"]}));
    assert!(info["nodeCount"].as_u64().unwrap() >= 9, "{info}");
    let e = err(
        &mut h,
        "import_excalidraw",
        json!({"path": LIB, "createFile": true}),
    );
    assert!(e.contains("library"), "{e}");
}
