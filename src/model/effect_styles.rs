//! Named effect lists in [`TokenSet`]. Layers store resolved copies. The editor updates
//! them when the style changes and drops the link after hand edits.

use super::effects::{Effect, EffectKind, shadow_color};
use super::node::Node;
use super::text_styles::style_slug;
use super::tokens::{
    Keyed, TokenKind, TokenProp, TokenSet, TokenValue, check_description, linked_id,
    replace_linked, var_name,
};
use crate::mcp::css;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One effect of a style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleEffect {
    #[serde(flatten)]
    pub effect: Effect,
    /// Color token id for a shadow. When set, `effect.color` holds the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_token: Option<String>,
}

impl StyleEffect {
    pub fn literal(effect: Effect) -> StyleEffect {
        StyleEffect {
            effect,
            color_token: None,
        }
    }

    // Reset unused fields so equivalent styles compare equal.
    fn normalized(mut self) -> StyleEffect {
        if !self.effect.kind.is_shadow() {
            self.color_token = None;
        }
        if self.color_token.is_some() {
            self.effect.color = shadow_color();
        }
        self
    }
}

/// A named effect style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectStyleDef {
    pub id: String,
    /// User-facing name, such as `Elevation/2`. See [`TokenSet::effect_style_var`].
    pub name: String,
    pub effects: Vec<StyleEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Source design system id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

impl Keyed for EffectStyleDef {
    fn key(&self) -> &str {
        &self.id
    }
}

impl EffectStyleDef {
    pub fn uses_any(&self, ids: &HashSet<&str>) -> bool {
        (self.effects.iter()).any(|e| e.color_token.as_deref().is_some_and(|t| ids.contains(t)))
    }

    fn aliases_mut(&mut self) -> impl Iterator<Item = &mut StyleEffect> {
        self.effects.iter_mut().filter(|e| e.color_token.is_some())
    }
}

/// CSS needs separate variables for shadows, layer blur and backdrop blur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectVar {
    /// `--elevation-2`, a `box-shadow` value.
    Shadows,
    /// `--elevation-2-blur`, a `filter` value.
    Blur,
    /// `--elevation-2-backdrop-blur`, a `backdrop-filter` value.
    BackdropBlur,
}

impl EffectVar {
    pub const ALL: [EffectVar; 3] = [EffectVar::Shadows, EffectVar::Blur, EffectVar::BackdropBlur];

    fn suffix(self) -> &'static str {
        match self {
            EffectVar::Shadows => "",
            EffectVar::Blur => "-blur",
            EffectVar::BackdropBlur => "-backdrop-blur",
        }
    }

    /// The CSS property that takes the variable, in kebab case.
    pub fn property(self) -> &'static str {
        match self {
            EffectVar::Shadows => "box-shadow",
            EffectVar::Blur => "filter",
            EffectVar::BackdropBlur => "backdrop-filter",
        }
    }
}

fn var_slugs(slug: &str) -> [String; 3] {
    EffectVar::ALL.map(|v| format!("{slug}{}", v.suffix()))
}

fn numbered(base: &str, i: usize) -> String {
    if i < 2 {
        base.to_string()
    } else {
        format!("{base} {i}")
    }
}

impl TokenSet {
    /// Every effect style, local ones first in creation order.
    pub fn effect_styles(&self) -> &[EffectStyleDef] {
        &self.effect_styles
    }

    pub fn effect_style(&self, id: &str) -> Option<&EffectStyleDef> {
        self.effect_styles.iter().find(|s| s.id == id)
    }

    /// Restore effect styles stored separately in the project manifest.
    pub fn with_effect_styles(mut self, styles: Vec<EffectStyleDef>) -> TokenSet {
        self.effect_styles = styles;
        self
    }

    /// CSS variable name without dashes, including the design system prefix.
    pub fn effect_style_slug(&self, s: &EffectStyleDef) -> String {
        match s.system.as_deref().and_then(|id| self.system(id)) {
            Some(sys) => format!("{}-{}", sys.prefix, style_slug(&s.name)),
            None => style_slug(&s.name),
        }
    }

    /// The style's CSS variable: `Elevation/2` is `--elevation-2`.
    pub fn effect_style_var(&self, s: &EffectStyleDef) -> String {
        format!("--{}", self.effect_style_slug(s))
    }

    /// Find a style and variable kind by CSS name, with or without `--`.
    pub fn effect_style_by_var(&self, name: &str) -> Option<(&EffectStyleDef, EffectVar)> {
        let want = name.trim().strip_prefix("--").unwrap_or(name.trim());
        self.effect_styles.iter().find_map(|s| {
            let slugs = var_slugs(&self.effect_style_slug(s));
            let at = slugs.iter().position(|v| v == want)?;
            Some((s, EffectVar::ALL[at]))
        })
    }

