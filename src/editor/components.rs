//! Components across pages. An instance may point at a main component on another page of
//! the project ([`InstanceLink::page`]). Editing a main syncs instances on its own page in the
//! same undo step (see [`Editor::fixup`]). Instances on other pages follow lazily, when their
//! page is shown, saved or exported, the way token values do: the sync is derived state and
//! adds nothing to that page's undo history.

use super::Editor;
use super::structure::{Source, sync_node};
use crate::history::{Recording, Tx};
use crate::model::{Document, InstanceLink, Node, NodeId};
use std::collections::HashSet;

impl Editor {
    /// The page holding the main that `link`, on the current document, points at.
    pub fn main_page(&self, link: &InstanceLink) -> Option<usize> {
        match &link.page {
            None => Some(self.current_page()),
            Some(id) => self.pages.iter().position(|p| p.id == *id),
        }
    }

    /// The document holding the main that `link`, on the current document, points at.
    pub fn main_doc(&self, link: &InstanceLink) -> Option<&Document> {
        self.main_page(link).map(|i| self.page_doc(i))
    }

    /// Sync these instance roots on the current document from their mains, wherever they
    /// are, inside the open transaction.
    pub(super) fn sync_instances(&mut self, roots: &[NodeId]) {
        let cur = self.current_page();
        let srcs: Vec<(NodeId, Option<usize>)> = (roots.iter())
            .filter_map(|r| {
                let l = self.doc.get(*r)?.instance.as_ref()?;
                let page = self.main_page(l)?;
                Some((*r, (page != cur).then_some(page)))
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
        for (r, page) in srcs {
            // Every page but the current one is parked, so its slot holds its document.
            let src = match page {
                None => Source::LOCAL,
                Some(p) => Source {
                    doc: self.pages[p].stash.as_ref().map(|s| &s.doc),
                    page: Some(&self.pages[p].id),
                },
            };
            if page.is_none() || src.doc.is_some() {
                sync_node(&mut tx, src, r, true);
            }
        }
    }

    /// Bring page `i`'s instances of mains on other pages up to date, after the pages it
    /// takes mains from. Adds no undo step; the page is marked for saving if anything moved.
    /// Waits while an edit is open (a drag or text edit), so it never splits one.
    pub fn sync_page_instances(&mut self, i: usize) {
        if self.rec.is_none() {
            self.sync_deep(i, &mut HashSet::new());
        }
    }

    /// [`Editor::sync_page_instances`] for every page (before saving or exporting).
    pub fn sync_all_instances(&mut self) {
        if self.rec.is_some() {
            return;
        }
        let mut seen = HashSet::new();
        for i in 0..self.pages.len() {
            self.sync_deep(i, &mut seen);
        }
    }

    fn sync_deep(&mut self, i: usize, seen: &mut HashSet<usize>) {
        if i >= self.pages.len() || !seen.insert(i) {
            return;
        }
        let own = &self.pages[i].id;
        let mut sources: Vec<usize> = Vec::new();
        let mut roots: Vec<NodeId> = Vec::new();
        for n in self.page_doc(i).nodes.values() {
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
            let parent = n.parent.and_then(|p| self.page_doc(i).get(p));
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
            ed.rec = Some((Recording::new("Sync components", &ed.doc), None));
            ed.sync_instances(&roots);
            let changed = ed
                .rec
                .as_ref()
                .is_some_and(|(r, _)| !r.touched().is_empty());
            if changed {
                ed.fixup();
            }
            ed.rec = None;
            changed
        });
        if changed {
            self.pages[i].saved_gen = u64::MAX;
        }
    }

    /// Prepare copied layers for pasting onto the current page. `from` is the page they were
    /// copied from, when it is a page of this project; links to mains elsewhere are dropped,
    /// as before. A main component copied from another page pastes as an instance of it.
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
        // Live mains on another page become instances of them.
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
                        overrides: Vec::new(),
                    });
                }
            }
        }
        let copied: HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
        for n in &mut out {
            let Some(l) = &mut n.instance else { continue };
            if l.page.is_none() && !copied.contains(&l.main) {
                // A local main left behind: it is on the source page.
                l.page = from.map(String::from);
            }
            // A link to this page stays explicit until the copies are in, so it can't be
            // mistaken for a copied layer with the same id (see `Editor::insert_subtrees`).
            match l.page.as_deref() {
                Some(p) if self.pages.iter().any(|s| s.id == p) => {}
                Some(_) => n.instance = None,
                // Main copied along: the paste links to the copy.
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

    /// A project whose first page holds a component (a frame with a red rect), and an empty
    /// second page. Returns the editor, the main and its rect.
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
    fn pasting_a_main_on_another_page_makes_an_instance_of_it() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        let n = e.doc.get(inst).unwrap();
        assert!(!n.component);
        let l = n.instance.as_ref().unwrap();
        assert_eq!(
            (l.main, l.page.as_deref()),
            (main, Some(e.pages[0].id.as_str()))
        );
        assert!(e.is_instance_root(inst));
        assert_eq!(e.instance_main(inst), Some((0, main)));
        assert_eq!(inner_fill(&e, 1, inst), Some(RED));
        e.doc.validate().unwrap();
    }

    #[test]
    fn instances_follow_their_main_when_their_page_is_shown_with_no_undo_step() {
        let (mut e, main, rect) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        let undo_len = e.history.undo_len();
        e.switch_page(0);
        set_main_fill(&mut e, rect, BLUE);
        assert_eq!(inner_fill(&e, 1, inst), Some(RED), "lazy: not until shown");
        e.switch_page(1);
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
        assert_eq!(e.history.undo_len(), undo_len, "the sync adds no undo step");
        assert!(e.dirty_pages().contains(&1), "the synced page needs saving");
        // Undo on the main's page undoes the edit there only; the instance follows later.
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
        assert_eq!(e.page_data(1).doc.get(inst).unwrap().children.len(), 1);
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
        // Reset reads the main from its page.
        e.selection = vec![inst];
        e.reset_overrides();
        assert_eq!(inner_fill(&e, 1, inst), Some(BLUE));
    }

    #[test]
    fn structural_edits_to_the_main_reach_other_pages() {
        let (mut e, main, _) = two_pages();
        copy(&mut e, &[main]);
        let inst = paste_on(&mut e, 1);
        e.switch_page(0);
        let extra = Node::new(NodeId(0), NodeKind::Rect, Rect::new(50.0, 10.0, 5.0, 5.0));
        e.add_node(extra, Some(main));
        e.switch_page(1);
        assert_eq!(e.doc.get(inst).unwrap().children.len(), 2);
        e.doc.validate().unwrap();
        // Undo after the sync shows the main as it is, not the old snapshot.
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
        // Local instances sync in the edit's own undo step, as before.
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
        // Go to main switches to the main's page.
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
        assert_ne!(b, a_in_b, "an instance is wrapped, not converted");
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
    fn old_links_without_a_page_still_load() {
        let l: InstanceLink = serde_json::from_str(r#"{"main": 4}"#).unwrap();
        assert_eq!(l.page, None);
        let saved = serde_json::to_string(&l).unwrap();
        assert!(!saved.contains("page"), "{saved}");
    }
}
