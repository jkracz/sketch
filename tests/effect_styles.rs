use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, Effect, EffectKind, NodeId, StyleEffect, TokenKind, TokenProp};
use sketch_rs::project::Store;
use sketch_rs::session::Session;

fn store() -> Store {
    let dir = std::env::temp_dir().join(format!("sketch-rs-effects-{:016x}", fastrand::u64(..)));
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

fn shadow(y: f64, blur: f64) -> Effect {
    Effect {
        y,
        blur,
        ..Effect::new(EffectKind::DropShadow)
    }
}

/// `--shade` (black, white in Dark), `Elevation/2` (two shadows, the first in `--shade`, and
/// a layer blur) and `Glass` (a background blur). Returns the ids of the two styles.
fn seed(h: &mut exec::SessionHost) -> (String, String) {
    h.ed.edit_tokens("seed", None, |t| {
        let dark = vec![("dark".to_string(), "#ffffff".to_string())];
        let shade = t
            .create_in_modes(TokenKind::Color, "shade", "#000000", &dark, None)
            .unwrap();
        let effects = vec![
            StyleEffect {
                effect: shadow(1.0, 2.0),
                color_token: Some(shade),
            },
            StyleEffect::literal(Effect {
                color: Color::rgb(255, 0, 0),
                spread: 1.0,
                ..shadow(4.0, 8.0)
            }),
            StyleEffect::literal(Effect {
                blur: 3.0,
                ..Effect::new(EffectKind::LayerBlur)
            }),
        ];
        let elevation = t
            .create_effect_style("Elevation/2", effects, Some("Raised cards"))
            .unwrap();
        let glass = vec![StyleEffect::literal(Effect::new(
            EffectKind::BackgroundBlur,
        ))];
        let glass = t.create_effect_style("Glass", glass, None).unwrap();
        (elevation, glass)
    })
}

fn artboard(h: &mut exec::SessionHost) -> Value {
    call(
        h,
        "create_artboard",
        json!({"name": "Page", "styles": {"width": "600px", "height": "400px"}}),
    )["id"]
        .clone()
}

fn write(h: &mut exec::SessionHost, ab: &Value, html: &str) -> Vec<Value> {
    let r = call(
        h,
        "write_html",
        json!({"targetNodeId": ab, "mode": "insert-children", "html": html}),
    );
    r["createdNodeIds"].as_array().unwrap().clone()
}

fn effects(h: &exec::SessionHost, id: NodeId) -> Vec<Effect> {
    h.ed.doc.get(id).unwrap().style.effects.clone()
}

fn bound(h: &exec::SessionHost, id: NodeId) -> bool {
    h.ed.bound_effect_style(h.ed.doc.get(id).unwrap()).is_some()
}

const CARD: &str = "<div style=\"width: 100px; height: 60px; background: #fff; \
                    box-shadow: var(--elevation-2)\"></div>";

#[test]
fn get_tokens_lists_styles_in_short_form() {
    let store = store();
    let mut h = host(&store, "Effects");
    seed(&mut h);
    let all = call(&mut h, "get_tokens", json!({}));
    let styles = all["effectStyles"].as_array().unwrap();
    assert_eq!(styles.len(), 2);
    assert_eq!(styles[0]["name"], "Elevation/2");
    assert_eq!(styles[0]["var"], "--elevation-2");
    assert_eq!(
        styles[0]["css"],
        "box-shadow: 0px 1px 2px var(--shade), 0px 4px 8px 1px #ff0000; filter: blur(3px)"
    );
    assert_eq!(styles[0]["description"], "Raised cards");
    assert_eq!(styles[1]["css"], "backdrop-filter: blur(8px)");
    let only = call(&mut h, "get_tokens", json!({"types": ["effectStyle"]}));
    assert!(only["tokens"].as_array().unwrap().is_empty());
    assert_eq!(only["effectStyles"].as_array().unwrap().len(), 2);
    let none = call(&mut h, "get_tokens", json!({"types": ["color"]}));
    assert!(none.get("effectStyles").is_none(), "{none}");
    let pattern = call(&mut h, "get_tokens", json!({"namePattern": "--elev*"}));
    assert_eq!(pattern["effectStyles"].as_array().unwrap().len(), 1);
}

#[test]
fn css_export_writes_a_variable_per_style_in_every_mode_that_changes_it() {
    let store = store();
    let mut h = host(&store, "Effects");
    seed(&mut h);
    let css = call(&mut h, "get_tokens", json!({"format": "css"}));
    let css = css.as_str().unwrap();
    let shadows = "  --elevation-2: 0px 1px 2px var(--shade), 0px 4px 8px 1px #ff0000;\n";
    let root = css.split("[data-theme=\"dark\"] {").next().unwrap();
    assert!(root.contains(shadows), "{css}");
    assert!(root.contains("  --elevation-2-blur: blur(3px);\n"), "{css}");
    assert!(root.contains("  --glass: none;\n"), "{css}");
    assert!(
        root.contains("  --glass-backdrop-blur: blur(8px);\n"),
        "{css}"
    );
    // `var(--shade)` resolves where the variable is set, so Dark sets it again.
    let dark = css.split("[data-theme=\"dark\"] {").nth(1).unwrap();
    assert!(dark.contains(shadows), "{css}");
    assert!(!dark.contains("--glass"), "{css}");
}

#[test]
fn a_variable_in_box_shadow_binds_the_style() {
    let store = store();
    let mut h = host(&store, "Effects");
    let (elevation, _) = seed(&mut h);
    let ab = artboard(&mut h);
    let html = format!(
        "{CARD}<p style=\"text-shadow: var(--elevation-2)\">Text</p>\
         <div style=\"width: 50px; height: 50px; backdrop-filter: var(--glass-backdrop-blur)\"></div>\
         <div style=\"width: 50px; height: 50px; box-shadow: var(--elevation-2), 0 0 0 1px blue\"></div>"
    );
    let ids = write(&mut h, &ab, &html);
    let card = node(&ids[0]);
    let n = h.ed.doc.get(card).unwrap();
    assert_eq!(n.effect_style.as_ref(), Some(&elevation));
    assert!(
        n.tokens.is_empty(),
        "the style owns the color: {:?}",
        n.tokens
    );
    let e = effects(&h, card);
    assert_eq!(e.len(), 3, "shadows and the blur");
    assert_eq!((e[0].color, e[1].spread), (Color::BLACK, 1.0));
    assert_eq!(e[2].kind, EffectKind::LayerBlur);

    assert!(bound(&h, node(&ids[1])), "text takes it from text-shadow");
    assert_eq!(
        effects(&h, node(&ids[2]))[0].kind,
        EffectKind::BackgroundBlur
    );
    assert!(bound(&h, node(&ids[2])), "a blur variable binds too");
    // Inside a longer list the variable is only its value.
    assert!(!bound(&h, node(&ids[3])));
    assert_eq!(effects(&h, node(&ids[3])).len(), 3);

    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [ids[0]]}));
    let st = &st[ids[0].as_str().unwrap()];
    assert_eq!(st["effectStyle"], "elevation-2");
    assert_eq!(st["boxShadow"], "var(--elevation-2)");
    assert_eq!(st["filter"], "var(--elevation-2-blur)");
    let jsx = call(&mut h, "get_jsx", json!({"nodeId": ids[0]}));
    let jsx = jsx.as_str().unwrap();
    assert!(jsx.contains("boxShadow: \"var(--elevation-2)\""), "{jsx}");
    assert!(!jsx.contains("effectStyle"), "{jsx}");
    // The second shadow has spread, which `text-shadow` can't take, so text shows values.
    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [ids[1]]}));
    let st = &st[ids[1].as_str().unwrap()];
    assert_eq!(st["effectStyle"], "elevation-2");
    assert_eq!(
        st["textShadow"],
        "0px 1px 2px var(--shade), 0px 4px 8px #ff0000"
    );
}