    /// The name of one of the style's variables, such as `--elevation-2-blur`.
    pub fn effect_var_name(&self, s: &EffectStyleDef, var: EffectVar) -> String {
        format!("{}{}", self.effect_style_var(s), var.suffix())
    }

    /// Whether the style's shadows work as a `text-shadow`, which has no `inset` or spread.
    pub fn effect_style_fits_text(&self, s: &EffectStyleDef) -> bool {
        (s.effects.iter().map(|e| &e.effect))
            .filter(|e| e.visible && e.kind.is_shadow())
            .all(|e| e.kind == EffectKind::DropShadow && e.spread == 0.0)
    }

    /// Find a style by variable (`--elevation-2` or `elevation-2`) or by name (`Elevation/2`).
    pub fn effect_style_by_name(&self, name: &str) -> Option<&EffectStyleDef> {
        let name = name.trim();
        let want = name.strip_prefix("--").unwrap_or(name);
        self.effect_styles
            .iter()
            .find(|s| self.effect_style_slug(s) == want)
            .or_else(|| self.effect_styles.iter().find(|s| s.name == name))
    }

    /// Why a style can't be edited here: it belongs to a design system.
    pub fn effect_style_locked(&self, id: &str) -> Option<String> {
        let s = self.effect_style(id)?;
        let sys = self.system(s.system.as_deref()?)?;
        Some(format!(
            "\"{}\" belongs to the {} design system, so it can't be changed in this file. \
             Detach the system to edit it.",
            self.effect_style_var(s),
            sys.name
        ))
    }

    /// Resolve color aliases in `mode`. Returns None for a broken alias.
    pub fn resolve_effect_style_in(&self, id: &str, mode: Option<&str>) -> Option<Vec<Effect>> {
        let s = self.effect_style(id)?;
        (s.effects.iter())
            .map(|e| {
                let mut out = e.effect;
                if let Some(t) = &e.color_token {
                    match self.resolve_in(t, mode)? {
                        TokenValue::Color(c) => out.color = c,
                        _ => return None,
                    }
                }
                Some(out)
            })
            .collect()
    }

    /// The token effect `i` of `s` takes its color from.
    pub fn effect_style_color_token(&self, s: &EffectStyleDef, i: usize) -> Option<&super::Token> {
        self.get(s.effects.get(i)?.color_token.as_deref()?)
    }

    /// CSS with token aliases as `var(--token)`. Returns None without visible effects of `var`.
    pub fn effect_style_css(&self, s: &EffectStyleDef, var: EffectVar) -> Option<String> {
        let effects: Vec<Effect> = s.effects.iter().map(|e| e.effect).collect();
        match var {
            EffectVar::Shadows => css::shadow_list_css(&effects, false, |i, c| {
                match self.effect_style_color_token(s, i) {
                    Some(t) => format!("var({})", self.css_name(t)),
                    None => css::color_css(c),
                }
            }),
            EffectVar::Blur => css::blur_list_css(&effects, EffectKind::LayerBlur),
            EffectVar::BackdropBlur => css::blur_list_css(&effects, EffectKind::BackgroundBlur),
        }
    }

    /// The style as one line of CSS: `box-shadow: 0px 1px 2px #00000040; filter: blur(4px)`.
    pub fn effect_style_short(&self, s: &EffectStyleDef) -> String {
        let parts: Vec<String> = EffectVar::ALL
            .into_iter()
            .filter_map(|v| {
                Some(format!(
                    "{}: {}",
                    v.property(),
                    self.effect_style_css(s, v)?
                ))
            })
            .collect();
        if parts.is_empty() {
            "box-shadow: none".into()
        } else {
            parts.join("; ")
        }
    }

    /// The variables `s` writes, as (name, value): always its shadows (`none` without any),
    /// and a blur variable for each kind of blur it has.
    pub fn effect_style_decls(&self, s: &EffectStyleDef) -> Vec<(String, String)> {
        EffectVar::ALL
            .into_iter()
            .filter_map(|v| {
                let value = match (v, self.effect_style_css(s, v)) {
                    (_, Some(css)) => css,
                    (EffectVar::Shadows, None) => "none".into(),
                    _ => return None,
                };
                Some((self.effect_var_name(s, v), value))
            })
            .collect()
    }

