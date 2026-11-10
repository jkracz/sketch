//! Hidden, read-only copies of published components, stored in the file manifest.
//! Instances use stable component and source layer ids to keep overrides across updates.

use super::tokens::{Keyed, TokenSet, replace_linked};
use super::{Document, Node, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "Stored", into = "Stored")]
pub struct LinkedComponent {
    /// `<system>/<component>` (`linked_id`).
    pub id: String,
    pub system: String,
    pub name: String,
    /// The main, as the only root, and its layers. Bindings use this file's linked ids.
    /// Shared, because every token edit snapshots the token set for undo.
    pub doc: Arc<Document>,
}

impl LinkedComponent {
    /// `nodes` holds the main, with no parent, and its layers.
    pub fn new(id: String, system: &str, name: String, nodes: Vec<Node>) -> LinkedComponent {
        let next_id = nodes.iter().map(|n| n.id.0).max().unwrap_or(0) + 1;
        let root = (nodes.iter())
            .filter(|n| n.parent.is_none())
            .map(|n| n.id)
            .take(1)
            .collect();
        let doc = Document {
            name: name.clone(),
            nodes: nodes
                .into_iter()
                .map(|n| (n.id, n))
                .collect::<HashMap<_, _>>(),
            root,
            next_id,
        };
        LinkedComponent {
            id,
            system: system.to_string(),
            name,
            doc: Arc::new(doc),
        }
    }

    pub fn main(&self) -> Option<NodeId> {
        self.doc.root.first().copied()
    }

    pub fn size(&self) -> (f64, f64) {
        (self.main().and_then(|m| self.doc.get(m))).map_or((0.0, 0.0), |m| (m.rect.w, m.rect.h))
    }
}

impl PartialEq for LinkedComponent {
    fn eq(&self, o: &Self) -> bool {
        (self.id == o.id && self.system == o.system && self.name == o.name)
            && (Arc::ptr_eq(&self.doc, &o.doc) || self.doc == o.doc)
    }
}

impl Keyed for LinkedComponent {
    fn key(&self) -> &str {
        &self.id
    }
}

/// Store parents before children for repeatable saves.
#[derive(Serialize, Deserialize)]
struct Stored {
    id: String,
    system: String,
    name: String,
    nodes: Vec<Node>,
}

impl From<Stored> for LinkedComponent {
    fn from(s: Stored) -> LinkedComponent {
        LinkedComponent::new(s.id, &s.system, s.name, s.nodes)
    }
}

impl From<LinkedComponent> for Stored {
    fn from(c: LinkedComponent) -> Stored {
        let nodes = (c
            .main()
            .map(|m| c.doc.subtree(m))
            .unwrap_or_default()
            .iter())
        .filter_map(|id| c.doc.get(*id).cloned())
        .collect();
        Stored {
            id: c.id,
            system: c.system,
            name: c.name,
            nodes,
        }
    }
}

impl TokenSet {
    /// Grouped by system, then sorted by name.
    pub fn components(&self) -> &[LinkedComponent] {
        &self.components
    }

    pub fn component(&self, id: &str) -> Option<&LinkedComponent> {
        self.components.iter().find(|c| c.id == id)
    }

    /// A component by its id, its name, or `<system name>/<component name>`, ignoring case.
    /// The first match wins when two systems have a component of that name.
    pub fn find_component(&self, name: &str) -> Option<&LinkedComponent> {
        let name = name.trim();
        let is = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
        let system_name = |c: &LinkedComponent| self.system(&c.system).map(|s| s.name.as_str());
        (self.component(name))
            .or_else(|| self.components.iter().find(|c| is(&c.name, name)))
            .or_else(|| {
                let (sys, comp) = name.rsplit_once('/')?;
                (self.components.iter())
                    .find(|c| is(&c.name, comp) && system_name(c).is_some_and(|s| is(s, sys)))
            })
    }

    pub fn with_components(mut self, components: Vec<LinkedComponent>) -> TokenSet {
        self.components = components;
        self
    }

    /// Keep the system's position in the list and return removed components.
    pub fn replace_components(
        &mut self,
        system: &str,
        new: Vec<LinkedComponent>,
    ) -> Vec<LinkedComponent> {
        replace_linked(&mut self.components, |c| c.system == system, new)
    }
}
