//! Files that use a design system's components: placing them, updating on open, removal,
//! nesting, publishing again and copying between files.

use super::tests::{publish_open, source, temp_store};
use super::{PublishRequest, Published};
use crate::editor::Editor;
use crate::editor::shortcuts::Action;
use crate::geom::{Rect, v2};
use crate::model::{Color, Node, NodeId, NodeKind, TokenProp};
use crate::project::Store;
use crate::session::Session;
use std::path::{Path, PathBuf};

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 255, 0);
const BLUE: Color = Color::rgb(0, 0, 255);
const GRAY: Color = Color::rgb(128, 128, 128);

fn add(ed: &mut Editor, kind: NodeKind, r: Rect, parent: Option<NodeId>) -> NodeId {
    ed.add_node(Node::new(NodeId(0), kind, r), parent)
}

/// A main named `name` holding a rect bound to `--brand` and a gray rect.
fn component(ed: &mut Editor, name: &str, at: f64) -> (NodeId, NodeId, NodeId) {
    let main = add(
        ed,
        NodeKind::Frame { clip: true },
        Rect::new(at, 0.0, 100.0, 40.0),
        None,
    );
    let bound = add(
        ed,
        NodeKind::Rect,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Some(main),
    );
    let plain = add(
        ed,
        NodeKind::Rect,
        Rect::new(20.0, 0.0, 10.0, 10.0),
        Some(main),
    );
    let brand = ed.tokens.by_css_name("--brand").unwrap().id.clone();
    ed.bind_token(&[bound], TokenProp::Fill, &brand);
    set_fill(ed, plain, GRAY);
    ed.edit("name", |tx| tx.node_mut(main).unwrap().name = name.into());
    ed.selection = vec![main];
    ed.create_component();
    (main, bound, plain)
}

fn set_fill(ed: &mut Editor, id: NodeId, c: Color) {
    ed.edit("fill", |tx| {
        tx.node_mut(id).unwrap().style.set_fill(Some(c))
    });
}

fn fill(ed: &Editor, page: usize, id: NodeId) -> Option<Color> {
    ed.page_doc(page).get(id)?.style.fill()
}

fn kids(ed: &Editor, page: usize, id: NodeId) -> Vec<NodeId> {
    ed.page_doc(page).get(id).unwrap().children.clone()
}

/// A project named `name` that uses `system`.
fn consumer(store: &Store, name: &str, system: &Published) -> (Session, Editor) {
    let mut s = Session::new(store.clone());
    let mut ed = Editor::default();
    s.create(&mut ed, name).unwrap();
    ed.use_system(system).unwrap();
    (s, ed)
}

fn place(ed: &mut Editor, name: &str, x: f64) -> NodeId {
    let id = ed.tokens.find_component(name).unwrap().id.clone();
    ed.place_component(&id, v2(x, 300.0)).unwrap()
}

fn close(s: &mut Session, ed: &mut Editor) -> PathBuf {
    s.save_all(ed).unwrap();
    s.project.as_ref().unwrap().dir.clone()
}

fn reopen(store: &Store, dir: &Path) -> (Session, Editor) {
    let mut s = Session::new(store.clone());
    let mut ed = Editor::default();
    s.open(&mut ed, dir).unwrap();
    (s, ed)
}

#[test]
fn a_file_places_a_systems_component_by_name_with_linked_bindings() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    component(&mut src, "Button", 0.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (_c, mut app) = consumer(&store, "App", &sys);

    let c = &app.tokens.components()[0];
    assert_eq!((c.name.as_str(), c.size()), ("Button", (100.0, 40.0)));
    assert_eq!(app.tokens.find_component("brand/BUTTON").unwrap().id, c.id);
    let undo_len = app.history.undo_len();
    let inst = place(&mut app, "button", 500.0);
    let n = app.doc.get(inst).unwrap();
    assert_eq!((n.rect.x, n.rect.y, n.rect.w), (500.0, 300.0, 100.0));
    assert!(!n.component && app.is_instance_root(inst));
    assert_eq!(app.instance_main(inst), None, "no page holds the main");
    let bound = app.doc.get(kids(&app, 0, inst)[0]).unwrap();
    let token = app.tokens.by_css_name("--brand-brand").unwrap();
    assert_eq!(bound.tokens.get(&TokenProp::Fill), Some(&token.id));
    assert_eq!(bound.style.fill(), Some(RED));
    assert_eq!(app.history.undo_len(), undo_len + 1);
    app.undo();
    assert!(app.doc.get(inst).is_none());
}