#[test]
fn a_layer_in_a_dark_frame_shows_the_dark_color() {
    let store = store();
    let mut h = host(&store, "Effects");
    seed(&mut h);
    let ab = artboard(&mut h);
    let html = format!("<div data-theme=\"dark\" style=\"display: flex\">{CARD}</div>");
    let ids = write(&mut h, &ab, &html);
    let card = h.ed.doc.get(node(&ids[0])).unwrap().children[0];
    assert_eq!(effects(&h, card)[0].color, Color::WHITE);
    assert!(bound(&h, card));
}

#[test]
fn update_styles_applies_by_name_and_a_value_detaches() {
    let store = store();
    let mut h = host(&store, "Effects");
    let (elevation, _) = seed(&mut h);
    let ab = artboard(&mut h);
    let plain = "<div style=\"width: 40px; height: 40px; background: #fff\"></div>";
    let ids = write(&mut h, &ab, &format!("{plain}{plain}"));
    let ids = Value::Array(ids);
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": ids, "styles": {"effectStyle": "Elevation/2"}}]}),
    );
    assert!(r.get("ignoredStyles").is_none(), "{r}");
    let (a, b) = (node(&ids[0]), node(&ids[1]));
    assert!(bound(&h, a) && bound(&h, b));

    // One style edit reaches both layers and undoes in one step.
    let steps = h.ed.history.undo_len();
    h.ed.edit_tokens("Edit effect style", None, |t| {
        let mut list = t.effect_style(&elevation).unwrap().effects.clone();
        list[0].effect.blur = 20.0;
        t.update_effect_style(&elevation, None, Some(list), None)
    })
    .unwrap();
    assert_eq!(h.ed.history.undo_len(), steps + 1);
    assert_eq!(
        (effects(&h, a)[0].blur, effects(&h, b)[0].blur),
        (20.0, 20.0)
    );
    call(&mut h, "undo", json!({}));
    assert_eq!((effects(&h, a)[0].blur, effects(&h, b)[0].blur), (2.0, 2.0));

    // A shadow set by value replaces the shadows, keeps the blur and drops the style.
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ids[1]], "styles": {"boxShadow": "0 0 4px red"}}]}),
    );
    assert!(!bound(&h, b));
    assert_eq!(effects(&h, b).len(), 2);
    assert!(bound(&h, a));

    // In one update the style goes on first, so a value next to it wins.
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ids[1]], "styles":
            {"filter": "blur(9px)", "effectStyle": "--elevation-2"}}]}),
    );
    assert!(!bound(&h, b));
    let e = effects(&h, b);
    assert_eq!((e.len(), e[2].blur), (3, 9.0));

    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ids[1]], "styles": {"effectStyle": "nope"}}]}),
    );
    assert!(
        r["ignoredStyles"].to_string().contains("effectStyle"),
        "{r}"
    );
}

