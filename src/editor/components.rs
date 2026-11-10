//! Components across pages and design systems: an instance's main may be on another page
//! ([`InstanceLink::page`]) or in a design system the file uses ([`InstanceLink::component`]).
//! Same-page instances sync in the main's undo step. Instances of mains elsewhere sync when
//! shown, saved or exported, without adding to their page's undo history.

use super::Editor;
use super::structure::{Source, instantiate, sync_node};
use crate::geom::{Rect, V2};
use crate::history::{Recording, Tx};
use crate::model::{Document, InstanceLink, LinkedComponent, Node, NodeId};
use std::collections::HashSet;

impl Editor {
    /// Resolve the main's page relative to the page currently held in `doc`. None for a
    /// design system's component.
    pub fn main_page(&self, link: &InstanceLink) -> Option<usize> {
        if link.component.is_some() {
            return None;
        }
        match &link.page {
            None => Some(self.current_page()),
            Some(id) => self.pages.iter().position(|p| p.id == *id),
        }
    }

    /// The document that holds the main: a page, or a design system component's copy.
    pub fn main_doc(&self, link: &InstanceLink) -> Option<&Document> {
        match &link.component {
            Some(c) => self.tokens.component(c).map(|c| &*c.doc),
            None => self.main_page(link).map(|i| self.page_doc(i)),
        }
    }

    /// Place an instance of design system component `id` (see `TokenSet::components`) with
    /// its top-left at world position `at`, and select it.
    pub fn place_component(&mut self, id: &str, at: V2) -> Option<NodeId> {
        let c = self.tokens.component(id)?.clone();
        let main = c.doc.get(c.main()?)?;
        let rect = Rect::new(at.x, at.y, main.rect.w, main.rect.h);
        let src = Source {
            doc: Some(&c.doc),
            page: None,
            component: Some(&c.id),
        };
        let label = format!("Insert {}", c.name);
        let id = self.edit(&label, |tx| {
            instantiate(tx, src, main.id, None, None, Some(rect))
        });
        self.selection = vec![id];
        Some(id)
    }

    /// Sync these instance roots from their mains, on any page or in a design system, in
    /// the open transaction.
    pub(super) fn sync_instances(&mut self, roots: &[NodeId]) {
        enum From {
            Here,
            Page(usize),
            System(LinkedComponent),
        }
        let cur = self.current_page();
        let srcs: Vec<(NodeId, From)> = (roots.iter())
            .filter_map(|r| {
                let l = self.doc.get(*r)?.instance.as_ref()?;
                let from = match &l.component {
                    Some(c) => From::System(self.tokens.component(c)?.clone()),
                    None => match self.main_page(l)? {
                        p if p == cur => From::Here,
                        p => From::Page(p),
                    },
                };
                Some((*r, from))
            })
            .collect();
        if srcs.is_empty() {
            return;
        }
        self.revision += 1;
        self.doc_stamp = super::next_stamp();
        let (rec, _) = self
            .rec
            .get_or_insert_with(|| (Recording::new("Edit", &self.doc), None));
        let mut tx = Tx::new(&mut self.doc, rec);
        for (r, from) in &srcs {
            // Every page but the current one is parked, so its slot holds its document.
            let src = match from {
                From::Here => Source::LOCAL,
                From::Page(p) => match self.pages[*p].stash.as_ref() {
                    Some(s) => Source {
                        doc: Some(&s.doc),
                        page: Some(&self.pages[*p].id),
                        component: None,
                    },
                    None => continue,
                },
                From::System(c) => Source {
                    doc: Some(&c.doc),
                    page: None,
                    component: Some(&c.id),
                },
            };
            sync_node(&mut tx, src, *r, true);
        }
    }

    /// Sync source pages first, without adding undo steps. Mark changed pages for saving.
    /// Defer during edits or [`Editor::with_page`] to avoid splitting an edit.
    pub fn sync_page_instances(&mut self, i: usize) {
        if self.can_sync() {
            self.sync_deep(i, &mut HashSet::new());
        }
    }

    pub fn sync_all_instances(&mut self) {
        if !self.can_sync() {
            return;
        }
        let mut seen = HashSet::new();
        for i in 0..self.pages.len() {
            self.sync_deep(i, &mut seen);
        }
    }

    fn can_sync(&self) -> bool {
        self.rec.is_none() && self.lent_page.is_none()
    }