#[test]
fn an_update_on_open_syncs_every_page_keeps_overrides_and_undoes_in_one_step() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    let (main, _, plain) = component(&mut src, "Button", 0.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;

    let (mut c, mut app) = consumer(&store, "App", &sys);
    let a = place(&mut app, "Button", 0.0);
    let overridden = kids(&app, 0, a)[1];
    set_fill(&mut app, overridden, GREEN);
    app.mark_overrides(&[overridden], "fill");
    app.commit();
    app.add_page();
    let b = place(&mut app, "Button", 0.0);
    app.switch_page(0);
    let dir = close(&mut c, &mut app);

    // v2: wider, a new layer, a new fill for the overridden layer.
    src.edit("resize", |tx| tx.node_mut(main).unwrap().rect.w = 160.0);
    add(
        &mut src,
        NodeKind::Ellipse,
        Rect::new(40.0, 0.0, 8.0, 8.0),
        Some(main),
    );
    set_fill(&mut src, plain, BLUE);
    publish_open(&s, &mut src, &PublishRequest::default());

    let (_c, mut app) = reopen(&store, &dir);
    assert!(app.status.contains("Updated Brand to v2"), "{}", app.status);
    for (page, inst) in [(0, a), (1, b)] {
        let d = app.page_doc(page);
        assert_eq!(d.get(inst).unwrap().rect.w, 160.0, "page {page}");
        assert_eq!(kids(&app, page, inst).len(), 3, "page {page}");
    }
    assert_eq!(fill(&app, 0, overridden), Some(GREEN), "the override stays");
    assert_eq!(fill(&app, 1, kids(&app, 1, b)[1]), Some(BLUE));
    assert_eq!(app.dirty_pages(), [0, 1]);

    app.undo();
    assert_eq!(app.history.undo_len(), 0);
    assert_eq!(kids(&app, 0, a).len(), 2);
    assert_eq!(app.doc.get(a).unwrap().rect.w, 100.0);
    assert_eq!(fill(&app, 0, overridden), Some(GREEN));
    app.switch_page(1);
    assert_eq!(kids(&app, 1, b).len(), 2);
    app.doc.validate().unwrap();
}