#[test]
fn detaching_in_the_app_keeps_the_color_token() {
    let store = store();
    let mut h = host(&store, "Effects");
    seed(&mut h);
    let ab = artboard(&mut h);
    let ids = write(&mut h, &ab, CARD);
    let card = node(&ids[0]);
    h.ed.detach_effect_style(&[card]);
    assert!(!bound(&h, card));
    let n = h.ed.doc.get(card).unwrap();
    assert!(n.tokens.contains_key(&TokenProp::EffectColor(0)));
    let st = call(&mut h, "get_computed_styles", json!({"nodeIds": [ids[0]]}));
    let st = &st[ids[0].as_str().unwrap()];
    assert!(st.get("effectStyle").is_none(), "{st}");
    assert_eq!(
        st["boxShadow"],
        "0px 1px 2px var(--shade), 0px 4px 8px 1px #ff0000"
    );
}

#[test]
fn styles_save_with_the_project() {
    let store = store();
    let mut h = host(&store, "Effects");
    seed(&mut h);
    let ab = artboard(&mut h);
    let ids = write(&mut h, &ab, CARD);
    let id = node(&ids[0]);
    call(&mut h, "finish_working_on_nodes", json!({}));
    let dir = h.session.project.as_ref().unwrap().dir.clone();

    let mut s = Session::new(store.clone());
    let mut ed = sketch_rs::editor::Editor::default();
    s.open(&mut ed, &dir).unwrap();
    assert_eq!(ed.tokens.effect_styles().len(), 2);
    assert_eq!(ed.tokens.effect_styles(), h.ed.tokens.effect_styles());
    assert!(ed.bound_effect_style(ed.doc.get(id).unwrap()).is_some());
}

