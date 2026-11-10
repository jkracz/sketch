//! Milestones 1–2: local projects, the home screen, autosave and pages.

mod common;

use common::*;
use sketch_rs::demo;
use sketch_rs::editor::Editor;
use sketch_rs::geom::{Rect, v2};
use sketch_rs::model::{Color, Node, NodeId, NodeKind};
use sketch_rs::project::{PageData, Project, Store};
use sketch_rs::render::frame;
use sketch_rs::session::{self, Session};
use sketch_rs::ui::UiState;
use sketch_rs::ui::home::{HomeAction, HomeView};

fn temp_store(tag: &str) -> Store {
    let root = std::env::temp_dir().join(format!("sketch-rs-{tag}-{:016x}", fastrand::u64(..)));
    Store::open(root).unwrap()
}

/// A few projects with different content, as a user would have them.
fn populate(store: &Store) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    let showcase = demo::showcase();
    let boards: Vec<(&str, sketch_rs::model::Document)> = vec![
        ("Launch plan", showcase.doc.clone()),
        ("Brand moodboard", demo::moodboard().doc),
        ("Onboarding flow", demo::flow().doc),
        ("Empty ideas", sketch_rs::model::Document::default()),
    ];
    for (i, (name, doc)) in boards.into_iter().enumerate() {
        let p = store
            .create_with_pages(
                name,
                vec![PageData {
                    id: sketch_rs::project::new_id(),
                    name: "Page 1".into(),
                    doc,
                    camera: None,
                }],
            )
            .unwrap();
        let mut m = Project::open(&p.dir).unwrap();
        // Stagger modification times so the grid order is deterministic.
        m.manifest.modified = 1_760_000_000_000 - i as u64 * 3_600_000 * (i as u64 + 1);
        m.write_manifest().unwrap();
        dirs.push(p.dir);
    }
    dirs
}

#[test]
fn m1_home_screen_lists_projects_with_thumbnails() {
    let store = temp_store("home");
    let dirs = populate(&store);
    let Some(g) = gpu() else { return };
    for d in &dirs {
        let p = Project::open(d).unwrap();
        let doc = &p.load_pages().unwrap()[0].doc;
        if let Some(png) = session::render_thumbnail(g, doc).unwrap() {
            std::fs::write(p.thumbnail_path(), png).unwrap();
        }
    }
    // A trashed project shows up in the trash view.
    let (old, _) = store.create("Old sketches").unwrap();
    store.trash(&old.dir).unwrap();

    let mut st = UiState {
        home: Some(HomeView::load(&store, 1_760_000_600_000)),
        ..Default::default()
    };
    let hv = st.home.as_ref().unwrap();
    let names: Vec<&str> = hv
        .entries
        .iter()
        .map(|e| e.manifest.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Launch plan",
            "Brand moodboard",
            "Onboarding flow",
            "Empty ideas"
        ]
    );
    assert_eq!(
        hv.entries.iter().filter(|e| e.thumbnail.is_some()).count(),
        3
    );
    assert_eq!(hv.trash.len(), 1);

    let mut ed = Editor::default();
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(shot("milestone1_home.png")).unwrap();
    // Thumbnails are drawn: the first card has non-background pixels (the demo frames).
    let card = Rect::new(220.0, 70.0, 232.0, 148.0);
    assert!(count(&img, card, |p| p[0] < 200 || p[2] < 200) > 200);

    st.home.as_mut().unwrap().show_trash = true;
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(shot("milestone1_trash.png")).unwrap();
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m1_autosave_survives_reopen_and_crash_temp_files() {
    let store = temp_store("autosave");
    let mut sess = Session::new(store.clone());
    let mut ed = Editor::default();
    sess.create(&mut ed, "Crashy").unwrap();
    let dir = sess.project.as_ref().unwrap().dir.clone();
    ed.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 50.0, 40.0)),
        None,
    );
    let t = std::time::Instant::now();
    assert!(!sess.autosave_due(&ed, t), "debounced");
    assert!(sess.autosave_due(&ed, t + session::AUTOSAVE_DELAY));
    sess.save(&mut ed).unwrap();
    // Simulate a crash in the middle of the next write: a stray temp file next to the page.
    let page = Project::open(&dir).unwrap().page_path(ed.active_page_id());
    let tmp = page.with_file_name(format!(
        ".{}.123-deadbeef.tmp",
        page.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(&tmp, "{ truncated").unwrap();
    let mut ed2 = Editor::default();
    Session::new(store.clone()).open(&mut ed2, &dir).unwrap();
    assert_eq!(ed2.doc.nodes.len(), 1);
    assert!(!tmp.exists(), "stale temp files are cleaned on open");
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m1_home_actions_rename_duplicate_trash() {
    let store = temp_store("actions");
    let dirs = populate(&store);
    let mut hv = HomeView::load(&store, sketch_rs::project::now_ms());
    hv.actions = vec![
        HomeAction::Rename(dirs[0].clone(), "Q4 launch".into()),
        HomeAction::Duplicate(dirs[1].clone()),
        HomeAction::Trash(dirs[3].clone()),
    ];
    for a in std::mem::take(&mut hv.actions) {
        match a {
            HomeAction::Rename(d, n) => {
                store.rename(&d, &n).unwrap();
            }
            HomeAction::Duplicate(d) => {
                store.duplicate(&d).unwrap();
            }
            HomeAction::Trash(d) => {
                store.trash(&d).unwrap();
            }
            _ => unreachable!(),
        }
    }
    hv.refresh(&store, sketch_rs::project::now_ms());
    let mut names: Vec<String> = hv.entries.iter().map(|e| e.manifest.name.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Brand moodboard",
            "Brand moodboard copy",
            "Onboarding flow",
            "Q4 launch"
        ]
    );
    assert_eq!(hv.trash[0].manifest.name, "Empty ideas");
    std::fs::remove_dir_all(&store.root).ok();
}

/// Run the home screen for one frame with `events`, then return what the user asked for.
fn home_frame(ctx: &egui::Context, hv: &mut HomeView, events: Vec<egui::Event>) -> Vec<HomeAction> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        sketch_rs::ui::home::draw(ui, hv, false, false);
    });
    out.textures_delta.clear();
    std::mem::take(&mut hv.actions)
}