    fn sync_deep(&mut self, i: usize, seen: &mut HashSet<usize>) {
        if i >= self.pages.len() || !seen.insert(i) {
            return;
        }
        let own = &self.pages[i].id;
        let doc = self.page_doc(i);
        let mut sources: Vec<usize> = Vec::new();
        // Instances of design system components: the outermost layer linked to one.
        let comp = |id: Option<NodeId>| {
            id.and_then(|id| doc.get(id)?.instance.as_ref()?.component.as_deref())
        };
        let mut roots: Vec<NodeId> = (doc.nodes.values())
            .filter(|n| {
                let c = comp(Some(n.id));
                c.is_some_and(|c| self.tokens.component(c).is_some()) && comp(n.parent) != c
            })
            .map(|n| n.id)
            .collect();
        for n in doc.nodes.values() {
            let Some(page) = n.instance.as_ref().and_then(|l| l.page.as_ref()) else {
                continue;
            };
            if page == own {
                continue;
            }
            let Some(j) = self.pages.iter().position(|p| p.id == *page) else {
                continue;
            };
            if !sources.contains(&j) {
                sources.push(j);
            }
            // Instance roots: the outermost layer linked to that page.
            let parent = n.parent.and_then(|p| doc.get(p));
            if parent
                .is_none_or(|p| p.instance.as_ref().and_then(|l| l.page.as_ref()) != Some(page))
            {
                roots.push(n.id);
            }
        }
        if roots.is_empty() {
            return;
        }
        // A main can hold instances of mains elsewhere: sync those pages first.
        for j in sources {
            self.sync_deep(j, seen);
        }
        let changed = self.with_page(i, |ed| {
            // New layers stay off ids a redo will recreate.
            let next_id = ed.doc.next_id;
            ed.doc.next_id = next_id.max(ed.history.id_ceiling());
            ed.rec = Some((Recording::new("Sync components", &ed.doc), None));
            ed.sync_instances(&roots);
            ed.fixup();
            // Mode overrides can restore the original values, leaving no net change.
            let (rec, _) = ed.rec.take().unwrap();
            let changed =
                (rec.touched().iter()).any(|id| rec.before(*id) != Some(&ed.doc.get(*id).cloned()));
            if !changed {
                ed.doc.next_id = next_id;
            }
            changed
        });
        if changed {
            self.pages[i].saved_gen = u64::MAX;
        }
    }

