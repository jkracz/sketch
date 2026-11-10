use super::node::{Color, Node};
use super::tokens::TokenProp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    DropShadow,
    InnerShadow,
    /// Blurs the layer and its children.
    LayerBlur,
    /// Blurs what is behind the layer, inside its shape.
    BackgroundBlur,
}

impl EffectKind {
    pub const ALL: [EffectKind; 4] = [
        EffectKind::DropShadow,
        EffectKind::InnerShadow,
        EffectKind::LayerBlur,
        EffectKind::BackgroundBlur,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EffectKind::DropShadow => "Drop shadow",
            EffectKind::InnerShadow => "Inner shadow",
            EffectKind::LayerBlur => "Layer blur",
            EffectKind::BackgroundBlur => "Background blur",
        }
    }

    pub fn is_shadow(self) -> bool {
        matches!(self, EffectKind::DropShadow | EffectKind::InnerShadow)
    }
}

/// One effect on a layer. Lengths are in px and mean what they do in CSS.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    #[serde(rename = "type")]
    pub kind: EffectKind,
    /// A shadow's `box-shadow` blur radius, or a blur's `blur()` radius.
    pub blur: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub x: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub y: f64,
    /// Positive spread grows drop shadows and shrinks inner shadows.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spread: f64,
    #[serde(default = "shadow_color", skip_serializing_if = "is_shadow_color")]
    pub color: Color,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn yes() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
fn shadow_color() -> Color {
    Color::rgba(0, 0, 0, 64)
}
fn is_shadow_color(c: &Color) -> bool {
    *c == shadow_color()
}

impl Effect {
    pub fn new(kind: EffectKind) -> Effect {
        let shadow = kind.is_shadow();
        Effect {
            kind,
            blur: if shadow { 4.0 } else { 8.0 },
            x: 0.0,
            y: if shadow { 4.0 } else { 0.0 },
            spread: 0.0,
            color: shadow_color(),
            visible: true,
        }
    }

    /// Standard deviation of the Gaussian. CSS gives shadows twice it and `blur()` exactly it.
    pub fn sigma(&self) -> f64 {
        let blur = self.blur.max(0.0);
        if self.kind.is_shadow() {
            blur * 0.5
        } else {
            blur
        }
    }
}

/// Combined sigma of the visible effects of `kind`. Blurring twice adds the variances.
pub fn blur_sigma(effects: &[Effect], kind: EffectKind) -> f64 {
    effects
        .iter()
        .filter(|e| e.visible && e.kind == kind)
        .map(|e| e.sigma().powi(2))
        .sum::<f64>()
        .sqrt()
}

/// An effect and the token bound to its color.
pub type BoundEffect = (Effect, Option<String>);

/// Bindings use effect indices. Route list edits through here to keep bindings with effects.
pub fn edit_effects(n: &mut Node, f: impl FnOnce(&mut Vec<BoundEffect>)) {
    let mut list: Vec<BoundEffect> = std::mem::take(&mut n.style.effects)
        .into_iter()
        .enumerate()
        .map(|(i, e)| (e, n.tokens.remove(&TokenProp::EffectColor(i))))
        .collect();
    n.tokens
        .retain(|p, _| !matches!(p, TokenProp::EffectColor(_)));
    f(&mut list);
    for (i, (e, token)) in list.into_iter().enumerate() {
        if let Some(t) = token.filter(|_| e.kind.is_shadow()) {
            n.tokens.insert(TokenProp::EffectColor(i), t);
        }
        n.style.effects.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_keep_only_what_they_use_on_disk() {
        let blur = Effect::new(EffectKind::LayerBlur);
        let json = serde_json::to_string(&blur).unwrap();
        assert_eq!(json, r#"{"type":"layer-blur","blur":8.0}"#);
        assert_eq!(serde_json::from_str::<Effect>(&json).unwrap(), blur);
        let minimal: Effect = serde_json::from_str(r#"{"type":"drop-shadow","blur":2}"#).unwrap();
        assert!(minimal.visible);
        assert_eq!((minimal.x, minimal.y, minimal.spread), (0.0, 0.0, 0.0));
    }

    #[test]
    fn sigma_follows_css() {
        assert_eq!(Effect::new(EffectKind::DropShadow).sigma(), 2.0);
        assert_eq!(Effect::new(EffectKind::BackgroundBlur).sigma(), 8.0);
        let mut hidden = Effect::new(EffectKind::LayerBlur);
        hidden.visible = false;
        let both = [
            Effect {
                blur: 3.0,
                ..Effect::new(EffectKind::LayerBlur)
            },
            Effect {
                blur: 4.0,
                ..Effect::new(EffectKind::LayerBlur)
            },
            hidden,
        ];
        assert_eq!(blur_sigma(&both, EffectKind::LayerBlur), 5.0);
        assert_eq!(blur_sigma(&both, EffectKind::BackgroundBlur), 0.0);
    }
}
