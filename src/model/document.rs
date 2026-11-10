//! The document arena: nodes keyed by id plus ordered child lists.

use super::node::{Color, Node, NodeId, NodeKind};
use crate::geom::{Rect, V2};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub nodes: HashMap<NodeId, Node>,
    /// Top-level nodes in paint order, bottom to top.
    pub root: Vec<NodeId>,
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            name: "Untitled".into(),
            nodes: HashMap::new(),
            root: Vec::new(),
            next_id: 1,
        }
    }
}

impl Document {
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn children(&self, parent: Option<NodeId>) -> &[NodeId] {
        match parent {
            None => &self.root,
            Some(p) => self
                .nodes
                .get(&p)
                .map(|n| n.children.as_slice())
                .unwrap_or(&[]),
        }
    }

    pub fn children_mut(&mut self, parent: Option<NodeId>) -> Option<&mut Vec<NodeId>> {
        match parent {
            None => Some(&mut self.root),
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
        }
    }

    /// World-space position of a node's local coordinate origin's parent space.
    /// The top solid fill at `id` or its nearest ancestor with one. Text and pencil strokes
    /// use their fill as ink, so they don't count as surfaces.
    pub fn surface_color(&self, id: Option<NodeId>) -> Option<Color> {
        let mut at = id;
        while let Some(n) = at.and_then(|id| self.get(id)) {
            let ink = matches!(n.kind, NodeKind::Text { .. } | NodeKind::Stroke { .. });
            if let Some(fill) = n.style.fill().filter(|c| c.a > 0 && !ink) {
                return Some(fill);
            }
            at = n.parent;
        }
        None
    }

    pub fn parent_origin(&self, parent: Option<NodeId>) -> V2 {
        let mut o = V2::ZERO;
        let mut cur = parent;
        while let Some(id) = cur {
            let Some(n) = self.nodes.get(&id) else { break };
            o = o + n.rect.min();
            cur = n.parent;
        }
        o
    }

    pub fn world_rect(&self, id: NodeId) -> Option<Rect> {
        let n = self.nodes.get(&id)?;
        Some(n.rect.translate(self.parent_origin(n.parent)))
    }

    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(p) = cur {
            out.push(p);
            cur = self.nodes.get(&p).and_then(|n| n.parent);
        }
        out
    }

    pub fn is_ancestor(&self, ancestor: NodeId, of: NodeId) -> bool {
        self.ancestors(of).contains(&ancestor)
    }

    /// The node and all its descendants, parent first.
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            if let Some(n) = self.nodes.get(&out[i]) {
                out.extend(n.children.iter().copied());
            }
            i += 1;
        }
        out
    }

    /// All nodes in paint order (depth-first, parents before children).
    pub fn paint_order(&self) -> Vec<NodeId> {
        fn walk(doc: &Document, ids: &[NodeId], out: &mut Vec<NodeId>) {
            for id in ids {
                out.push(*id);
                if let Some(n) = doc.nodes.get(id) {
                    walk(doc, &n.children, out);
                }
            }
        }
        let mut out = Vec::with_capacity(self.nodes.len());
        walk(self, &self.root, &mut out);
        out
    }

    /// Whether a node and all its ancestors are visible.
    pub fn is_effectively_visible(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.nodes.get(&c) {
                Some(n) if n.visible => cur = n.parent,
                _ => return false,
            }
        }
        true
    }

    pub fn is_effectively_locked(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.nodes.get(&c) {
                Some(n) if n.locked => return true,
                Some(n) => cur = n.parent,
                None => return false,
            }
        }
        false
    }

    /// Bounds of all visible content in world space.
    pub fn content_bounds(&self) -> Option<Rect> {
        let rects: Vec<Rect> = self
            .root
            .iter()
            .filter(|id| self.nodes.get(id).is_some_and(|n| n.visible))
            .filter_map(|id| self.world_rect(*id))
            .collect();
        Rect::union_all(&rects)
    }

    /// Mend the tree after undo or redo replayed snapshots over changes made outside the
    /// history (instances synced from another page): drop references to missing nodes and
    /// nodes no longer reachable, and keep `next_id` above every id in use.
    pub fn prune(&mut self) {
        let nodes = &self.nodes;
        self.root.retain(|id| nodes.contains_key(id));
        let missing: Vec<NodeId> = (self.nodes.values())
            .filter(|n| n.children.iter().any(|c| !self.nodes.contains_key(c)))
            .map(|n| n.id)
            .collect();
        for id in missing {
            let n = self.nodes.get(&id).unwrap();
            let kept = (n.children.iter().copied())
                .filter(|c| self.nodes.contains_key(c))
                .collect();
            self.nodes.get_mut(&id).unwrap().children = kept;
        }
        let mut reached = std::collections::HashSet::new();
        let mut stack = self.root.clone();
        while let Some(id) = stack.pop() {
            if reached.insert(id) {
                stack.extend(self.nodes[&id].children.iter().copied());
            }
        }
        self.nodes.retain(|id, _| reached.contains(id));
        let top = self.nodes.keys().map(|id| id.0 + 1).max().unwrap_or(1);
        self.next_id = self.next_id.max(top);
    }

    /// Structural invariants, used by tests and after loading files.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        let mut check = |ids: &[NodeId], parent: Option<NodeId>| -> Result<(), String> {
            for id in ids {
                let n = self.nodes.get(id).ok_or(format!("missing node {id}"))?;
                if n.parent != parent {
                    return Err(format!(
                        "node {id} has parent {:?}, expected {:?}",
                        n.parent, parent
                    ));
                }
                if !seen.insert(*id) {
                    return Err(format!("node {id} appears twice"));
                }
            }
            Ok(())
        };
        check(&self.root, None)?;
        for n in self.nodes.values() {
            check(&n.children, Some(n.id))?;
            if n.id.0 >= self.next_id {
                return Err(format!("node {} >= next_id {}", n.id, self.next_id));
            }
        }
        if seen.len() != self.nodes.len() {
            return Err(format!(
                "{} nodes unreachable",
                self.nodes.len() - seen.len()
            ));
        }
        Ok(())
    }
}