    /// Mains copied from another project page become instances. Drop links outside the project.
    pub(super) fn relink_for_paste(
        &self,
        nodes: &[Node],
        roots: &[NodeId],
        from: Option<&str>,
    ) -> Vec<Node> {
        let here = self.pages[self.current_page()].id.as_str();
        let from = from.filter(|f| self.pages.iter().any(|p| p.id == *f));
        let src = from.and_then(|f| self.page_doc_by_id(f));
        let mut out = nodes.to_vec();
        if let (Some(f), Some(src)) = (from, src)
            && f != here
        {
            for r in roots {
                if !src.get(*r).is_some_and(|m| m.component) {
                    continue;
                }
                let sub: HashSet<NodeId> = src.subtree(*r).into_iter().collect();
                for n in out.iter_mut().filter(|n| sub.contains(&n.id)) {
                    n.component = false;
                    n.instance = Some(InstanceLink {
                        main: n.id,
                        page: Some(f.to_string()),
                        component: None,
                        overrides: Vec::new(),
                    });
                }
            }
        }
        let copied: HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
        for n in &mut out {
            let Some(l) = &mut n.instance else { continue };
            if let Some(c) = &l.component {
                // Linked while this file uses that system; otherwise plain layers.
                if self.tokens.component(c).is_none() {
                    n.instance = None;
                }
                continue;
            }
            if l.page.is_none() && !copied.contains(&l.main) {
                // A local main left behind: it is on the source page.
                l.page = from.map(String::from);
            }
            // A link to this page stays explicit until the copies are in, so it can't be
            // mistaken for a copied layer with the same id (see `Editor::insert_subtrees`).
            match l.page.as_deref() {
                Some(p) if self.pages.iter().any(|s| s.id == p) => {}
                Some(_) => n.instance = None,
                None if copied.contains(&l.main) => {}
                None => n.instance = None,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Color, NodeKind};

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);
    const GREEN: Color = Color::rgb(0, 255, 0);

    fn two_pages() -> (Editor, NodeId, NodeId) {
        let mut e = Editor::default();
        let f = Node::new(
            NodeId(0),
            NodeKind::Frame { clip: true },
            Rect::new(0.0, 0.0, 100.0, 50.0),
        );
        let main = e.add_node(f, None);
        let mut r = Node::new(NodeId(0), NodeKind::Rect, Rect::new(10.0, 10.0, 20.0, 20.0));
        r.style.set_fill(Some(RED));
        let rect = e.add_node(r, Some(main));
        e.selection = vec![main];
        e.create_component();
        e.add_page_in_background(Some("Second"));
        (e, main, rect)
    }

    fn copy(e: &mut Editor, ids: &[NodeId]) {
        e.selection = ids.to_vec();
        e.copy_internal();
    }

    fn paste_on(e: &mut Editor, page: usize) -> NodeId {
        e.switch_page(page);
        e.paste(crate::geom::V2::ZERO);
        e.selection[0]
    }

    fn set_main_fill(e: &mut Editor, rect: NodeId, c: Color) {
        e.edit("fill", |tx| {
            tx.node_mut(rect).unwrap().style.set_fill(Some(c))
        });
    }

    fn inner_fill(e: &Editor, page: usize, inst: NodeId) -> Option<Color> {
        let d = e.page_doc(page);
        let child = d.get(inst).unwrap().children[0];
        d.get(child).unwrap().style.fill()
    }

    #[test]
    fn instances_follow_their_main_when_their_page_is_shown_with_no_undo_step() {
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        let n = e.doc.get(inst).unwrap();
        assert!(!n.component);
        let l = n.instance.as_ref().unwrap();
        assert_eq!(
            (l.main, l.page.as_deref()),
            (main, Some(e.pages[0].id.as_str()))
        );
        let undo_len = e.history.undo_len();
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        assert_eq!(inner_fill(&e, 1, inst), Some(RED), "lazy: not until shown");
        e.switch_page(1);
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
        assert_eq!(e.history.undo_len(), undo_len, "the sync adds no undo step");
        assert!(e.dirty_pages().contains(&1), "the synced page needs saving");
        e.switch_page(0);
        e.undo();
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
        e.switch_page(1);
        assert_eq!(inner_fill(&e, 1, inst), Some(RED));
    }

    #[test]
    fn saving_or_exporting_syncs_pages_that_are_not_shown() {
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        e.sync_all_instances();
        assert_eq!(e.active_page, 0);
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
    }

    #[test]
    fn overrides_on_another_page_survive_a_sync() {
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        let child = e.doc.get(inst).unwrap().children[0];
        e.edit("fill", |tx| {
            tx.node_mut(child).unwrap().style.set_fill(Some(GREEN))
        });
        e.mark_overrides(&[child], "fill");
        e.commit();
        e.switch_page(0);
        e.edit("move", |tx| tx.node_mut(rect).unwrap().rect.x = 30.0);
        set_main_fill(&mut e, rect, BLUE);
        e.switch_page(1);
        let c = e.doc.get(child).unwrap();
        assert_eq!((c.style.fill(), c.rect.x), (Some(GREEN), 30.0));
        e.selection = vec![inst];
        e.reset_overrides();
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
    }

    #[test]
    fn undo_on_an_instance_page_keeps_the_mains_current_size() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        e.switch_page(0);
        e.edit("resize", |tx| tx.node_mut(main).unwrap().rect.w = 300.0);
        e.switch_page(1);
        e.edit("move", |tx| tx.node_mut(inst).unwrap().rect.x = 500.0);
        e.undo();
        assert_eq!(e.doc.get(inst).unwrap().rect.w, 300.0);
    }

    #[test]
    fn instances_pasted_back_onto_the_main_page_link_locally() {
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        copy(&mut e, &[inst]);
        let back = paste_on(&mut e, 0);
        let l = e.doc.get(back).unwrap().instance.clone().unwrap();
        assert_eq!((l.main, l.page), (main, None));
        set_main_fill(&mut e, rect, BLUE);
        assert_eq!(inner_fill(&e, 0, back), Some(BLUE));
    }