    /// Encode a style's id and resolved effects for [`css::EFFECT_STYLE_MARKER`].
    /// CSS cannot represent every effect list.
    pub(super) fn effect_style_marker(
        &self,
        key: &str,
        value: &str,
        mode: Option<&str>,
    ) -> Option<String> {
        let s = match key {
            "effectStyle" => self.effect_style_by_name(value)?,
            _ => {
                let (s, var) = self.effect_style_by_var(var_name(value)?)?;
                let takes = match var {
                    EffectVar::Shadows => matches!(key, "boxShadow" | "textShadow"),
                    EffectVar::Blur => key == "filter",
                    EffectVar::BackdropBlur => css::is_backdrop_filter(key),
                };
                takes.then_some(s)?
            }
        };
        let effects = self.resolve_effect_style_in(&s.id, mode)?;
        Some(format!(
            "{} {}",
            s.id,
            serde_json::to_string(&effects).ok()?
        ))
    }

    /// The literal CSS an effect style's variable stands for inside a longer value, such as
    /// `var(--elevation-2), 0 0 0 1px red`.
    pub(super) fn effect_var_literal(&self, name: &str, mode: Option<&str>) -> Option<String> {
        let (s, var) = self.effect_style_by_var(name)?;
        let effects = self.resolve_effect_style_in(&s.id, mode)?;
        match var {
            EffectVar::Shadows => css::shadow_list_css(&effects, false, |_, c| css::color_css(c)),
            EffectVar::Blur => css::blur_list_css(&effects, EffectKind::LayerBlur),
            EffectVar::BackdropBlur => css::blur_list_css(&effects, EffectKind::BackgroundBlur),
        }
    }

    /// Mode changes require redeclaring the style because `var()` resolves where it is set.
    pub(super) fn effect_style_varies(&self, s: &EffectStyleDef) -> bool {
        let first = self.resolve_effect_style_in(&s.id, None);
        (self.choices().into_iter().skip(1))
            .any(|m| self.resolve_effect_style_in(&s.id, Some(&m.name)) != first)
    }

    /// Find the owner of `slug`, without dashes, excluding the style id `except`.
    pub(super) fn effect_var_taken(&self, slug: &str, except: &str) -> Option<&EffectStyleDef> {
        self.effect_styles.iter().find(|s| {
            s.id != except
                && var_slugs(&self.effect_style_slug(s))
                    .iter()
                    .any(|v| v == slug)
        })
    }

    fn check_effect_style_name(&self, id: &str, name: &str) -> Result<(), String> {
        let slug = style_slug(name);
        if slug.is_empty() {
            return Err(format!(
                "Effect style name \"{name}\" needs a letter or number to work as a CSS variable."
            ));
        }
        let ours = var_slugs(&slug);
        if let Some(other) = ours.iter().find_map(|v| self.effect_var_taken(v, id)) {
            return Err(format!(
                "The effect style \"{}\" already uses --{slug} or one of its blur variables. \
                 Pick another name.",
                other.name
            ));
        }
        if let Some(t) = self
            .sorted()
            .into_iter()
            .find(|t| ours.contains(&self.slug(t)))
        {
            return Err(format!(
                "The token {} already uses that name. Pick another name.",
                self.css_name(t)
            ));
        }
        Ok(())
    }

    fn checked_effects(&self, effects: Vec<StyleEffect>) -> Result<Vec<StyleEffect>, String> {
        (effects.into_iter())
            .map(|e| {
                let e = e.normalized();
                match e.color_token.as_deref().map(|t| self.get(t)) {
                    Some(Some(t)) if t.kind == TokenKind::Color => Ok(e),
                    Some(_) => Err("A shadow's color takes a color token.".to_string()),
                    None => Ok(e),
                }
            })
            .collect()
    }

    /// Add an effect style and return its id.
    pub fn create_effect_style(
        &mut self,
        name: &str,
        effects: Vec<StyleEffect>,
        description: Option<&str>,
    ) -> Result<String, String> {
        let name = name.trim().trim_start_matches("--").to_string();
        let id = crate::project::new_id();
        self.check_effect_style_name(&id, &name)?;
        check_description(description)?;
        let effects = self.checked_effects(effects)?;
        self.effect_styles.push(EffectStyleDef {
            id: id.clone(),
            name,
            effects,
            description: description.filter(|d| !d.is_empty()).map(String::from),
            system: None,
        });
        Ok(id)
    }