#[test]
fn design_systems_carry_styles_with_the_prefix() {
    let store = store();
    let mut src = host(&store, "Acme");
    let (elevation, glass) = seed(&mut src);
    let r = call(&mut src, "publish_design_system", json!({}));
    assert!(r.to_string().contains("--acme-elevation-2"), "{r}");

    let mut h = host(&store, "Landing");
    let list = call(&mut h, "list_design_systems", json!({}));
    assert_eq!(list["systems"][0]["effectStyles"], 2);
    let text = call(&mut h, "get_design_system", json!({"id": "acme"}));
    let text = text.as_str().unwrap();
    assert!(text.contains("## Effect styles"), "{text}");
    assert!(
        text.contains("--acme-elevation-2: box-shadow: 0px 1px 2px var(--acme-shade)"),
        "{text}"
    );
    let json = call(
        &mut h,
        "get_design_system",
        json!({"id": "acme", "format": "json", "types": ["effectStyle"]}),
    );
    assert!(json["tokens"].as_array().unwrap().is_empty());
    assert_eq!(json["effectStyles"][0]["var"], "--acme-elevation-2");
    let css = call(
        &mut h,
        "get_design_system",
        json!({"id": "acme", "format": "css"}),
    );
    assert!(
        css.as_str()
            .unwrap()
            .contains("--acme-glass-backdrop-blur: blur(8px);"),
        "{css}"
    );

    call(&mut h, "use_design_system", json!({"id": "acme"}));
    let own = call(&mut h, "get_tokens", json!({}));
    assert_eq!(own["designSystems"][0]["effectStyles"], 2);
    let ab = artboard(&mut h);
    let html =
        "<div style=\"width: 100px; height: 60px; box-shadow: var(--acme-elevation-2)\"></div>";
    let ids = write(&mut h, &ab, html);
    let card = node(&ids[0]);
    assert!(bound(&h, card));
    assert_eq!(effects(&h, card)[0].blur, 2.0);
    let linked = h.ed.doc.get(card).unwrap().effect_style.clone().unwrap();
    let locked = h.ed.edit_tokens("edit", None, |t| {
        t.update_effect_style(&linked, Some("Mine"), None, None)
    });
    assert!(locked.unwrap_err().contains("design system"));

    // A new version changes one style and drops the other.
    src.ed.edit_tokens("edit", None, |t| {
        let mut list = t.effect_style(&elevation).unwrap().effects.clone();
        list[0].effect.blur = 30.0;
        t.update_effect_style(&elevation, None, Some(list), None)
            .unwrap();
        t.delete_effect_styles(&[glass]);
    });
    let r = call(&mut src, "publish_design_system", json!({}));
    let r = r.to_string();
    assert!(
        r.contains("--acme-elevation-2") && r.contains("--acme-glass"),
        "{r}"
    );
    let updates = sketch_rs::systems::update_all(&store, &mut h.ed);
    assert!(updates[0].contains("--acme-glass"), "{updates:?}");
    assert_eq!(effects(&h, card)[0].blur, 30.0);
    assert_eq!(h.ed.tokens.effect_styles().len(), 1);

    // Detached, the style is local and keeps its variable and its layers.
    let system = h.ed.tokens.systems()[0].id.clone();
    h.ed.detach_system(&system).unwrap();
    let s =
        h.ed.tokens
            .effect_style_by_name("--acme-elevation-2")
            .unwrap();
    assert!(s.system.is_none());
    assert!(bound(&h, card));
}