    #[test]
    fn instances_copied_between_other_pages_keep_their_main() {
        let (mut e, main, rect) = two_pages();
        e.add_page_in_background(Some("Third"));
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        copy(&mut e, &[inst]);
        let third = paste_on(&mut e, 2);
        let l = e.doc.get(third).unwrap().instance.clone().unwrap();
        assert_eq!(l.page.as_deref(), Some(e.pages[0].id.as_str()));
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        e.selection = vec![];
        e.switch_page(2);
        assert_eq!(inner_fill(&e, 2, third), Some(BLUE));
        e.selection = vec![third];
        e.go_to_main();
        assert_eq!((e.active_page, e.selection.clone()), (0, vec![main]));
    }

    #[test]
    fn links_to_pages_outside_the_project_are_dropped_on_paste() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        copy(&mut e, &[inst]);
        e.clipboard_page = Some("another-project".into());
        e.clipboard.iter_mut().for_each(|n| {
            if let Some(l) = &mut n.instance {
                l.page = Some("gone".into());
            }
        });
        let pasted = paste_on(&mut e, 1);
        assert!(e.doc.get(pasted).unwrap().instance.is_none());
    }

    #[test]
    fn a_main_on_another_page_can_hold_instances_from_a_third() {
        // Page 0: component A. Page 1: component B holding an instance of A.
        // Page 2: an instance of B. Editing A reaches page 2 through page 1.
        let (mut e, a, rect) = two_pages();
        e.add_page_in_background(Some("Third"));
        copy(&mut e, &[a]);
        let a_in_b = paste_on(&mut e, 1);
        e.selection = vec![a_in_b];
        e.create_component();
        let b = e.selection[0];
        copy(&mut e, &[b]);
        let b_inst = paste_on(&mut e, 2);
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        e.switch_page(2);
        let d = e.page_doc(2);
        let leaf = *d.subtree(b_inst).last().unwrap();
        assert_eq!(d.get(leaf).unwrap().style.fill(), Some(BLUE));
    }

    #[test]
    fn undo_on_a_background_page_waits_to_sync() {
        // Agents undo on pages the user isn't viewing (`Editor::with_page`).
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        e.edit("move", |tx| tx.node_mut(inst).unwrap().rect.x = 500.0);
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        e.with_page(1, |ed| ed.undo());
        assert_eq!(inner_fill(&e, 1, inst), Some(RED));
        e.sync_all_instances();
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
    }

    #[test]
    fn undo_after_a_structural_sync_keeps_the_page_whole() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        e.add_node(
            Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 9.0, 9.0)),
            None,
        );
        e.switch_page(0);
        let extra = Node::new(NodeId(0), NodeKind::Rect, Rect::new(50.0, 10.0, 5.0, 5.0));
        let extra = e.add_node(extra, Some(main));
        e.switch_page(1);
        assert_eq!(e.doc.get(inst).unwrap().children.len(), 2);
        // An unrelated undo rewinds `next_id`; new layers must not take the synced child's id.
        e.undo();
        e.doc.validate().unwrap();
        let n = Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 9.0, 9.0));
        e.add_node(n, None);
        e.doc.validate().unwrap();
        assert_eq!(e.doc.get(inst).unwrap().children.len(), 2);
        e.undo();
        e.undo();
        e.doc.validate().unwrap();
        assert!(e.doc.nodes.is_empty(), "{:?}", e.doc.nodes.keys());
        e.redo();
        e.switch_page(0);
        e.edit("delete", |tx| tx.remove(extra));
        e.switch_page(1);
        e.edit("move", |tx| tx.node_mut(inst).unwrap().rect.x = 300.0);
        e.switch_page(0);
        e.undo();
        e.switch_page(1);
        e.undo();
        e.doc.validate().unwrap();
        assert_eq!(e.doc.get(inst).unwrap().children.len(), 2);
    }

    #[test]
    fn a_sync_after_undo_leaves_ids_free_for_redo() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        let r = Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 9.0, 9.0));
        let r = e.add_node(r, None);
        e.switch_page(0);
        let extra = Node::new(NodeId(0), NodeKind::Rect, Rect::new(50.0, 10.0, 5.0, 5.0));
        e.add_node(extra, Some(main));
        e.switch_page(1);
        e.undo();
        e.undo();
        e.redo();
        e.redo();
        e.doc.validate().unwrap();
        assert!(e.doc.get(r).is_some_and(|n| n.parent.is_none()));
        assert_eq!(e.doc.get(inst).unwrap().children.len(), 2);
    }

    #[test]
    fn old_links_without_a_page_still_load() {
        let l: InstanceLink = serde_json::from_str(r#"{"main": 4}"#).unwrap();
        assert_eq!(l.page, None);
    }
}