fn pointer(pos: egui::Pos2, pressed: Option<bool>) -> Vec<egui::Event> {
    let mut v = vec![egui::Event::PointerMoved(pos)];
    if let Some(pressed) = pressed {
        v.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    v
}

#[test]
fn dragging_a_card_onto_a_folder_files_it_there() {
    let store = temp_store("drag");
    let dirs = populate(&store);
    let folder = store.create_folder("Acme").unwrap();
    let mut hv = HomeView::load(&store, 1_760_000_600_000);
    let ctx = egui::Context::default();
    sketch_rs::ui::setup_style(&ctx, Default::default());
    // Two frames to settle layout and fonts.
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);

    // The first card, dropped on the only folder row and then on Recents.
    let card = egui::pos2(320.0, 150.0);
    for (target, folder) in [
        (egui::pos2(100.0, 146.0), Some(folder)),
        (egui::pos2(100.0, 47.0), None),
    ] {
        assert!(home_frame(&ctx, &mut hv, pointer(card, None)).is_empty());
        assert!(home_frame(&ctx, &mut hv, pointer(card, Some(true))).is_empty());
        let mut pos = card;
        for _ in 0..8 {
            pos = pos.lerp(target, 0.5);
            assert!(home_frame(&ctx, &mut hv, pointer(pos, None)).is_empty());
        }
        home_frame(&ctx, &mut hv, pointer(target, None));
        let actions = home_frame(&ctx, &mut hv, pointer(target, Some(false)));
        assert_eq!(actions, [HomeAction::Move(dirs[0].clone(), folder)]);
        home_frame(&ctx, &mut hv, vec![]);
    }
    std::fs::remove_dir_all(&store.root).ok();
}

/// Run one frame and return what it painted, to find a label's position.
fn home_shapes(
    ctx: &egui::Context,
    hv: &mut HomeView,
    events: Vec<egui::Event>,
) -> Vec<egui::Shape> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        sketch_rs::ui::home::draw(ui, hv, false, false);
    });
    out.textures_delta.clear();
    out.shapes.into_iter().map(|c| c.shape).collect()
}

