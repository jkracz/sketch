//! Design tokens in the editor: undoable token edits, binding node properties to tokens, and
//! keeping bound properties and text styles on every page in step with their tokens, each
//! layer in the mode its frame picks.

use super::Editor;
use crate::history::Recording;
use crate::model::{Node, NodeId, TokenClip, TokenProp, TokenSet};

impl Editor {
    /// Edit tokens or text styles as one undo step; all pages follow. `key` merges repeated edits.
    pub fn edit_tokens<R>(
        &mut self,
        label: &str,
        key: Option<&str>,
        f: impl FnOnce(&mut TokenSet) -> R,
    ) -> R {
        self.begin_coalescing(label, key);
        let before = self.tokens.clone();
        let r = f(&mut self.tokens);
        let edits = self.tokens.diff(&before);
        if !edits.is_empty() {
            self.meta_dirty = true;
            self.revision += 1;
            if let Some((rec, _)) = &mut self.rec {
                rec.set_tokens(edits);
            }
        }
        self.commit();
        self.sync_tokens();
        r
    }

    /// Bind `prop` of these layers to a token and take on its value. Layers without the
    /// property are skipped.
    pub fn bind_token(&mut self, ids: &[NodeId], prop: TokenProp, token: &str) {
        let targets: Vec<(NodeId, crate::model::TokenValue)> = ids
            .iter()
            .copied()
            .filter(|id| self.doc.get(*id).is_some_and(|n| prop.applies_to(n)))
            .filter_map(|id| {
                let mode = self.tokens.mode_at(&self.doc, id);
                Some((id, self.tokens.resolve_in(token, mode)?))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let field = prop.field();
        self.begin(&format!("Apply token to {field}"));
        {
            let mut tx = self.tx();
            for (id, value) in &targets {
                if let Some(n) = tx.node_mut(*id) {
                    prop.write(n, value);
                    n.tokens.insert(prop, token.to_string());
                }
            }
        }
        let targets: Vec<NodeId> = targets.into_iter().map(|(id, _)| id).collect();
        self.mark_overrides(&targets, field);
        for id in &targets {
            self.fit_text(*id);
        }
        self.commit();
    }

    /// Unbind `prop`, keeping the value the layers show now.
    pub fn detach_token(&mut self, ids: &[NodeId], prop: TokenProp) {
        let field = prop.field();
        self.begin(&format!("Detach token from {field}"));
        {
            let mut tx = self.tx();
            for id in ids {
                if let Some(n) = tx.node_mut(*id) {
                    n.tokens.remove(&prop);
                }
            }
        }
        self.mark_overrides(ids, field);
        self.commit();
    }

    /// The token `prop` is bound to on `n`, if it still exists.
    pub fn bound_token(&self, n: &Node, prop: TokenProp) -> Option<&crate::model::Token> {
        self.tokens.get(n.tokens.get(&prop)?)
    }

    /// End-of-transaction check: a bound property that no longer shows its token was edited
    /// by hand, so it is now a plain value. Runs for every edit path. Bindings to tokens that
    /// don't exist (deleted, or pasted from another project) go too.
    ///
    /// A layer whose mode changed keeps the old mode's value until [`Editor::resync_modes`]
    /// writes the new one, so that value counts too (see [`Editor::kept_modes`]).
    pub(super) fn drop_broken_bindings(&mut self) {
        let Some((rec, _)) = &self.rec else { return };
        let mut broken: Vec<(NodeId, TokenProp)> = Vec::new();
        for id in rec.touched() {
            let Some(n) = self.doc.get(*id) else { continue };
            for (prop, token) in &n.tokens {
                let modes = self.kept_modes(*id, prop.field());
                let kept = self
                    .tokens
                    .get(token)
                    .filter(|t| t.kind == prop.kind())
                    .is_some_and(|t| match &modes {
                        Some(modes) => (modes.iter())
                            .filter_map(|m| self.tokens.resolve_in(&t.id, *m))
                            .any(|v| prop.holds(n, &v)),
                        None => (self.tokens.resolve_any(&t.id).iter()).any(|v| prop.holds(n, v)),
                    });
                if !kept {
                    broken.push((*id, *prop));
                }
            }
        }
        if broken.is_empty() {
            return;
        }
        let mut tx = self.tx();
        for (id, prop) in broken {
            if let Some(n) = tx.node_mut(id) {
                n.tokens.remove(&prop);
            }
        }
    }

    /// The modes whose token values a touched layer may hold in `field` without counting as
    /// edited: the one it shows and the one it showed before the open transaction, plus its
    /// main component's when the instance doesn't override the field, since sync copies the
    /// main's value. None allows any mode, for new layers, which copy values from anywhere.
    pub(super) fn kept_modes(&self, id: NodeId, field: &str) -> Option<Vec<Option<&str>>> {
        let (rec, _) = self.rec.as_ref()?;
        let main = (self.doc.get(id)?.instance.as_ref())
            .filter(|l| {
                !l.overrides
                    .iter()
                    .any(|o| o == super::structure::override_key(field))
            })
            .map(|l| self.tokens.mode_at(&self.doc, l.main));
        let mut before = None;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = match rec.before(c) {
                Some(b) => b.as_ref()?,
                None => self.doc.get(c)?,
            };
            if let Some(name) = n.mode.as_deref().and_then(|m| self.tokens.mode_name(m)) {
                before = Some(name);
                break;
            }
            cur = n.parent;
        }
        let mut modes = vec![self.tokens.mode_at(&self.doc, id), before];
        modes.extend(main);
        Some(modes)
    }

    /// Sync bound fields on every page. These derived values need no undo step; undo resyncs them.
    pub fn sync_tokens(&mut self) {
        if self.tokens.is_blank() {
            return;
        }
        for i in 0..self.pages.len() {
            let doc = self.page_doc(i);
            let stale: Vec<NodeId> = (doc.nodes.values())
                .filter(|n| self.is_stale(doc, n))
                .map(|n| n.id)
                .collect();
            if stale.is_empty() {
                continue;
            }
            self.with_page(i, |ed| ed.write_token_values(&stale));
            self.pages[i].saved_gen = u64::MAX;
        }
    }

    fn is_stale(&self, doc: &crate::model::Document, n: &Node) -> bool {
        if n.tokens.is_empty() && n.text_style_ref.is_none() {
            return false;
        }
        let mode = self.tokens.mode_at(doc, n.id);
        n.tokens.iter().any(|(prop, token)| {
            (self.tokens.resolve_in(token, mode)).is_some_and(|v| !prop.holds(n, &v))
        }) || self.style_is_stale(n, mode)
    }

    fn write_token_values(&mut self, ids: &[NodeId]) {
        self.commit();
        self.rec = Some((Recording::new("Sync tokens", &self.doc), None));
        self.write_values(ids);
        self.fixup();
        self.rec = None;
    }

    /// Requires an open transaction.
    fn write_values(&mut self, ids: &[NodeId]) {
        let tokens = self.tokens.clone();
        let modes: Vec<Option<String>> = (ids.iter())
            .map(|id| tokens.mode_at(&self.doc, *id).map(String::from))
            .collect();
        {
            let mut tx = self.tx();
            for (id, mode) in ids.iter().zip(&modes) {
                let mode = mode.as_deref();
                let Some(n) = tx.node_mut(*id) else { continue };
                for (prop, token) in n.tokens.clone() {
                    if let Some(v) = tokens.resolve_in(&token, mode) {
                        prop.write(n, &v);
                    }
                }
                Self::write_style_values(&tokens, n, mode);
            }
        }
        for id in ids {
            self.fit_text(*id);
        }
    }

    /// Sync touched layers and their descendants within the open transaction.
    /// Returns the layers whose values changed.
    pub(super) fn resync_modes(&mut self) -> Vec<NodeId> {
        let Some((rec, _)) = &self.rec else {
            return Vec::new();
        };
        if self.tokens.is_blank() {
            return Vec::new();
        }
        let mut seen = std::collections::HashSet::new();
        let stale: Vec<NodeId> = (rec.touched().iter())
            .filter(|id| self.doc.nodes.contains_key(id))
            .flat_map(|id| self.doc.subtree(*id))
            .filter(|id| seen.insert(*id))
            .filter(|id| {
                self.doc
                    .get(*id)
                    .is_some_and(|n| self.is_stale(&self.doc, n))
            })
            .collect();
        if !stale.is_empty() {
            self.write_values(&stale);
        }
        stale
    }

    /// Set frames' modes by `TokenSet::choices` id. None inherits the parent's mode.
    /// Skip other layers.
    pub fn set_frame_mode(&mut self, ids: &[NodeId], mode: Option<&str>) {
        let frames: Vec<NodeId> = (ids.iter().copied())
            .filter(|id| {
                self.doc
                    .get(*id)
                    .is_some_and(|n| matches!(n.kind, crate::model::NodeKind::Frame { .. }))
            })
            .collect();
        if frames.is_empty() {
            return;
        }
        self.begin("Set mode");
        {
            let mut tx = self.tx();
            for id in &frames {
                if let Some(n) = tx.node_mut(*id) {
                    n.mode = mode.map(String::from);
                }
            }
        }
        self.mark_overrides(&frames, "mode");
        self.commit();
    }

    /// Copy tokens to the in-app clipboard, which survives switching projects.
    pub fn copy_tokens(&mut self, ids: &[String]) {
        self.token_clipboard = self.tokens.copy(ids);
        self.status = match ids.len() {
            1 => "Copied 1 token".into(),
            n => format!("Copied {n} tokens"),
        };
    }

    pub fn can_paste_tokens(&self) -> bool {
        !self.token_clipboard.tokens.is_empty()
    }

    /// Paste copied tokens as independent copies. Returns the new ids.
    pub fn paste_tokens(&mut self) -> Vec<String> {
        let clip: TokenClip = self.token_clipboard.clone();
        self.edit_tokens("Paste tokens", None, |t| t.paste(&clip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Color, NodeKind, Paint, TokenKind};

    fn editor_with_rect() -> (Editor, NodeId, String) {
        let mut e = Editor::default();
        let id = e.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(
                Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0)),
                None,
                None,
            )
        });
        let t = e.edit_tokens("add", None, |t| {
            t.create(TokenKind::Color, "brand", "#ff0000", None)
        });
        (e, id, t.unwrap())
    }

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);

    fn fill(e: &Editor, id: NodeId) -> Option<Color> {
        e.doc.get(id).unwrap().style.fill()
    }

    fn set_brand(e: &mut Editor, token: &str, v: &str) {
        e.edit_tokens("edit", None, |t| t.update(token, None, Some(v), None))
            .unwrap();
    }

    #[test]
    fn binding_applies_the_value_and_token_edits_follow() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        assert_eq!(fill(&e, id), Some(RED));
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, id), Some(BLUE));
    }

    #[test]
    fn aliases_carry_changes_through() {
        let (mut e, id, t) = editor_with_rect();
        let alias = e
            .edit_tokens("add", None, |s| {
                s.create(TokenKind::Color, "cta", "var(--brand)", None)
            })
            .unwrap();
        e.bind_token(&[id], TokenProp::Fill, &alias);
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, id), Some(BLUE));
    }

    #[test]
    fn editing_a_bound_property_by_hand_detaches_it() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        e.edit("paint", |tx| {
            tx.node_mut(id)
                .unwrap()
                .style
                .set_fill(Some(Color::rgb(1, 2, 3)));
        });
        assert!(e.doc.get(id).unwrap().tokens.is_empty());
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, id), Some(Color::rgb(1, 2, 3)));
    }

    #[test]
    fn unrelated_edits_keep_the_binding() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        e.edit("move", |tx| tx.node_mut(id).unwrap().rect.x = 50.0);
        assert!(e.doc.get(id).unwrap().tokens.contains_key(&TokenProp::Fill));
    }

    #[test]
    fn detaching_keeps_the_value() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        e.detach_token(&[id], TokenProp::Fill);
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, id), Some(RED));
    }

    #[test]
    fn deleting_a_token_keeps_values_and_undo_rebinds() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        let ids = vec![t.clone()];
        e.edit_tokens("delete", None, |s| s.delete(&ids));
        assert_eq!(fill(&e, id), Some(RED));
        assert!(
            e.bound_token(e.doc.get(id).unwrap(), TokenProp::Fill)
                .is_none()
        );
        e.undo();
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, id), Some(BLUE), "the binding is live again");
    }

    #[test]
    fn undo_restores_the_token_and_every_bound_value() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        set_brand(&mut e, &t, "#0000ff");
        e.undo();
        assert_eq!(fill(&e, id), Some(RED));
        e.redo();
        assert_eq!(fill(&e, id), Some(BLUE));
    }

    #[test]
    fn undoing_a_node_edit_never_shows_a_stale_token_value() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        let other = e.add_page();
        set_brand(&mut e, &t, "#0000ff"); // on page 2
        e.switch_page(0);
        e.undo(); // undoes the bind on page 1
        e.redo(); // its snapshot still says red
        assert_eq!(fill(&e, id), Some(BLUE));
        assert_eq!(other, 1);
    }

    #[test]
    fn token_edits_reach_parked_pages_and_mark_them_for_saving() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        e.mark_page_saved(0);
        e.add_page();
        e.mark_page_saved(0);
        e.mark_page_saved(1);
        e.meta_dirty = false;
        let undo_steps = e.pages[0].stash.as_ref().unwrap().history.undo_len();
        set_brand(&mut e, &t, "#0000ff");
        let parked = e.page_doc(0).get(id).unwrap();
        assert_eq!(parked.style.fill(), Some(BLUE));
        assert!(e.dirty_pages().contains(&0));
        assert!(e.meta_dirty, "the manifest holds the tokens");
        let h = &e.pages[0].stash.as_ref().unwrap().history;
        assert_eq!(h.undo_len(), undo_steps, "parked history stays clean");
    }

    #[test]
    fn drags_on_one_token_are_one_undo_step() {
        let (mut e, id, t) = editor_with_rect();
        e.bind_token(&[id], TokenProp::Fill, &t);
        let n = e.history.undo_len();
        for c in ["#110000", "#220000", "#330000"] {
            e.edit_tokens("Edit token", Some(&t), |s| {
                s.update(&t, None, Some(c), None)
            })
            .unwrap();
        }
        assert_eq!(e.history.undo_len(), n + 1);
        e.undo();
        assert_eq!(fill(&e, id), Some(RED));
    }

    #[test]
    fn binding_a_gradient_top_layer_adds_a_solid_fill() {
        let (mut e, id, t) = editor_with_rect();
        e.edit("gradient", |tx| {
            tx.node_mut(id).unwrap().style.fills = vec![Paint::linear(BLUE, RED)];
        });
        e.bind_token(&[id], TokenProp::Fill, &t);
        let n = e.doc.get(id).unwrap();
        assert_eq!(n.style.fills.len(), 2);
        assert_eq!(n.style.fill(), Some(RED));
        assert!(n.tokens.contains_key(&TokenProp::Fill));
    }

    #[test]
    fn spacing_tokens_lay_out_again() {
        let mut e = Editor::default();
        let (frame, a, b) = e.edit("add", |tx| {
            let f = tx.alloc_id();
            let mut n = Node::new(
                f,
                NodeKind::Frame { clip: true },
                Rect::new(0.0, 0.0, 400.0, 100.0),
            );
            n.layout = Some(crate::model::AutoLayout::default());
            tx.insert(n, None, None);
            let a = tx.alloc_id();
            tx.insert(
                Node::new(a, NodeKind::Rect, Rect::new(0.0, 0.0, 20.0, 20.0)),
                Some(f),
                None,
            );
            let b = tx.alloc_id();
            tx.insert(
                Node::new(b, NodeKind::Rect, Rect::new(30.0, 0.0, 20.0, 20.0)),
                Some(f),
                None,
            );
            (f, a, b)
        });
        let t = e
            .edit_tokens("add", None, |s| {
                s.create(TokenKind::Spacing, "gap", "40px", None)
            })
            .unwrap();
        e.bind_token(&[frame], TokenProp::Gap, &t);
        let gap = |e: &Editor| e.doc.get(b).unwrap().rect.x - e.doc.get(a).unwrap().rect.right();
        assert_eq!(gap(&e), 40.0);
        e.edit_tokens("edit", None, |s| s.update(&t, None, Some("4px"), None))
            .unwrap();
        assert_eq!(gap(&e), 4.0);
    }

    #[test]
    fn font_size_tokens_refit_text() {
        let mut e = Editor::default();
        let id = e.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(
                Node::new(
                    id,
                    NodeKind::Text {
                        text: "Hello".into(),
                        font_size: 16.0,
                        auto_width: true,
                        align: Default::default(),
                    },
                    Rect::new(0.0, 0.0, 10.0, 10.0),
                ),
                None,
                None,
            )
        });
        let t = e
            .edit_tokens("add", None, |s| {
                s.create(TokenKind::FontSize, "body", "16px", None)
            })
            .unwrap();
        e.bind_token(&[id], TokenProp::FontSize, &t);
        let w = e.doc.get(id).unwrap().rect.w;
        e.edit_tokens("edit", None, |s| s.update(&t, None, Some("32px"), None))
            .unwrap();
        assert!(e.doc.get(id).unwrap().rect.w > w * 1.5);
    }

    #[test]
    fn instances_follow_a_token_on_their_main_component() {
        let (mut e, id, t) = editor_with_rect();
        e.selection = vec![id];
        e.create_component();
        let main = e.selection[0];
        let child = e
            .doc
            .get(main)
            .map_or(id, |n| n.children.first().copied().unwrap_or(id));
        e.bind_token(&[child], TokenProp::Fill, &t);
        e.selection = vec![main];
        e.instance_of_selection();
        let inst = e.selection[0];
        let inst_child = e.doc.subtree(inst)[e.doc.subtree(inst).len() - 1];
        set_brand(&mut e, &t, "#0000ff");
        assert_eq!(fill(&e, inst_child), Some(BLUE));
        assert!(
            e.doc
                .get(inst_child)
                .unwrap()
                .tokens
                .contains_key(&TokenProp::Fill),
            "the sync with the main component keeps the binding"
        );
    }

    #[test]
    fn pasted_bindings_from_another_project_are_dropped() {
        let (mut e, id, _) = editor_with_rect();
        e.edit("paste", |tx| {
            tx.node_mut(id)
                .unwrap()
                .tokens
                .insert(TokenProp::Fill, "from-elsewhere".into());
        });
        assert!(e.doc.get(id).unwrap().tokens.is_empty());
    }

    fn themed() -> (Editor, NodeId, NodeId, String) {
        let (mut e, rect, _) = editor_with_rect();
        let dark = vec![("dark".to_string(), "#000000".to_string())];
        let bg = e
            .edit_tokens("add", None, |t| {
                t.create_in_modes(TokenKind::Color, "bg", "#ffffff", &dark, None)
            })
            .unwrap();
        let frame = e.edit("add", |tx| {
            let id = tx.alloc_id();
            let r = Rect::new(100.0, 0.0, 50.0, 50.0);
            tx.insert(Node::new(id, NodeKind::Frame { clip: true }, r), None, None)
        });
        e.set_frame_mode(&[frame], Some("dark"));
        e.bind_token(&[rect, frame], TokenProp::Fill, &bg);
        (e, frame, rect, bg)
    }

    #[test]
    fn layers_show_their_frames_mode() {
        let (mut e, frame, rect, bg) = themed();
        assert_eq!(fill(&e, frame), Some(Color::BLACK));
        assert_eq!(fill(&e, rect), Some(Color::WHITE));
        e.edit_tokens("edit", None, |t| {
            t.update_in_mode(&bg, "dark", Some("#0000ff"))
        })
        .unwrap();
        assert_eq!(fill(&e, frame), Some(BLUE));
        assert_eq!(fill(&e, rect), Some(Color::WHITE));
        e.set_frame_mode(&[frame], None);
        assert_eq!(fill(&e, frame), Some(Color::WHITE));
        e.undo();
        assert_eq!(fill(&e, frame), Some(BLUE));
    }

    #[test]
    fn a_layer_moved_into_a_dark_frame_takes_the_dark_value() {
        let (mut e, frame, rect, _) = themed();
        e.edit("move", |tx| tx.reparent(rect, Some(frame), None));
        assert_eq!(fill(&e, rect), Some(Color::BLACK));
        assert!(
            e.doc
                .get(rect)
                .unwrap()
                .tokens
                .contains_key(&TokenProp::Fill)
        );
        e.undo();
        assert_eq!(fill(&e, rect), Some(Color::WHITE));
    }

    #[test]
    fn a_hand_edit_on_an_instance_in_another_mode_detaches() {
        let (mut e, frame, rect, _) = themed();
        e.selection = vec![rect];
        e.create_component();
        e.instance_of_selection();
        let inst = e.selection[0];
        e.edit("move", |tx| tx.reparent(inst, Some(frame), None));
        let child = *e.doc.subtree(inst).last().unwrap();
        assert_eq!(
            fill(&e, child),
            Some(Color::BLACK),
            "the instance shows Dark"
        );
        e.begin("paint");
        e.tx()
            .node_mut(child)
            .unwrap()
            .style
            .set_fill(Some(Color::WHITE));
        e.mark_overrides(&[child], "fills");
        e.commit();
        assert_eq!(fill(&e, child), Some(Color::WHITE));
        assert!(e.doc.get(child).unwrap().tokens.is_empty());
    }

    #[test]
    fn setting_another_modes_value_by_hand_detaches() {
        let (mut e, _, rect, _) = themed();
        e.edit("paint", |tx| {
            tx.node_mut(rect)
                .unwrap()
                .style
                .set_fill(Some(Color::BLACK));
        });
        assert_eq!(fill(&e, rect), Some(Color::BLACK));
        assert!(e.doc.get(rect).unwrap().tokens.is_empty());
    }

    #[test]
    fn groups_fit_text_that_a_mode_change_resizes() {
        let mut e = Editor::default();
        let dark = vec![("dark".to_string(), "48px".to_string())];
        let size = e
            .edit_tokens("add", None, |t| {
                t.create_in_modes(TokenKind::FontSize, "body", "12px", &dark, None)
            })
            .unwrap();
        let (frame, group, text) = e.edit("add", |tx| {
            let f = tx.alloc_id();
            let r = Rect::new(0.0, 0.0, 400.0, 400.0);
            tx.insert(Node::new(f, NodeKind::Frame { clip: false }, r), None, None);
            let g = tx.alloc_id();
            tx.insert(
                Node::new(g, NodeKind::Group, Rect::new(0.0, 0.0, 1.0, 1.0)),
                Some(f),
                None,
            );
            let t = tx.alloc_id();
            let kind = NodeKind::Text {
                text: "Hello".into(),
                font_size: 12.0,
                auto_width: true,
                align: Default::default(),
            };
            tx.insert(
                Node::new(t, kind, Rect::new(0.0, 0.0, 10.0, 10.0)),
                Some(g),
                None,
            );
            (f, g, t)
        });
        e.bind_token(&[text], TokenProp::FontSize, &size);
        e.set_frame_mode(&[frame], Some("dark"));
        let (g, t) = (
            e.doc.get(group).unwrap().rect,
            e.doc.get(text).unwrap().rect,
        );
        assert_eq!(e.doc.get(text).unwrap().kind.font_size(), Some(48.0));
        assert_eq!((g.w, g.h), (t.w, t.h), "the group fits the bigger text");
    }

    #[test]
    fn copied_tokens_survive_a_project_switch() {
        let (mut e, _, t) = editor_with_rect();
        e.copy_tokens(std::slice::from_ref(&t));
        e.load(Default::default(), None, None);
        assert!(e.tokens.is_empty(), "tokens belong to the project");
        assert!(e.can_paste_tokens());
        let new = e.paste_tokens();
        assert_eq!(new.len(), 1);
        assert_ne!(new[0], t);
    }
}
