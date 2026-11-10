//! Apply, detach and track effect styles on layers.

use super::Editor;
use crate::model::effects::edit_effects;
use crate::model::{EffectStyleDef, Node, NodeId, TokenProp, TokenSet};

impl Editor {
    /// Give these layers a style's effects, as one undo step.
    pub fn apply_effect_style(&mut self, ids: &[NodeId], style: &str) {
        self.apply_effect_style_as(ids, style, "Apply effect style", None);
    }

    fn apply_effect_style_as(
        &mut self,
        ids: &[NodeId],
        style: &str,
        label: &str,
        key: Option<&str>,
    ) {
        let targets: Vec<(NodeId, Vec<crate::model::Effect>)> = (ids.iter().copied())
            .filter(|id| self.doc.get(*id).is_some())
            .filter_map(|id| {
                let mode = self.tokens.mode_at(&self.doc, id);
                Some((id, self.tokens.resolve_effect_style_in(style, mode)?))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        self.begin_coalescing(label, key);
        {
            let mut tx = self.tx();
            for (id, effects) in &targets {
                if let Some(n) = tx.node_mut(*id) {
                    edit_effects(n, |l| *l = effects.iter().map(|e| (*e, None)).collect());
                    n.effect_style = Some(style.to_string());
                }
            }
        }
        let targets: Vec<NodeId> = targets.into_iter().map(|(id, _)| id).collect();
        self.mark_overrides(&targets, "effects");
        self.commit();
    }

    /// Create a style from the first layer and apply it to all ids in one undo step.
    /// Return the new style id.
    pub fn create_effect_style_from(&mut self, ids: &[NodeId]) -> Option<String> {
        let from = self.doc.get(*ids.first()?)?.clone();
        let key = format!("effect-style-from:{ids:?}");
        let label = "Create effect style";
        let style = self.edit_tokens(label, Some(&key), |t| {
            t.add_default_effect_style(Some(&from))
        });
        self.apply_effect_style_as(ids, &style, label, Some(&key));
        self.history.seal();
        Some(style)
    }

    /// Detach the style while keeping resolved effects and shadow color token bindings.
    pub fn detach_effect_style(&mut self, ids: &[NodeId]) {
        let tokens = self.tokens.clone();
        self.begin("Detach effect style");
        {
            let mut tx = self.tx();
            for id in ids {
                let Some(n) = tx.node_mut(*id) else { continue };
                let Some(style) = n.effect_style.take() else {
                    continue;
                };
                let Some(s) = tokens.effect_style(&style) else {
                    continue;
                };
                for (i, e) in s.effects.iter().enumerate() {
                    if let Some(t) = e.color_token.as_ref().filter(|t| tokens.get(t).is_some()) {
                        n.tokens.insert(TokenProp::EffectColor(i), t.clone());
                    }
                }
            }
        }
        self.mark_overrides(ids, "effects");
        self.commit();
    }

    /// The effect style `n` uses, if it still exists.
    pub fn bound_effect_style(&self, n: &Node) -> Option<&EffectStyleDef> {
        self.tokens.effect_style(n.effect_style.as_ref()?)
    }

    /// Drop missing styles and hand-edited bindings. Accept values from `kept_modes`.
    /// Run after `drop_broken_bindings` so remaining color bindings identify hand edits.
    pub(super) fn check_effect_style_refs(&mut self) {
        let Some((rec, _)) = &self.rec else { return };
        let mut dropped: Vec<NodeId> = Vec::new();
        for id in rec.touched() {
            let Some(n) = self.doc.get(*id) else { continue };
            let Some(style) = &n.effect_style else {
                continue;
            };
            if self.tokens.effect_style(style).is_none() {
                dropped.push(*id);
                continue;
            }
            let modes = self.kept_modes(*id, "effects").unwrap_or_else(|| {
                (self.tokens.choices().iter())
                    .map(|m| Some(m.name.as_str()))
                    .collect()
            });
            let shown: Vec<_> = (modes.into_iter())
                .filter_map(|m| self.tokens.resolve_effect_style_in(style, m))
                .collect();
            if shown.is_empty() {
                continue;
            }
            let color_bound = (n.tokens.keys()).any(|p| matches!(p, TokenProp::EffectColor(_)));
            if color_bound || !shown.contains(&n.style.effects) {
                dropped.push(*id);
            }
        }
        if dropped.is_empty() {
            return;
        }
        let mut tx = self.tx();
        for id in dropped {
            if let Some(n) = tx.node_mut(id) {
                n.effect_style = None;
            }
        }
    }

    pub(super) fn effect_style_is_stale(&self, n: &Node, mode: Option<&str>) -> bool {
        (n.effect_style.as_ref())
            .and_then(|s| self.tokens.resolve_effect_style_in(s, mode))
            .is_some_and(|effects| effects != n.style.effects)
    }

    pub(super) fn write_effect_style_values(tokens: &TokenSet, n: &mut Node, mode: Option<&str>) {
        let resolved =
            (n.effect_style.as_ref()).and_then(|s| tokens.resolve_effect_style_in(s, mode));
        if let Some(effects) = resolved {
            n.style.effects = effects;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Color, Effect, EffectKind, NodeKind, StyleEffect, TokenKind};

    fn rect(e: &mut Editor) -> NodeId {
        e.edit("add", |tx| {
            let id = tx.alloc_id();
            tx.insert(
                Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0)),
                None,
                None,
            )
        })
    }

    fn shadow(blur: f64) -> Effect {
        Effect {
            blur,
            ..Effect::new(EffectKind::DropShadow)
        }
    }

    fn setup() -> (Editor, NodeId, String, String) {
        let mut e = Editor::default();
        let id = rect(&mut e);
        let (token, style) = e.edit_tokens("add", None, |t| {
            let dark = vec![("dark".to_string(), "#ffffff".to_string())];
            let token = t
                .create_in_modes(TokenKind::Color, "shade", "#000000", &dark, None)
                .unwrap();
            let effects = vec![StyleEffect {
                effect: shadow(4.0),
                color_token: Some(token.clone()),
            }];
            let style = t.create_effect_style("Elevation/2", effects, None).unwrap();
            (token, style)
        });
        (e, id, token, style)
    }

    fn effects(e: &Editor, id: NodeId) -> Vec<Effect> {
        e.doc.get(id).unwrap().style.effects.clone()
    }

    fn bound(e: &Editor, id: NodeId) -> bool {
        e.bound_effect_style(e.doc.get(id).unwrap()).is_some()
    }

    fn set_blur(e: &mut Editor, style: &str, blur: f64) {
        e.edit_tokens("Edit effect style", None, |t| {
            let mut effects = t.effect_style(style).unwrap().effects.clone();
            effects[0].effect.blur = blur;
            t.update_effect_style(style, None, Some(effects), None)
        })
        .unwrap();
    }

    #[test]
    fn a_style_edit_is_one_undo_step_on_every_page() {
        let (mut e, id, _, style) = setup();
        e.apply_effect_style(&[id], &style);
        assert_eq!(effects(&e, id)[0].color, Color::BLACK);
        e.add_page();
        let steps = e.history.undo_len();
        set_blur(&mut e, &style, 12.0);
        assert_eq!(e.history.undo_len(), steps + 1);
        let blur = |e: &Editor| e.page_doc(0).get(id).unwrap().style.effects[0].blur;
        assert_eq!(blur(&e), 12.0);
        e.undo();
        assert_eq!(blur(&e), 4.0);
    }

    #[test]
    fn an_aliased_token_carries_changes_through() {
        let (mut e, id, token, style) = setup();
        e.apply_effect_style(&[id], &style);
        e.edit_tokens("edit", None, |t| {
            t.update(&token, None, Some("#ff0000"), None)
        })
        .unwrap();
        assert_eq!(effects(&e, id)[0].color, Color::rgb(255, 0, 0));
        assert!(bound(&e, id));
    }

    #[test]
    fn layers_show_the_style_in_their_frames_mode() {
        let (mut e, id, _, style) = setup();
        e.apply_effect_style(&[id], &style);
        let frame = e.edit("add", |tx| {
            let f = tx.alloc_id();
            let r = Rect::new(0.0, 0.0, 400.0, 400.0);
            tx.insert(Node::new(f, NodeKind::Frame { clip: false }, r), None, None)
        });
        e.set_frame_mode(&[frame], Some("dark"));
        e.edit("move", |tx| tx.reparent(id, Some(frame), None));
        assert_eq!(effects(&e, id)[0].color, Color::WHITE);
        assert!(bound(&e, id), "a mode's value isn't a hand edit");
        let other = rect(&mut e);
        e.edit("move", |tx| tx.reparent(other, Some(frame), None));
        e.apply_effect_style(&[other], &style);
        assert_eq!(effects(&e, other)[0].color, Color::WHITE);
    }

    #[test]
    fn editing_the_effects_by_hand_drops_the_style() {
        let (mut e, id, _, style) = setup();
        e.apply_effect_style(&[id], &style);
        e.edit("move", |tx| tx.node_mut(id).unwrap().rect.x = 5.0);
        assert!(bound(&e, id), "other edits keep it");
        e.edit("blur", |tx| {
            tx.node_mut(id).unwrap().style.effects[0].blur = 9.0
        });
        assert!(!bound(&e, id));
        set_blur(&mut e, &style, 12.0);
        assert_eq!(effects(&e, id)[0].blur, 9.0);
    }

    #[test]
    fn detaching_keeps_the_effects_and_the_color_token() {
        let (mut e, id, token, style) = setup();
        e.apply_effect_style(&[id], &style);
        assert!(e.doc.get(id).unwrap().tokens.is_empty());
        e.detach_effect_style(&[id]);
        assert!(!bound(&e, id));
        let n = e.doc.get(id).unwrap();
        assert_eq!(n.tokens.get(&TokenProp::EffectColor(0)), Some(&token));
        set_blur(&mut e, &style, 12.0);
        assert_eq!(effects(&e, id)[0].blur, 4.0);
        e.edit_tokens("edit", None, |t| {
            t.update(&token, None, Some("#ff0000"), None)
        })
        .unwrap();
        assert_eq!(effects(&e, id)[0].color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn deleting_a_style_keeps_effects_and_undo_brings_it_back() {
        let (mut e, id, _, style) = setup();
        e.apply_effect_style(&[id], &style);
        let ids = vec![style.clone()];
        e.edit_tokens("delete", None, |t| t.delete_effect_styles(&ids));
        assert_eq!(effects(&e, id)[0].blur, 4.0);
        assert!(!bound(&e, id));
        e.undo();
        assert!(bound(&e, id));
        e.redo();
        e.edit("move", |tx| tx.node_mut(id).unwrap().rect.x = 5.0);
        assert!(e.doc.get(id).unwrap().effect_style.is_none());
    }

    #[test]
    fn a_style_made_from_a_layer_is_applied_in_one_step() {
        let (mut e, id, token, _) = setup();
        e.edit("effects", |tx| {
            let n = tx.node_mut(id).unwrap();
            n.style.effects = vec![Effect {
                color: Color::BLACK,
                ..shadow(6.0)
            }];
            n.tokens.insert(TokenProp::EffectColor(0), token.clone());
        });
        let steps = e.history.undo_len();
        let style = e.create_effect_style_from(&[id]).unwrap();
        assert_eq!(e.history.undo_len(), steps + 1);
        let s = e.tokens.effect_style(&style).unwrap();
        assert_eq!(s.effects[0].color_token, Some(token));
        assert!(bound(&e, id));
        assert!(
            e.doc.get(id).unwrap().tokens.is_empty(),
            "the style owns the color"
        );
        e.undo();
        assert!(e.tokens.effect_style(&style).is_none());
        assert!(e.doc.get(id).unwrap().effect_style.is_none());
    }

    #[test]
    fn instances_follow_a_style_on_their_main_component() {
        let (mut e, id, _, style) = setup();
        e.apply_effect_style(&[id], &style);
        e.selection = vec![id];
        e.create_component();
        e.instance_of_selection();
        let inst = e.selection[0];
        let inst_rect = *e.doc.subtree(inst).last().unwrap();
        set_blur(&mut e, &style, 12.0);
        assert_eq!(effects(&e, inst_rect)[0].blur, 12.0);
        assert!(bound(&e, inst_rect));
    }
}
