//! Local edits and token bindings override style fields without detaching the style.

use super::Editor;
use crate::model::{Node, NodeId, NodeKind, StyleField, TextStyleDef, TextStyleRef};

impl Editor {
    /// Apply a style as one undo step, clearing field overrides and competing token bindings.
    pub fn apply_text_style(&mut self, ids: &[NodeId], style: &str) {
        let targets: Vec<(NodeId, crate::model::ResolvedStyle)> = (self.text_layers(ids))
            .into_iter()
            .filter_map(|id| {
                let mode = self.tokens.mode_at(&self.doc, id);
                Some((id, self.tokens.resolve_style_in(style, mode)?))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        self.begin("Apply text style");
        {
            let mut tx = self.tx();
            for (id, r) in &targets {
                if let Some(n) = tx.node_mut(*id) {
                    for f in StyleField::ALL {
                        r.write(f, n);
                        if let Some(p) = f.prop() {
                            n.tokens.remove(&p);
                        }
                    }
                    n.text_style_ref = Some(TextStyleRef {
                        id: style.to_string(),
                        overrides: Default::default(),
                    });
                }
            }
        }
        let targets: Vec<NodeId> = targets.into_iter().map(|(id, _)| id).collect();
        self.mark_overrides(&targets, "font");
        for id in &targets {
            self.fit_text(*id);
        }
        self.commit();
    }

    /// Stop these layers using their text style. They keep the values they show.
    pub fn detach_text_style(&mut self, ids: &[NodeId]) {
        self.begin("Detach text style");
        {
            let mut tx = self.tx();
            for id in ids {
                if let Some(n) = tx.node_mut(*id) {
                    n.text_style_ref = None;
                }
            }
        }
        self.mark_overrides(ids, "font");
        self.commit();
    }

    /// The text style `n` uses, if it still exists.
    pub fn bound_style(&self, n: &Node) -> Option<&TextStyleDef> {
        self.tokens.style(&n.text_style_ref.as_ref()?.id)
    }

    fn text_layers(&self, ids: &[NodeId]) -> Vec<NodeId> {
        ids.iter()
            .copied()
            .filter(|id| {
                self.doc
                    .get(*id)
                    .is_some_and(|n| matches!(n.kind, NodeKind::Text { .. }))
            })
            .collect()
    }

    /// Run after `drop_broken_bindings` so only surviving token bindings become overrides.
    /// Like there, values from the modes in `kept_modes` count as the style's.
    pub(super) fn check_text_style_refs(&mut self) {
        let Some((rec, _)) = &self.rec else { return };
        let mut changes: Vec<(NodeId, Option<TextStyleRef>)> = Vec::new();
        for id in rec.touched() {
            let Some(n) = self.doc.get(*id) else { continue };
            let Some(r) = &n.text_style_ref else { continue };
            let is_text = matches!(n.kind, NodeKind::Text { .. });
            if !is_text || self.tokens.style(&r.id).is_none() {
                changes.push((*id, None));
                continue;
            }
            let modes = self.kept_modes(*id, "font").unwrap_or_else(|| {
                (self.tokens.choices().iter())
                    .map(|m| Some(m.name.as_str()))
                    .collect()
            });
            let styles: Vec<_> = (modes.into_iter())
                .filter_map(|m| self.tokens.resolve_style_in(&r.id, m))
                .collect();
            if styles.is_empty() {
                continue;
            }
            let mut next = r.clone();
            for f in StyleField::ALL {
                let token_owned = f.prop().is_some_and(|p| n.tokens.contains_key(&p));
                if token_owned || !styles.iter().any(|s| s.holds(f, n)) {
                    next.overrides.insert(f);
                }
            }
            if next != *r {
                changes.push((*id, Some(next)));
            }
        }
        if changes.is_empty() {
            return;
        }
        let mut tx = self.tx();
        for (id, r) in changes {
            if let Some(n) = tx.node_mut(id) {
                n.text_style_ref = r;
            }
        }
    }

    pub(super) fn style_is_stale(&self, n: &Node, mode: Option<&str>) -> bool {
        let Some(r) = &n.text_style_ref else {
            return false;
        };
        let Some(style) = self.tokens.resolve_style_in(&r.id, mode) else {
            return false;
        };
        StyleField::ALL
            .into_iter()
            .any(|f| !r.overrides.contains(&f) && !style.holds(f, n))
    }

    pub(super) fn write_style_values(
        tokens: &crate::model::TokenSet,
        n: &mut Node,
        mode: Option<&str>,
    ) {
        let Some(r) = &n.text_style_ref else { return };
        let Some(style) = tokens.resolve_style_in(&r.id, mode) else {
            return;
        };
        let overrides = r.overrides.clone();
        for f in StyleField::ALL {
            if !overrides.contains(&f) {
                style.write(f, n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{StyleInput, TokenKind, TokenProp};

    fn text(e: &mut Editor) -> NodeId {
        e.edit("add", |tx| {
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
        })
    }

    #[test]
    fn applying_a_style_in_a_dark_frame_takes_its_dark_values() {
        let mut e = Editor::default();
        let t = text(&mut e);
        let frame = e.edit("add", |tx| {
            let f = tx.alloc_id();
            let r = Rect::new(0.0, 0.0, 400.0, 400.0);
            tx.insert(Node::new(f, NodeKind::Frame { clip: false }, r), None, None);
            tx.reparent(t, Some(f), None);
            f
        });
        e.set_frame_mode(&[frame], Some("dark"));
        let style = e.edit_tokens("add", None, |s| {
            let dark = vec![("dark".to_string(), "48px".to_string())];
            s.create_in_modes(TokenKind::FontSize, "size", "12px", &dark, None)
                .unwrap();
            let mut i = StyleInput::default();
            i.set(StyleField::Size, "var(--size)".into());
            s.create_style("Body", &i, None).unwrap()
        });
        e.apply_text_style(&[t], &style);
        assert_eq!(size(&e, t), Some(48.0));
        let r = e.doc.get(t).unwrap().text_style_ref.clone().unwrap();
        assert!(r.overrides.is_empty(), "{:?}", r.overrides);

        // Deleting the token keeps the style's Dark value, so the layer doesn't shrink.
        let token = e.tokens.by_css_name("--size").unwrap().id.clone();
        e.edit_tokens("delete", None, |s| s.delete(&[token]));
        assert_eq!(size(&e, t), Some(48.0));
        let r = e.doc.get(t).unwrap().text_style_ref.clone().unwrap();
        assert!(r.overrides.is_empty(), "{:?}", r.overrides);
        let other = text(&mut e);
        e.edit("move", |tx| tx.reparent(other, Some(frame), None));
        e.apply_text_style(&[other], &style);
        assert_eq!(size(&e, other), Some(48.0));
    }

    fn setup() -> (Editor, NodeId, String, String) {
        let mut e = Editor::default();
        let id = text(&mut e);
        let (token, style) = e.edit_tokens("add", None, |t| {
            let token = t
                .create(TokenKind::FontSize, "size-l", "32px", None)
                .unwrap();
            let mut i = StyleInput::default();
            i.set(StyleField::Size, "var(--size-l)".into());
            i.set(StyleField::Weight, "700".into());
            (token, t.create_style("Heading", &i, None).unwrap())
        });
        (e, id, token, style)
    }

    fn size(e: &Editor, id: NodeId) -> Option<f64> {
        e.doc.get(id).unwrap().kind.font_size()
    }

    fn overrides(e: &Editor, id: NodeId) -> Vec<StyleField> {
        let r = e.doc.get(id).unwrap().text_style_ref.clone().unwrap();
        r.overrides.into_iter().collect()
    }

    fn set_size(e: &mut Editor, token: &str, v: &str) {
        e.edit_tokens("edit", None, |t| t.update(token, None, Some(v), None))
            .unwrap();
    }

    #[test]
    fn editing_a_field_by_hand_overrides_only_that_field() {
        let (mut e, id, token, style) = setup();
        e.apply_text_style(&[id], &style);
        e.edit("size", |tx| {
            if let NodeKind::Text { font_size, .. } = &mut tx.node_mut(id).unwrap().kind {
                *font_size = 20.0;
            }
        });
        assert_eq!(overrides(&e, id), [StyleField::Size]);
        set_size(&mut e, &token, "40px");
        assert_eq!(size(&e, id), Some(20.0), "the override stays");
        e.edit_tokens("edit", None, |t| {
            let mut i = StyleInput::default();
            i.set(StyleField::Weight, "300".into());
            t.update_style(&style, None, &i, None)
        })
        .unwrap();
        assert_eq!(e.doc.get(id).unwrap().text_style.base_weight(), 300);
        e.apply_text_style(&[id], &style);
        assert!(overrides(&e, id).is_empty(), "applying again resets");
        assert_eq!(size(&e, id), Some(40.0));
    }

    #[test]
    fn a_bound_token_overrides_the_field() {
        let (mut e, id, _, style) = setup();
        e.apply_text_style(&[id], &style);
        let other = e
            .edit_tokens("add", None, |t| {
                t.create(TokenKind::FontSize, "size-s", "12px", None)
            })
            .unwrap();
        e.bind_token(&[id], TokenProp::FontSize, &other);
        assert_eq!(overrides(&e, id), [StyleField::Size]);
        e.apply_text_style(&[id], &style);
        assert!(e.doc.get(id).unwrap().tokens.is_empty());
    }

    #[test]
    fn a_style_edit_is_one_undo_step_on_every_page() {
        let (mut e, id, _, style) = setup();
        e.apply_text_style(&[id], &style);
        e.add_page();
        let steps = e.history.undo_len();
        e.edit_tokens("Edit text style", None, |t| {
            let mut i = StyleInput::default();
            i.set(StyleField::Size, "48px".into());
            t.update_style(&style, None, &i, None)
        })
        .unwrap();
        assert_eq!(e.history.undo_len(), steps + 1);
        assert_eq!(e.page_doc(0).get(id).unwrap().kind.font_size(), Some(48.0));
        e.undo();
        assert_eq!(e.page_doc(0).get(id).unwrap().kind.font_size(), Some(32.0));
    }

    #[test]
    fn deleting_a_style_keeps_values_and_undo_brings_it_back() {
        let (mut e, id, _, style) = setup();
        e.apply_text_style(&[id], &style);
        let ids = vec![style.clone()];
        e.edit_tokens("delete", None, |t| t.delete_styles(&ids));
        assert_eq!(size(&e, id), Some(32.0));
        assert!(e.bound_style(e.doc.get(id).unwrap()).is_none());
        e.undo();
        assert!(e.bound_style(e.doc.get(id).unwrap()).is_some());
        e.redo();
        e.edit("move", |tx| tx.node_mut(id).unwrap().rect.x = 5.0);
        assert!(e.doc.get(id).unwrap().text_style_ref.is_none());
    }

    #[test]
    fn detaching_keeps_values() {
        let (mut e, id, token, style) = setup();
        e.apply_text_style(&[id], &style);
        e.detach_text_style(&[id]);
        set_size(&mut e, &token, "40px");
        assert_eq!(size(&e, id), Some(32.0));
    }

    #[test]
    fn instances_follow_a_style_on_their_main_component() {
        let (mut e, id, token, style) = setup();
        e.apply_text_style(&[id], &style);
        e.selection = vec![id];
        e.create_component();
        e.instance_of_selection();
        let inst = e.selection[0];
        let inst_text = *e.doc.subtree(inst).last().unwrap();
        set_size(&mut e, &token, "40px");
        assert_eq!(size(&e, inst_text), Some(40.0));
        assert!(e.doc.get(inst_text).unwrap().text_style_ref.is_some());
    }
}