#[test]
fn a_removed_component_leaves_frames_that_keep_their_look() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    component(&mut src, "Button", 0.0);
    let (card, _, _) = component(&mut src, "Card", 200.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (mut c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Card", 0.0);
    let dir = close(&mut c, &mut app);

    src.edit("delete", |tx| tx.remove(card));
    publish_open(&s, &mut src, &PublishRequest::default());
    let (_c, mut app) = reopen(&store, &dir);
    assert!(
        app.status
            .contains("Removed Card component; 1 instance is a plain frame now."),
        "{}",
        app.status
    );
    assert!(!app.is_instance_root(inst));
    assert_eq!(kids(&app, 0, inst).len(), 2);
    assert_eq!(fill(&app, 0, kids(&app, 0, inst)[1]), Some(GRAY));
    // Undo brings the component back, and the frame is an instance again.
    app.undo();
    assert!(app.is_instance_root(inst));
}

#[test]
fn nested_components_update_through_the_outer_copy() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    let (button, _, plain) = component(&mut src, "Button", 0.0);
    let inner = src.create_instance(button, v2(0.0, 200.0)).unwrap();
    src.selection = vec![inner];
    src.create_component();
    let card = src.doc.get(inner).unwrap().parent.unwrap();
    src.edit("name", |tx| tx.node_mut(card).unwrap().name = "Card".into());
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;

    let (mut c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Card", 0.0);
    let dir = close(&mut c, &mut app);
    set_fill(&mut src, plain, BLUE);
    publish_open(&s, &mut src, &PublishRequest::default());

    let (_c, app) = reopen(&store, &dir);
    let d = app.page_doc(0);
    let leaf = *d.subtree(inst).last().unwrap();
    assert_eq!(d.get(leaf).unwrap().style.fill(), Some(BLUE));
}

#[test]
fn a_file_that_uses_components_publishes_them_as_plain_layers() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    component(&mut src, "Button", 0.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Button", 0.0);
    app.selection = vec![inst];
    app.create_component();
    let r = publish_open(&c, &mut app, &PublishRequest::default());

    let published = &r.system.components[0];
    assert!(published.nodes.iter().all(|n| n.instance.is_none()));
    assert!(published.nested.is_empty());
    let bound = published.nodes.iter().find(|n| n.style.fill() == Some(RED));
    assert!(
        bound.unwrap().tokens.is_empty(),
        "other systems' tokens become values"
    );
}

#[test]
fn copies_persist_and_pasted_instances_stay_linked_where_the_system_is_used() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    component(&mut src, "Button", 0.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (mut c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Button", 0.0);
    let components = app.tokens.components().to_vec();
    let dir = close(&mut c, &mut app);
    let (_c, mut app) = reopen(&store, &dir);
    assert_eq!(app.tokens.components(), components);

    // Duplicating keeps the link to the system's layer ids.
    app.selection = vec![inst];
    app.run(Action::Duplicate);
    let dup = app.selection[0];
    assert_eq!(
        app.doc.get(dup).unwrap().instance,
        app.doc.get(inst).unwrap().instance
    );
    assert!(app.is_instance_root(dup));

    app.selection = vec![inst];
    app.run(Action::Copy);
    Session::new(store.clone())
        .create(&mut app, "Elsewhere")
        .unwrap();
    app.run(Action::Paste);
    assert!(app.doc.get(app.selection[0]).unwrap().instance.is_none());

    Session::new(store.clone())
        .create(&mut app, "Other")
        .unwrap();
    app.use_system(&sys).unwrap();
    app.run(Action::Paste);
    assert!(app.is_instance_root(app.selection[0]));
}

#[test]
fn an_instance_in_another_mode_keeps_it_and_saves_nothing_when_unchanged() {
    use crate::model::TokenKind;
    let store = temp_store();
    let (s, mut src) = source(&store);
    let (dim, fg) = src.edit_tokens("add", None, |t| {
        let dim = t.add_mode();
        let black = [(dim.clone(), "#000000".to_string())];
        let fg = t.create_in_modes(TokenKind::Color, "fg", "#ffffff", &black, None);
        (dim, fg.unwrap())
    });
    let (main, bound, plain) = component(&mut src, "Button", 0.0);
    src.bind_token(&[bound], TokenProp::Fill, &fg);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;

    let (mut c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Button", 0.0);
    let mode = crate::model::tokens::linked_id(&sys.id, &dim);
    app.edit("mode", |tx| {
        let n = tx.node_mut(inst).unwrap();
        n.mode = Some(mode.clone());
        n.instance.as_mut().unwrap().overrides.push("mode".into());
    });
    let leaf = kids(&app, 0, inst)[0];
    assert_eq!(fill(&app, 0, leaf), Some(Color::rgb(0, 0, 0)));
    c.save(&mut app).unwrap();
    app.sync_all_instances();
    assert!(app.dirty_pages().is_empty(), "a sync that changes nothing");
    let dir = close(&mut c, &mut app);

    src.edit("resize", |tx| tx.node_mut(main).unwrap().rect.w = 120.0);
    set_fill(&mut src, plain, BLUE);
    publish_open(&s, &mut src, &PublishRequest::default());
    let (_c, app) = reopen(&store, &dir);
    assert_eq!(app.doc.get(inst).unwrap().rect.w, 120.0);
    assert_eq!(fill(&app, 0, leaf), Some(Color::rgb(0, 0, 0)));
    let n = app.doc.get(leaf).unwrap();
    assert!(n.tokens.contains_key(&TokenProp::Fill), "still bound");
}

#[test]
fn a_files_copy_of_a_component_keeps_its_images() {
    use crate::model::Paint;
    let store = temp_store();
    let (s, mut src) = source(&store);
    let (_, _, plain) = component(&mut src, "Logo", 0.0);
    let asset = crate::assets::register_rgba(&image::RgbaImage::new(3, 3));
    src.edit("image", |tx| {
        tx.node_mut(plain).unwrap().style.fills = vec![Paint::Image {
            asset: asset.clone(),
            mode: Default::default(),
            opacity: 1.0,
        }]
    });
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (mut c, mut app) = consumer(&store, "App", &sys);
    let dir = close(&mut c, &mut app);
    let saved = dir
        .join(crate::project::ASSETS_DIR)
        .join(format!("{asset}.png"));
    assert!(saved.exists(), "saved with the file before any instance");
}

#[test]
fn undoing_an_update_brings_back_a_removed_layer_with_its_override() {
    let store = temp_store();
    let (s, mut src) = source(&store);
    let (_, _, plain) = component(&mut src, "Button", 0.0);
    let sys = publish_open(&s, &mut src, &PublishRequest::default()).system;
    let (mut c, mut app) = consumer(&store, "App", &sys);
    let inst = place(&mut app, "Button", 0.0);
    let overridden = kids(&app, 0, inst)[1];
    set_fill(&mut app, overridden, GREEN);
    app.mark_overrides(&[overridden], "fill");
    app.commit();
    let dir = close(&mut c, &mut app);

    src.edit("delete", |tx| tx.remove(plain));
    publish_open(&s, &mut src, &PublishRequest::default());
    let (_c, mut app) = reopen(&store, &dir);
    assert!(app.doc.get(overridden).is_none());
    app.undo();
    let n = app.doc.get(overridden).unwrap();
    assert_eq!(n.style.fill(), Some(GREEN));
    assert_eq!(n.instance.as_ref().unwrap().overrides, ["fills"]);
}