fn find_text(shapes: &[egui::Shape], label: &str) -> Option<egui::Pos2> {
    shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
        egui::Shape::Vec(v) => find_text(v, label),
        _ => None,
    })
}

fn press(
    ctx: &egui::Context,
    hv: &mut HomeView,
    pos: egui::Pos2,
    button: egui::PointerButton,
) -> Vec<HomeAction> {
    home_frame(ctx, hv, pointer(pos, None));
    let click = |pressed| {
        vec![egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]
    };
    let mut actions = home_frame(ctx, hv, click(true));
    actions.extend(home_frame(ctx, hv, click(false)));
    actions.extend(home_frame(ctx, hv, vec![]));
    actions
}

fn settled_home(store: &Store) -> (egui::Context, HomeView) {
    let mut hv = HomeView::load(store, 1_760_000_600_000);
    let ctx = egui::Context::default();
    sketch_rs::ui::setup_style(&ctx, Default::default());
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    (ctx, hv)
}

const ENTER: egui::Key = egui::Key::Enter;

fn tap(key: egui::Key) -> Vec<egui::Event> {
    let ev = |pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    vec![ev(true), ev(false)]
}

#[test]
fn typing_a_folder_name_then_enter_saves_it() {
    let store = temp_store("folder-enter");
    let f = store.create_folder("New Folder").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    hv.renaming = Some((f.dir.clone(), f.name.clone()));
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Acme".into())]);
    let mut actions = home_frame(&ctx, &mut hv, tap(ENTER));
    actions.extend(home_frame(&ctx, &mut hv, vec![]));
    assert_eq!(
        actions,
        [HomeAction::RenameFolder(f.dir.clone(), "Acme".into())]
    );
    assert_eq!(hv.renaming, None, "the field closes");
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn clicking_away_from_a_folder_name_saves_it() {
    let store = temp_store("folder-away");
    let f = store.create_folder("New Folder").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    hv.renaming = Some((f.dir.clone(), f.name.clone()));
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Acme".into())]);
    let actions = press(
        &ctx,
        &mut hv,
        egui::pos2(700.0, 600.0),
        egui::PointerButton::Primary,
    );
    assert_eq!(
        actions,
        [HomeAction::RenameFolder(f.dir.clone(), "Acme".into())]
    );
    assert_eq!(hv.renaming, None, "the field closes");
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn enter_saves_a_project_rename() {
    let store = temp_store("project-enter");
    let dirs = populate(&store);
    let (ctx, mut hv) = settled_home(&store);
    hv.renaming = Some((dirs[0].clone(), "Launch plan".into()));
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Q4".into())]);
    let mut actions = home_frame(&ctx, &mut hv, tap(ENTER));
    actions.extend(home_frame(&ctx, &mut hv, vec![]));
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(hv.renaming, None, "the field closes");
    std::fs::remove_dir_all(&store.root).ok();
}

/// The app's handling of the actions these tests care about.
fn apply(store: &Store, hv: &mut HomeView, actions: Vec<HomeAction>) {
    for a in actions {
        match a {
            HomeAction::NewFolder(project) => {
                let f = store.create_folder("New Folder").unwrap();
                if let Some(d) = project {
                    store.move_project(&d, Some(&f)).unwrap();
                }
                hv.name_new_folder(f);
            }
            HomeAction::RenameFolder(d, n) => {
                store.rename_folder(&d, &n).unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    hv.refresh(store, 1_760_000_600_000);
}

#[test]
fn the_plus_button_makes_a_folder_you_can_name_straight_away() {
    let store = temp_store("plus");
    let (ctx, mut hv) = settled_home(&store);
    let plus = egui::pos2(178.0, 117.0);
    let actions = press(&ctx, &mut hv, plus, egui::PointerButton::Primary);
    assert_eq!(actions, [HomeAction::NewFolder(None)]);
    apply(&store, &mut hv, actions);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Acme".into())]);
    let actions = home_frame(&ctx, &mut hv, tap(ENTER));
    apply(&store, &mut hv, actions);
    home_frame(&ctx, &mut hv, vec![]);

    let folders = store.folders();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "Acme", "typing replaced the placeholder");
    assert_eq!(hv.selected_folder(), Some(&folders[0]));
    assert_eq!(hv.renaming, None);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn move_to_can_start_a_new_folder() {
    let store = temp_store("menu-new");
    let dirs = populate(&store);
    let (ctx, mut hv) = settled_home(&store);
    press(
        &ctx,
        &mut hv,
        egui::pos2(320.0, 150.0),
        egui::PointerButton::Secondary,
    );
    let actions = pick_move_to(&ctx, &mut hv, "New Folder…");
    assert_eq!(actions, [HomeAction::NewFolder(Some(dirs[0].clone()))]);
    apply(&store, &mut hv, actions);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Acme".into())]);
    let actions = home_frame(&ctx, &mut hv, tap(ENTER));
    apply(&store, &mut hv, actions);

    let list = store.list();
    let moved = list
        .iter()
        .find(|e| e.manifest.name == "Launch plan")
        .unwrap();
    assert_eq!(moved.folder.as_ref().unwrap().name, "Acme");
    std::fs::remove_dir_all(&store.root).ok();
}

fn editor_frame(ctx: &egui::Context, ed: &mut Editor, st: &mut UiState, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    frame::layout(ctx, input, ed, st)
        .output
        .textures_delta
        .clear();
}

#[test]
fn enter_saves_a_page_and_a_layer_rename() {
    let ctx = egui::Context::default();
    sketch_rs::ui::setup_style(&ctx, Default::default());
    let mut ed = demo::showcase();
    let mut st = UiState::default();
    st.pages.renaming = Some((0, ed.pages[0].name.clone()));
    for _ in 0..3 {
        editor_frame(&ctx, &mut ed, &mut st, vec![]);
    }
    editor_frame(
        &ctx,
        &mut ed,
        &mut st,
        vec![egui::Event::Text("Board".into())],
    );
    editor_frame(&ctx, &mut ed, &mut st, tap(ENTER));
    editor_frame(&ctx, &mut ed, &mut st, vec![]);
    assert_eq!(st.pages.renaming, None);
    assert_eq!(ed.pages[0].name, "Board");

    let id = ed.doc.root[0];
    st.renaming = Some((id, ed.doc.get(id).unwrap().name.clone()));
    for _ in 0..3 {
        editor_frame(&ctx, &mut ed, &mut st, vec![]);
    }
    editor_frame(
        &ctx,
        &mut ed,
        &mut st,
        vec![egui::Event::Text("Hero".into())],
    );
    editor_frame(&ctx, &mut ed, &mut st, tap(ENTER));
    editor_frame(&ctx, &mut ed, &mut st, vec![]);
    assert_eq!(st.renaming, None);
    assert_eq!(ed.doc.get(id).unwrap().name, "Hero");
}

#[test]
fn escape_cancels_a_folder_rename() {
    let store = temp_store("folder-esc");
    let f = store.create_folder("New Folder").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    hv.renaming = Some((f.dir.clone(), f.name.clone()));
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![egui::Event::Text("Acme".into())]);
    let mut actions = home_frame(&ctx, &mut hv, tap(egui::Key::Escape));
    actions.extend(home_frame(&ctx, &mut hv, vec![]));
    assert_eq!(actions, []);
    assert_eq!(hv.renaming, None, "the field closes");
    std::fs::remove_dir_all(&store.root).ok();
}

/// Open a card's menu, go to Move to, and pick `folder`.
fn pick_move_to(ctx: &egui::Context, hv: &mut HomeView, folder: &str) -> Vec<HomeAction> {
    let shapes = home_shapes(ctx, hv, vec![]);
    let to = find_text(&shapes, "Move to").expect("the menu lists Move to");
    home_frame(ctx, hv, pointer(to, None));
    let mut item = None;
    for _ in 0..30 {
        let shapes = home_shapes(ctx, hv, vec![egui::Event::PointerMoved(to)]);
        // The sidebar paints the same name, so take the copy furthest right.
        item = find_all(&shapes, folder)
            .into_iter()
            .max_by(|a, b| a.x.total_cmp(&b.x));
        if item.is_some_and(|p| p.x > to.x) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let item = item
        .filter(|p| p.x > to.x)
        .expect("the submenu lists the folder");
    press(ctx, hv, item, egui::PointerButton::Primary)
}

fn find_all(shapes: &[egui::Shape], label: &str) -> Vec<egui::Pos2> {
    let mut out = Vec::new();
    for s in shapes {
        match s {
            egui::Shape::Text(t) if t.galley.text() == label => {
                out.push(t.visual_bounding_rect().center());
            }
            egui::Shape::Vec(v) => out.extend(find_all(v, label)),
            _ => {}
        }
    }
    out
}

#[test]
fn right_click_move_to_files_a_project() {
    let store = temp_store("move-menu");
    let dirs = populate(&store);
    let f = store.create_folder("Acme").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    press(
        &ctx,
        &mut hv,
        egui::pos2(320.0, 150.0),
        egui::PointerButton::Secondary,
    );
    let actions = pick_move_to(&ctx, &mut hv, "Acme");
    assert_eq!(actions, [HomeAction::Move(dirs[0].clone(), Some(f))]);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn move_to_works_with_a_folder_name_still_being_typed() {
    let store = temp_store("move-while-naming");
    let dirs = populate(&store);
    let f = store.create_folder("New Folder").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    hv.renaming = Some((f.dir.clone(), f.name.clone()));
    home_frame(&ctx, &mut hv, vec![]);
    home_frame(&ctx, &mut hv, vec![]);
    press(
        &ctx,
        &mut hv,
        egui::pos2(320.0, 150.0),
        egui::PointerButton::Secondary,
    );
    let actions = pick_move_to(&ctx, &mut hv, "New Folder");
    assert_eq!(actions, [HomeAction::Move(dirs[0].clone(), Some(f))]);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn the_more_button_move_to_files_a_project() {
    let store = temp_store("more-menu");
    let dirs = populate(&store);
    let f = store.create_folder("Acme").unwrap();
    let (ctx, mut hv) = settled_home(&store);
    // The ⋯ button on the first card.
    press(
        &ctx,
        &mut hv,
        egui::pos2(441.0, 235.0),
        egui::PointerButton::Primary,
    );
    let actions = pick_move_to(&ctx, &mut hv, "Acme");
    assert_eq!(actions, [HomeAction::Move(dirs[0].clone(), Some(f))]);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn folder_view_shows_its_projects_and_follows_the_store() {
    let store = temp_store("folders");
    let dirs = populate(&store);
    let Some(g) = gpu() else { return };
    for d in &dirs {
        let p = Project::open(d).unwrap();
        let doc = &p.load_pages().unwrap()[0].doc;
        if let Some(png) = session::render_thumbnail(g, doc).unwrap() {
            std::fs::write(p.thumbnail_path(), png).unwrap();
        }
    }
    let acme = store.create_folder("Acme").unwrap();
    store.create_folder("Globex").unwrap();
    store
        .create_folder("A folder with a very long name indeed")
        .unwrap();
    store.move_project(&dirs[0], Some(&acme)).unwrap();
    store.move_project(&dirs[1], Some(&acme)).unwrap();

    let mut st = UiState {
        home: Some(HomeView::load(&store, 1_760_000_600_000)),
        ..Default::default()
    };
    let hv = st.home.as_ref().unwrap();
    assert_eq!(hv.folders.len(), 3);
    assert_eq!(hv.entries.len(), 4, "Recents shows every project");

    let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".artifacts");
    std::fs::create_dir_all(&evidence).unwrap();
    let mut ed = Editor::default();
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(evidence.join("home_folders_recents.png")).unwrap();

    // Each capture has its own egui context, so start from a view with no cached thumbnails.
    let mut hv = HomeView::load(&store, 1_760_000_600_000);
    hv.folder = Some(acme.id.clone());
    st.home = Some(hv);
    let img = frame::capture(g, &mut ed, &mut st, [1280.0, 800.0], 2.0, |_| {});
    img.save(evidence.join("home_folders_acme.png")).unwrap();
    let card = Rect::new(440.0, 136.0, 464.0, 296.0);
    assert!(count(&img, card, |p| p[0] < 200 || p[2] < 200) > 200);
    // The folder holds two cards, so the third column is empty.
    let third = Rect::new(1460.0, 140.0, 464.0, 296.0);
    assert_eq!(count(&img, third, |p| p[0] < 200 || p[2] < 200), 0);

    // New projects go in the selected folder through the same path as the app.
    let hv = st.home.as_mut().unwrap();
    let f = hv.selected_folder().cloned();
    assert_eq!(f.as_ref().map(|f| f.name.as_str()), Some("Acme"));
    store.create_in("Inside", f.as_ref()).unwrap();

    // A folder that goes away (say, an agent or Finder) falls back to Recents.
    store.trash_folder(&acme.dir).unwrap();
    hv.refresh(&store, 1_760_000_600_000);
    assert_eq!(hv.folder, None);
    assert_eq!(hv.trash.len(), 3);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m2_pages_keep_separate_canvases_history_and_cameras() {
    let mut ed = editor(1000.0, 700.0);
    ed.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 100.0, 100.0)),
        None,
    );
    ed.camera.pan = v2(-50.0, -50.0);
    let p2 = ed.add_page();
    assert_eq!(ed.active_page, p2);
    assert!(
        ed.doc.nodes.is_empty(),
        "a new page is a fresh infinite canvas"
    );
    ed.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Ellipse,
            Rect::new(5000.0, -3000.0, 10.0, 10.0),
        ),
        None,
    );
    ed.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Ellipse,
            Rect::new(-90000.0, 7e5, 10.0, 10.0),
        ),
        None,
    );
    ed.switch_page(0);
    assert_eq!(ed.doc.nodes.len(), 1);
    assert_eq!(ed.camera.pan, v2(-50.0, -50.0), "camera is per page");
    ed.undo();
    assert!(ed.doc.nodes.is_empty(), "undo is per page");
    ed.switch_page(1);
    assert_eq!(ed.doc.nodes.len(), 2);
    ed.undo();
    assert_eq!(ed.doc.nodes.len(), 1);
    ed.redo();
    assert_eq!(ed.doc.nodes.len(), 2);
    let b = ed.doc.content_bounds().unwrap();
    assert!(
        b.w > 90000.0 && b.h > 7e5,
        "content spans a huge canvas: {b:?}"
    );

    ed.rename_page(1, "Far away");
    let dup = ed.duplicate_page(1);
    assert_eq!(ed.pages[dup].name, "Far away copy");
    assert_eq!(ed.doc.nodes.len(), 2);
    ed.move_page(dup, 0);
    assert_eq!(ed.pages[0].name, "Far away copy");
    assert_eq!(ed.active_page, 0);
    assert!(ed.delete_page(0).is_some());
    assert_eq!(ed.pages.len(), 2);
    assert_eq!(ed.page_name(), "Page 1");
    assert!(ed.delete_page(0).is_some());
    assert!(
        ed.delete_page(0).is_none(),
        "the last page can't be deleted"
    );
}

#[test]
fn m2_copy_paste_between_pages_and_projects() {
    let mut ed = editor(1000.0, 700.0);
    let mut n = Node::new(NodeId(0), NodeKind::Rect, Rect::new(10.0, 20.0, 30.0, 40.0));
    n.style.set_fill(Some(Color::rgb(200, 10, 10)));
    let id = ed.add_node(n, None);
    ed.selection = vec![id];
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('c'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    ed.add_page();
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('v'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    assert_eq!(ed.doc.nodes.len(), 1);
    let pasted = ed.doc.get(ed.selection[0]).unwrap();
    assert_eq!(pasted.style.fill(), Some(Color::rgb(200, 10, 10)));
    assert_eq!(
        pasted.rect.min(),
        v2(10.0, 20.0),
        "paste on another page keeps the position"
    );

    // Another project: the clipboard survives opening a different project.
    let store = temp_store("clip");
    let mut sess = Session::new(store.clone());
    sess.create(&mut ed, "Other").unwrap();
    assert!(ed.doc.nodes.is_empty());
    key(
        &mut ed,
        sketch_rs::editor::Key::Char('v'),
        sketch_rs::editor::Modifiers::cmd(),
    );
    assert_eq!(ed.doc.nodes.len(), 1);
    std::fs::remove_dir_all(&store.root).ok();
}

#[test]
fn m2_pages_panel_screenshot() {
    let Some(g) = gpu() else { return };
    let page = |name: &str, doc: sketch_rs::model::Document| PageData {
        id: sketch_rs::project::new_id(),
        name: name.into(),
        doc,
        camera: None,
    };
    let mut ed = Editor::default();
    ed.open_pages(
        vec![
            page("Launch plan", demo::showcase().doc),
            page("Moodboard", demo::moodboard().doc),
            page("Flows", demo::flow().doc),
            page("Archive", sketch_rs::model::Document::default()),
        ],
        None,
    );
    ed.switch_page(2);
    assert_eq!(ed.page_name(), "Flows");
    let mut st = UiState {
        project_name: "Q4 launch".into(),
        save_state: "Saved".into(),
        ..Default::default()
    };
    let img = frame::capture(g, &mut ed, &mut st, [1440.0, 900.0], 2.0, |ed| {
        ed.zoom_to_fit()
    });
    img.save(shot("milestone2_pages.png")).unwrap();
    // The canvas shows the Flows page: its blue buttons are on screen.
    let blue = Color::rgb(0x2f, 0x6f, 0xf3);
    assert!(
        count(&img, Rect::new(260.0, 60.0, 900.0, 800.0), |p| close(
            p, blue, 12
        )) > 500
    );
}

#[test]
fn instances_of_mains_on_other_pages_keep_their_token_mode_on_reopen() {
    use sketch_rs::model::{TokenKind, TokenProp};
    let store = temp_store("xpage");
    let mut sess = Session::new(store.clone());
    let mut ed = Editor::default();
    sess.create(&mut ed, "Cross page").unwrap();
    let dir = sess.project.as_ref().unwrap().dir.clone();
    let (dark, fg) = ed.edit_tokens("add", None, |t| {
        let d = t.add_mode();
        let dark = [(d.clone(), "#000000".to_string())];
        let fg = t.create_in_modes(TokenKind::Color, "fg", "#ffffff", &dark, None);
        (d, fg.unwrap())
    });
    let f = ed.add_node(
        Node::new(
            NodeId(0),
            NodeKind::Frame { clip: true },
            Rect::new(0.0, 0.0, 80.0, 40.0),
        ),
        None,
    );
    let r = ed.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(5.0, 5.0, 10.0, 10.0)),
        Some(f),
    );
    ed.bind_token(&[r], TokenProp::Fill, &fg);
    ed.selection = vec![f];
    ed.create_component();
    let cmd = sketch_rs::editor::Modifiers::cmd();
    key(&mut ed, sketch_rs::editor::Key::Char('c'), cmd);
    ed.add_page();
    key(&mut ed, sketch_rs::editor::Key::Char('v'), cmd);
    let inst = ed.selection[0];
    // Dark on the instance, as the inspector sets it.
    ed.edit("mode", |tx| {
        let n = tx.node_mut(inst).unwrap();
        n.mode = Some(dark.clone());
        n.instance.as_mut().unwrap().overrides.push("mode".into());
    });
    let leaf = *ed.doc.subtree(inst).last().unwrap();
    let black = Some(Color::rgb(0, 0, 0));
    assert_eq!(ed.doc.get(leaf).unwrap().style.fill(), black);
    sess.save(&mut ed).unwrap();
    assert!(
        ed.dirty_pages().is_empty(),
        "a sync that ends where it began saves nothing"
    );
    ed.switch_page(0);
    ed.switch_page(1);
    assert!(ed.dirty_pages().is_empty());

    let mut ed2 = Editor::default();
    Session::new(store.clone()).open(&mut ed2, &dir).unwrap();
    assert_eq!(ed2.active_page, 1, "opens on the instance's page");
    let n = ed2.doc.get(leaf).unwrap();
    assert_eq!(n.style.fill(), black);
    assert_eq!(n.tokens.get(&TokenProp::Fill), Some(&fg));
    std::fs::remove_dir_all(&store.root).ok();
}