    /// Copy a layer's effects and color aliases, or start with one drop shadow.
    /// Choose an unused name.
    pub fn add_default_effect_style(&mut self, from: Option<&Node>) -> String {
        let base = "Effect style";
        let mut i = 1;
        while self
            .check_effect_style_name("", &numbered(base, i))
            .is_err()
        {
            i += 1;
        }
        // A layer that uses a style holds colors, so the aliases come from the style.
        let styled = from.and_then(|n| self.effect_style(n.effect_style.as_ref()?));
        let effects = match from.filter(|n| !n.style.effects.is_empty()) {
            Some(_) if styled.is_some() => styled.map(|s| s.effects.clone()).unwrap_or_default(),
            Some(n) => (n.style.effects.iter().enumerate())
                .map(|(i, e)| {
                    let token = (n.tokens.get(&TokenProp::EffectColor(i)))
                        .filter(|t| self.get(t).is_some_and(|t| t.kind == TokenKind::Color));
                    StyleEffect {
                        effect: *e,
                        color_token: token.cloned(),
                    }
                    .normalized()
                })
                .collect(),
            None => vec![StyleEffect::literal(Effect::new(EffectKind::DropShadow))],
        };
        let id = crate::project::new_id();
        self.effect_styles.push(EffectStyleDef {
            id: id.clone(),
            name: numbered(base, i),
            effects,
            description: None,
            system: None,
        });
        id
    }

    /// Change a style's name, effects or description. An empty description clears it.
    /// Nothing changes unless the whole request is valid.
    pub fn update_effect_style(
        &mut self,
        id: &str,
        name: Option<&str>,
        effects: Option<Vec<StyleEffect>>,
        description: Option<&str>,
    ) -> Result<(), String> {
        let mut s = self
            .effect_style(id)
            .ok_or("Effect style not found")?
            .clone();
        if let Some(why) = self.effect_style_locked(id) {
            return Err(why);
        }
        if let Some(n) = name {
            s.name = n.trim().trim_start_matches("--").to_string();
        }
        self.check_effect_style_name(id, &s.name)?;
        check_description(description)?;
        if let Some(e) = effects {
            s.effects = self.checked_effects(e)?;
        }
        if let Some(d) = description {
            s.description = (!d.is_empty()).then(|| d.to_string());
        }
        let at = self.effect_styles.iter().position(|x| x.id == id).unwrap();
        self.effect_styles[at] = s;
        Ok(())
    }

    /// Remove styles. A design system's styles stay. Layers that used them keep their effects.
    pub fn delete_effect_styles(&mut self, ids: &[String]) {
        self.effect_styles
            .retain(|s| !ids.contains(&s.id) || s.system.is_some());
    }

    /// Flatten removed aliases using the first mode, since styles cannot store per-mode literals.
    fn flattened_effect_style(
        &self,
        s: &EffectStyleDef,
        gone: impl Fn(&str) -> bool,
    ) -> EffectStyleDef {
        let mut s = s.clone();
        for e in s.aliases_mut() {
            let token = e.color_token.as_deref().unwrap_or_default();
            if gone(token) {
                if let Some(TokenValue::Color(c)) = self.resolve(token) {
                    e.effect.color = c;
                }
                e.color_token = None;
            }
        }
        s
    }

    pub(super) fn flatten_effect_style_aliases(&mut self, gone: &HashSet<&str>) {
        self.effect_styles = (self.effect_styles.iter())
            .map(|s| self.flattened_effect_style(s, |t| gone.contains(t)))
            .collect();
    }

    /// Local styles for publishing, with external token aliases flattened to colors.
    pub fn publishable_effect_styles(&self) -> Vec<EffectStyleDef> {
        let elsewhere = |t: &str| self.get(t).is_none_or(|t| t.system.is_some());
        self.effect_styles
            .iter()
            .filter(|s| s.system.is_none())
            .map(|s| self.flattened_effect_style(s, elsewhere))
            .collect()
    }

    pub(super) fn linked_effect_styles(
        system: &str,
        published: &[EffectStyleDef],
    ) -> Vec<EffectStyleDef> {
        published
            .iter()
            .map(|s| {
                let mut s = EffectStyleDef {
                    id: linked_id(system, &s.id),
                    system: Some(system.to_string()),
                    ..s.clone()
                };
                for e in s.aliases_mut() {
                    e.color_token = e.color_token.as_deref().map(|t| linked_id(system, t));
                }
                s
            })
            .collect()
    }

    pub(super) fn effect_style_clashes(
        &self,
        prefix: &str,
        published: &[EffectStyleDef],
    ) -> Vec<String> {
        published
            .iter()
            .map(|s| format!("{prefix}-{}", style_slug(&s.name)))
            .filter(|slug| {
                var_slugs(slug).iter().any(|v| {
                    self.effect_var_taken(v, "").is_some() || self.by_css_name(v).is_some()
                })
            })
            .map(|slug| format!("--{slug}"))
            .collect()
    }

