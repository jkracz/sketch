//! Main components as a design system publishes them.
//!
//! Every main component on every page goes into the registry entry with its layer tree.
//! Layers keep their ids from the source page, so a file that uses the system can name each
//! one `<system>/<component>/<node>` and keep its instances' overrides when the system is
//! published again.
//!
//! Bindings follow the rest of the entry: the file's own tokens, styles and modes keep their
//! local ids, which the using file links (`linked_id`) the way it links token aliases.
//! Bindings to another design system become the plain values they show, the same way token
//! aliases are flattened.

use crate::editor::Editor;
use crate::model::{Document, Node, NodeId, TokenSet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One main component and its layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedComponent {
    /// `<page id>-<node id>` of the main in the source file (`component_id`). It stays the
    /// same across publishes while the main stays on its page.
    pub id: String,
    pub name: String,
    /// The main, then its layers depth first, each with its id from the source page. The
    /// main has no parent and sits at 0,0, so moving it on the canvas changes nothing here.
    /// It keeps its own mode but not one a frame around it picks, the same as its instances.
    pub nodes: Vec<Node>,
    /// Layers of instances nested inside this component, by node id: the published
    /// component their `instance.main` is a layer of.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nested: BTreeMap<NodeId, String>,
}

impl PublishedComponent {
    /// Image assets its layers use.
    pub fn assets(&self) -> Vec<String> {
        crate::assets::used_by_nodes(&self.nodes)
    }
}

pub fn component_id(page: &str, main: NodeId) -> String {
    format!("{page}-{}", main.0)
}

/// Every main component in the open project, on every page, as it would publish now. Sorted
/// by name, so reordering layers or pages changes nothing.
pub fn publishable(ed: &Editor) -> Vec<PublishedComponent> {
    let mut out: Vec<PublishedComponent> = (0..ed.pages.len())
        .flat_map(|i| {
            let doc = ed.page_doc(i);
            let page = ed.pages[i].id.as_str();
            (doc.nodes.values())
                .filter(|n| n.component)
                .map(move |m| publish_one(doc, page, m.id, &ed.tokens))
        })
        .collect();
    out.sort_by(|a, b| {
        (a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.id.cmp(&b.id))
    });
    out
}

fn publish_one(doc: &Document, page: &str, main: NodeId, tokens: &TokenSet) -> PublishedComponent {
    let mut nodes = Vec::new();
    let mut nested = BTreeMap::new();
    let mut stack = vec![main];
    while let Some(id) = stack.pop() {
        let Some(src) = doc.get(id) else { continue };
        let mut n = src.clone();
        stack.extend(n.children.iter().rev());
        if id == main {
            n.parent = None;
            n.rect.x = 0.0;
            n.rect.y = 0.0;
            n.locked = false;
        } else {
            // A main inside another main publishes on its own; here it's plain layers.
            n.component = false;
        }
        flatten_bindings(&mut n, tokens);
        if let Some(l) = &n.instance {
            match owner(doc, l.main) {
                Some(c) => {
                    nested.insert(id, component_id(page, c));
                }
                // Its main is gone, so it keeps its look as plain layers.
                None => n.instance = None,
            }
        }
        nodes.push(n);
    }
    PublishedComponent {
        id: component_id(page, main),
        name: doc.get(main).map(|m| m.name.clone()).unwrap_or_default(),
        nodes,
        nested,
    }
}

/// The main component that `id` is a layer of: itself or its nearest main ancestor.
fn owner(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut cur = doc.get(id);
    while let Some(n) = cur {
        if n.component {
            return Some(n.id);
        }
        cur = n.parent.and_then(|p| doc.get(p));
    }
    None
}

/// Drop bindings to anything that isn't this file's own: another system's tokens, styles and
/// modes, or ones that no longer exist. The layer's fields already hold the values they show.
fn flatten_bindings(n: &mut Node, tokens: &TokenSet) {
    n.tokens
        .retain(|_, t| tokens.get(t).is_some_and(|t| t.system.is_none()));
    if (n.text_style_ref.as_ref())
        .is_some_and(|r| tokens.style(&r.id).is_none_or(|s| s.system.is_some()))
    {
        n.text_style_ref = None;
    }
    if (n.effect_style.as_deref())
        .is_some_and(|id| tokens.effect_style(id).is_none_or(|s| s.system.is_some()))
    {
        n.effect_style = None;
    }
    if (n.mode.as_deref()).is_some_and(|m| !tokens.modes().iter().any(|x| x.id == m)) {
        n.mode = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{publish_open, source, temp_store};
    use super::super::{
        Contents, PublishRequest, Registry, changes, has_unpublished_changes, unpublish,
    };
    use super::*;
    use crate::geom::{Rect, v2};
    use crate::model::{Color, NodeKind, Paint, TokenKind, TokenProp};
    use crate::session::Session;

    fn add(ed: &mut Editor, kind: NodeKind, r: Rect, parent: Option<NodeId>) -> NodeId {
        ed.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(Node::new(id, kind, r), parent, None)
        })
    }

    /// A main component "name" at `at` with one rectangle inside. Returns (main, rect).
    fn component(ed: &mut Editor, name: &str, at: f64) -> (NodeId, NodeId) {
        let frame = NodeKind::Frame { clip: true };
        let main = add(ed, frame, Rect::new(at, 0.0, 100.0, 40.0), None);
        let rect = add(
            ed,
            NodeKind::Rect,
            Rect::new(10.0, 10.0, 20.0, 20.0),
            Some(main),
        );
        ed.edit("name", |tx| tx.node_mut(main).unwrap().name = name.into());
        ed.select(vec![main]);
        ed.create_component();
        (main, rect)
    }

    fn set_fill(ed: &mut Editor, id: NodeId, c: Color) {
        ed.edit("fill", |tx| {
            tx.node_mut(id).unwrap().style.fills = vec![Paint::solid(c)]
        });
    }

    fn by_name<'a>(list: &'a [PublishedComponent], name: &str) -> &'a PublishedComponent {
        list.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn mains_on_every_page_publish_and_only_their_content_counts_as_a_change() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let (button, rect) = component(&mut ed, "Button", 300.0);
        let first_page = ed.pages[0].id.clone();
        ed.add_page();
        let (card, _) = component(&mut ed, "Card", 0.0);
        let r = publish_open(&s, &mut ed, &PublishRequest::default());

        let names: Vec<&str> = r
            .system
            .components
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["Button", "Card"]);
        assert!(
            r.added.contains(&"Button component".to_string()),
            "{:?}",
            r.added
        );
        let b = by_name(&r.system.components, "Button");
        assert_eq!(b.id, component_id(&first_page, button));
        assert_eq!(
            by_name(&r.system.components, "Card").id,
            component_id(ed.active_page_id(), card)
        );
        let ids: Vec<NodeId> = b.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids, [button, rect]);
        assert_eq!(
            (b.nodes[0].rect.x, b.nodes[0].rect.y, b.nodes[0].parent),
            (0.0, 0.0, None)
        );
        assert!(!has_unpublished_changes(&store, &ed));
        // Files can still use a system that publishes components.
        let mut s2 = Session::new(store.clone());
        let mut app = Editor::default();
        s2.create(&mut app, "App").unwrap();
        app.use_system(&r.system).unwrap();

        // Moving a main or switching pages isn't a change; editing a layer inside one is.
        ed.switch_page(0);
        ed.edit("move", |tx| tx.node_mut(button).unwrap().rect.x = 900.0);
        assert!(!has_unpublished_changes(&store, &ed));
        set_fill(&mut ed, rect, Color::rgb(0, 0, 255));
        assert!(has_unpublished_changes(&store, &ed));
        let old = Registry::in_store(&store).get(&r.system.id);
        let c = changes(old.as_ref(), "brand", &Contents::of(&ed));
        assert_eq!(c.changed, ["Button component"]);
        assert!(c.added.is_empty() && c.removed.is_empty(), "{c:?}");

        ed.edit("delete", |tx| tx.remove(button));
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.removed, ["Button component"]);
        assert!(
            r.message().contains("removed Button component"),
            "{}",
            r.message()
        );
    }

    #[test]
    fn bindings_to_other_systems_become_values_and_local_ones_keep_their_ids() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let base = publish_open(&s, &mut ed, &PublishRequest::default()).system;

        let mut s2 = Session::new(store.clone());
        let mut app = Editor::default();
        s2.create(&mut app, "App").unwrap();
        app.use_system(&base).unwrap();
        let local = app
            .edit_tokens("add", None, |t| {
                t.create(TokenKind::Color, "ink", "#00ff00", None)
            })
            .unwrap();
        let (main, rect) = component(&mut app, "Badge", 0.0);
        let brand = app.tokens.by_css_name("--brand-brand").unwrap().id.clone();
        app.bind_token(&[rect], TokenProp::Fill, &brand);
        app.bind_token(&[main], TokenProp::Fill, &local);
        let other_mode = app.tokens.system(&base.id).unwrap().modes[0].id.clone();
        app.edit("mode", |tx| {
            tx.node_mut(main).unwrap().mode = Some(other_mode)
        });
        let r = publish_open(&s2, &mut app, &PublishRequest::default());

        let c = by_name(&r.system.components, "Badge");
        let (m, inner) = (&c.nodes[0], &c.nodes[1]);
        assert!(inner.tokens.is_empty(), "{:?}", inner.tokens);
        assert_eq!(inner.style.fill(), Some(Color::rgb(255, 0, 0)));
        assert_eq!(m.tokens.get(&TokenProp::Fill), Some(&local));
        assert!(m.mode.is_none());
        assert!(!has_unpublished_changes(&store, &app));
    }

    #[test]
    fn nested_instances_name_the_component_they_come_from() {
        let store = temp_store();
        let (s, mut ed) = source(&store);
        let (button, _) = component(&mut ed, "Button", 0.0);
        let inst = ed.create_instance(button, v2(0.0, 200.0)).unwrap();
        ed.select(vec![inst]);
        ed.create_component();
        let card = ed.doc.get(inst).unwrap().parent.unwrap();
        ed.edit("name", |tx| tx.node_mut(card).unwrap().name = "Card".into());
        let r = publish_open(&s, &mut ed, &PublishRequest::default());

        let page = ed.active_page_id().to_string();
        let c = by_name(&r.system.components, "Card");
        assert_eq!(c.nodes.len(), 3);
        assert_eq!(c.nested.len(), 2, "{:?}", c.nested);
        assert!(c.nested.values().all(|v| *v == component_id(&page, button)));
        assert_eq!(c.nested.get(&inst), Some(&component_id(&page, button)));
        assert!(by_name(&r.system.components, "Button").nested.is_empty());
    }

    #[test]
    fn images_are_copied_with_the_entry_and_go_when_unused_or_unpublished() {
        let store = temp_store();
        let mut s = Session::new(store.clone());
        let mut ed = Editor::default();
        s.create(&mut ed, "Icons").unwrap();
        let (_, rect) = component(&mut ed, "Logo", 0.0);
        let asset = crate::assets::register_rgba(&image::RgbaImage::new(2, 2));
        let image = Paint::Image {
            asset: asset.clone(),
            mode: Default::default(),
            opacity: 1.0,
        };
        ed.edit("image", |tx| {
            tx.node_mut(rect).unwrap().style.fills = vec![image]
        });
        // A file with only components can publish.
        let r = publish_open(&s, &mut ed, &PublishRequest::default());
        assert_eq!(r.system.tokens.len(), 0);
        let reg = Registry::in_store(&store);
        let dir = reg.assets_dir(&r.system.id).unwrap();
        assert!(dir.join(format!("{asset}.png")).exists());
        assert_eq!(reg.get(&r.system.id).unwrap(), r.system);

        set_fill(&mut ed, rect, Color::WHITE);
        publish_open(&s, &mut ed, &PublishRequest::default());
        assert!(!dir.join(format!("{asset}.png")).exists());
        unpublish(&store, &ed).unwrap();
        assert!(!dir.parent().unwrap().exists());
    }
}