    /// Replace a system's linked styles. Return removed styles as id and variable pairs.
    pub(super) fn replace_linked_effect_styles(
        &mut self,
        system: &str,
        new: Vec<EffectStyleDef>,
    ) -> Vec<(String, String)> {
        let ours = |s: &EffectStyleDef| s.system.as_deref() == Some(system);
        let gone = replace_linked(&mut self.effect_styles, ours, new);
        (gone.iter())
            .map(|s| (s.id.clone(), self.effect_style_var(s)))
            .collect()
    }

    /// Turn a system's styles into local ones that keep their variables.
    pub(super) fn detach_effect_styles(&mut self, system: &str) {
        let slugs: Vec<(String, String)> = self
            .effect_styles
            .iter()
            .filter(|s| s.system.as_deref() == Some(system))
            .map(|s| (s.id.clone(), self.effect_style_slug(s)))
            .collect();
        for s in &mut self.effect_styles {
            if let Some((_, slug)) = slugs.iter().find(|(id, _)| *id == s.id) {
                s.name = slug.clone();
                s.system = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;

    fn shadow(y: f64, blur: f64) -> Effect {
        Effect {
            y,
            blur,
            ..Effect::new(EffectKind::DropShadow)
        }
    }

    fn elevation() -> (TokenSet, String, String) {
        let mut s = TokenSet::default();
        let dark = vec![("dark".to_string(), "#ffffff".to_string())];
        let token = s
            .create_in_modes(TokenKind::Color, "shade", "#000000", &dark, None)
            .unwrap();
        let effects = vec![
            StyleEffect {
                effect: shadow(1.0, 2.0),
                color_token: Some(token.clone()),
            },
            StyleEffect::literal(Effect::new(EffectKind::LayerBlur)),
        ];
        let id = s.create_effect_style("Elevation/2", effects, None).unwrap();
        (s, token, id)
    }

    #[test]
    fn names_stay_clear_of_tokens_and_other_styles() {
        let (mut s, _, id) = elevation();
        assert!(s.create_effect_style("elevation 2", vec![], None).is_err());
        assert!(
            s.create_effect_style("Elevation/2 blur", vec![], None)
                .is_err()
        );
        assert!(s.create_effect_style("shade", vec![], None).is_err());
        let token = s.create(TokenKind::Color, "elevation-2", "#fff", None);
        assert!(token.unwrap_err().contains("effect style"));
        s.update_effect_style(&id, Some("Raised"), None, None)
            .unwrap();
        s.create(TokenKind::Color, "elevation-2", "#fff", None)
            .unwrap();
    }

    #[test]
    fn an_alias_takes_a_color_token() {
        let (mut s, _, id) = elevation();
        let size = s.create(TokenKind::FontSize, "size", "12px", None).unwrap();
        let bad = vec![StyleEffect {
            effect: shadow(0.0, 0.0),
            color_token: Some(size),
        }];
        let with_size = s.clone();
        assert!(
            s.update_effect_style(&id, Some("Other"), Some(bad), None)
                .is_err()
        );
        assert_eq!(s, with_size, "nothing changes, the name included");
    }

    #[test]
    fn deleting_a_token_flattens_the_alias() {
        let (mut s, token, id) = elevation();
        s.delete(&[token]);
        let e = &s.effect_style(&id).unwrap().effects[0];
        assert_eq!(
            (e.color_token.as_ref(), e.effect.color),
            (None, Color::BLACK)
        );
    }

    #[test]
    fn new_styles_copy_a_layers_effects_and_tokens() {
        let (mut s, token, _) = elevation();
        let mut n = Node::new(
            crate::model::NodeId(1),
            crate::model::NodeKind::Rect,
            crate::geom::Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        n.style.effects = vec![shadow(4.0, 8.0), shadow(1.0, 1.0)];
        n.tokens.insert(TokenProp::EffectColor(1), token.clone());
        let id = s.add_default_effect_style(Some(&n));
        let st = s.effect_style(&id).unwrap();
        assert_eq!(st.effects[0], StyleEffect::literal(shadow(4.0, 8.0)));
        assert_eq!(st.effects[1].color_token, Some(token));
        n.tokens.clear();
        n.style.effects = s.resolve_effect_style_in(&id, None).unwrap();
        n.effect_style = Some(id.clone());
        let copy = s.add_default_effect_style(Some(&n));
        assert_eq!(
            s.effect_style(&copy).unwrap().effects,
            s.effect_style(&id).unwrap().effects
        );
    }
}
