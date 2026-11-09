//! Text styles: named sets of text settings, each field a literal or a token alias. They live
//! in the [`TokenSet`] beside the tokens, so undo, saving and design systems treat them alike.
//!
//! Layers store resolved style values. Local edits override individual fields; the editor
//! keeps the remaining fields in sync with the style.

use super::node::{Node, NodeKind};
use super::paint::LetterSpacing;
use super::tokens::{Keyed, TokenKind, TokenProp, TokenSet, TokenValue, css_slug, linked_id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StyleField {
    Family,
    Size,
    Weight,
    Italic,
    LineHeight,
    LetterSpacing,
}

impl StyleField {
    pub const ALL: [StyleField; 6] = [
        StyleField::Family,
        StyleField::Size,
        StyleField::Weight,
        StyleField::Italic,
        StyleField::LineHeight,
        StyleField::LetterSpacing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StyleField::Family => "family",
            StyleField::Size => "size",
            StyleField::Weight => "weight",
            StyleField::Italic => "italic",
            StyleField::LineHeight => "line height",
            StyleField::LetterSpacing => "letter spacing",
        }
    }

    /// The token property for the same layer field. Italic has none.
    pub fn prop(self) -> Option<TokenProp> {
        Some(match self {
            StyleField::Family => TokenProp::FontFamily,
            StyleField::Size => TokenProp::FontSize,
            StyleField::Weight => TokenProp::FontWeight,
            StyleField::Italic => return None,
            StyleField::LineHeight => TokenProp::LineHeight,
            StyleField::LetterSpacing => TokenProp::LetterSpacing,
        })
    }

    /// The camelCase CSS property, as the MCP tools take it.
    pub fn css_key(self) -> &'static str {
        match self {
            StyleField::Family => "fontFamily",
            StyleField::Size => "fontSize",
            StyleField::Weight => "fontWeight",
            StyleField::Italic => "fontStyle",
            StyleField::LineHeight => "lineHeight",
            StyleField::LetterSpacing => "letterSpacing",
        }
    }
}

/// A named text style. Field values are literals or `TokenValue::Alias` to a token of the
/// field's kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyleDef {
    pub id: String,
    /// What the user typed, such as `Heading/L`. See [`style_slug`] for the class it becomes.
    pub name: String,
    pub family: TokenValue,
    pub size: TokenValue,
    pub weight: TokenValue,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// None is the font's own line height (CSS `normal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<TokenValue>,
    pub letter_spacing: TokenValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The design system this style comes from, by id, like `Token::system`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

impl Keyed for TextStyleDef {
    fn key(&self) -> &str {
        &self.id
    }
}

impl TextStyleDef {
    /// The token-like fields with the kind each takes. Italic isn't one.
    pub fn values_mut(&mut self) -> impl Iterator<Item = (TokenKind, &mut TokenValue)> {
        [
            (TokenKind::FontFamily, Some(&mut self.family)),
            (TokenKind::FontSize, Some(&mut self.size)),
            (TokenKind::FontWeight, Some(&mut self.weight)),
            (TokenKind::LineHeight, self.line_height.as_mut()),
            (TokenKind::LetterSpacing, Some(&mut self.letter_spacing)),
        ]
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
    }

    fn values(&self) -> impl Iterator<Item = &TokenValue> {
        [
            Some(&self.family),
            Some(&self.size),
            Some(&self.weight),
            self.line_height.as_ref(),
            Some(&self.letter_spacing),
        ]
        .into_iter()
        .flatten()
    }

    pub fn uses_any(&self, ids: &HashSet<&str>) -> bool {
        self.values()
            .any(|v| matches!(v, TokenValue::Alias(a) if ids.contains(a.as_str())))
    }
}

/// The style a text layer uses, and the fields it sets itself.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextStyleRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub overrides: BTreeSet<StyleField>,
}

/// A style with every alias followed to a literal.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    pub family: String,
    pub size: f64,
    pub weight: f64,
    pub italic: bool,
    pub line_height: Option<f64>,
    pub letter_spacing: LetterSpacing,
}

impl ResolvedStyle {
    pub fn holds(&self, f: StyleField, n: &Node) -> bool {
        let ts = &n.text_style;
        match f {
            StyleField::Family => ts.family.as_deref().unwrap_or("Inter") == self.family,
            StyleField::Size => n.kind.font_size() == Some(self.size),
            StyleField::Weight => ts.base_weight() == self.weight as u16,
            StyleField::Italic => ts.italic == self.italic,
            StyleField::LineHeight => ts.line_height == self.line_height,
            StyleField::LetterSpacing => ts.letter_spacing == self.letter_spacing,
        }
    }

    pub fn write(&self, f: StyleField, n: &mut Node) {
        let ts = &mut n.text_style;
        match f {
            StyleField::Family => ts.family = Some(self.family.clone()),
            StyleField::Size => {
                if let NodeKind::Text { font_size, .. } = &mut n.kind {
                    *font_size = self.size;
                }
            }
            StyleField::Weight => ts.set_weight(self.weight as u16),
            StyleField::Italic => ts.italic = self.italic,
            StyleField::LineHeight => ts.line_height = self.line_height,
            StyleField::LetterSpacing => ts.letter_spacing = self.letter_spacing,
        }
    }

    /// CSS for `css::apply`, at full precision to preserve exact field comparisons.
    pub fn literal_css(&self) -> Vec<(String, String)> {
        let spacing = match self.letter_spacing {
            LetterSpacing::Px(v) => format!("{v}px"),
            LetterSpacing::Em { em } => format!("{em}em"),
        };
        vec![
            ("fontFamily".into(), self.family.clone()),
            ("fontSize".into(), format!("{}px", self.size)),
            ("fontWeight".into(), self.weight.to_string()),
            (
                "fontStyle".into(),
                if self.italic { "italic" } else { "normal" }.into(),
            ),
            (
                "lineHeight".into(),
                self.line_height.map_or("normal".into(), |l| l.to_string()),
            ),
            ("letterSpacing".into(), spacing),
        ]
    }
}

/// Raw field input. None preserves an existing value or uses the default for a new style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleInput {
    pub family: Option<String>,
    pub size: Option<String>,
    pub weight: Option<String>,
    /// `italic` or `normal`.
    pub italic: Option<String>,
    /// A line height, or `normal` for the font's own.
    pub line_height: Option<String>,
    pub letter_spacing: Option<String>,
}

impl StyleInput {
    pub fn set(&mut self, f: StyleField, raw: String) {
        let slot = match f {
            StyleField::Family => &mut self.family,
            StyleField::Size => &mut self.size,
            StyleField::Weight => &mut self.weight,
            StyleField::Italic => &mut self.italic,
            StyleField::LineHeight => &mut self.line_height,
            StyleField::LetterSpacing => &mut self.letter_spacing,
        };
        *slot = Some(raw);
    }
}

/// A style name as a CSS class, without the dot: `Heading/L` becomes `heading-l`.
pub fn style_slug(name: &str) -> String {
    css_slug(&name.replace('/', " "))
}

fn numbered(base: &str, i: usize) -> String {
    if i < 2 {
        base.to_string()
    } else {
        format!("{base} {i}")
    }
}

impl TokenSet {
    /// Every text style, local ones first in creation order.
    pub fn styles(&self) -> &[TextStyleDef] {
        &self.styles
    }

    pub fn style(&self, id: &str) -> Option<&TextStyleDef> {
        self.styles.iter().find(|s| s.id == id)
    }

    /// A token set as a project stores it, with its text styles.
    pub fn with_styles(mut self, styles: Vec<TextStyleDef>) -> TokenSet {
        self.styles = styles;
        self
    }

    /// The CSS class of `s`, without the dot. A design system's styles start with its prefix.
    pub fn style_class(&self, s: &TextStyleDef) -> String {
        match s.system.as_deref().and_then(|id| self.system(id)) {
            Some(sys) => format!("{}-{}", sys.prefix, style_slug(&s.name)),
            None => style_slug(&s.name),
        }
    }

    /// Find a style by class (`heading-l` or `.heading-l`) or by name (`Heading/L`).
    pub fn style_by_name(&self, name: &str) -> Option<&TextStyleDef> {
        let name = name.trim();
        let want = name.strip_prefix('.').unwrap_or(name);
        self.styles
            .iter()
            .find(|s| self.style_class(s) == want)
            .or_else(|| self.styles.iter().find(|s| s.name == name))
    }

    /// Why a style can't be edited here: it belongs to a design system.
    pub fn style_locked(&self, id: &str) -> Option<String> {
        let s = self.style(id)?;
        let sys = self.system(s.system.as_deref()?)?;
        Some(format!(
            "\".{}\" belongs to the {} design system, so it can't be changed in this file. \
             Detach the system to edit it.",
            self.style_class(s),
            sys.name
        ))
    }

    /// Follow every alias to a literal. None when an alias is broken.
    pub fn resolve_style(&self, id: &str) -> Option<ResolvedStyle> {
        let s = self.style(id)?;
        let num = |v: &TokenValue| match self.literal(v)? {
            TokenValue::Number(n) => Some(n),
            _ => None,
        };
        Some(ResolvedStyle {
            family: match self.literal(&s.family)? {
                TokenValue::Text(f) => f,
                _ => return None,
            },
            size: num(&s.size)?,
            weight: num(&s.weight)?,
            italic: s.italic,
            line_height: match &s.line_height {
                Some(v) => Some(num(v)?),
                None => None,
            },
            letter_spacing: match self.literal(&s.letter_spacing)? {
                TokenValue::Number(px) => LetterSpacing::Px(px),
                TokenValue::Em(em) => LetterSpacing::Em { em },
                _ => return None,
            },
        })
    }

    /// Expand `textStyle` into values and a [`css::STYLE_MARKER`] binding. Later fields win;
    /// unknown names stay for `css::apply` to report as ignored.
    pub fn expand_styles(&self, styles: &mut Vec<(String, String)>) {
        if !styles.iter().any(|(k, _)| k == "textStyle") {
            return;
        }
        let mut out = Vec::with_capacity(styles.len() + 7);
        for (k, v) in styles.drain(..) {
            let found = (k == "textStyle")
                .then(|| self.style_by_name(&v))
                .flatten()
                .and_then(|s| Some((s.id.clone(), self.resolve_style(&s.id)?)));
            match found {
                Some((id, r)) => {
                    out.extend(r.literal_css());
                    out.push((crate::mcp::css::STYLE_MARKER.into(), id));
                }
                None => out.push((k, v)),
            }
        }
        *styles = out;
    }

    fn literal(&self, v: &TokenValue) -> Option<TokenValue> {
        match v {
            TokenValue::Alias(a) => self.resolve(a),
            v => Some(v.clone()),
        }
    }

    /// One field's value as CSS: `var(--token)` for an alias.
    pub fn style_value_css(&self, s: &TextStyleDef, f: StyleField) -> String {
        let field = |kind, v: &TokenValue| self.value_css(kind, v);
        match f {
            StyleField::Family => field(TokenKind::FontFamily, &s.family),
            StyleField::Size => field(TokenKind::FontSize, &s.size),
            StyleField::Weight => field(TokenKind::FontWeight, &s.weight),
            StyleField::Italic => if s.italic { "italic" } else { "normal" }.into(),
            StyleField::LineHeight => match &s.line_height {
                Some(v) => field(TokenKind::LineHeight, v),
                None => "normal".into(),
            },
            StyleField::LetterSpacing => field(TokenKind::LetterSpacing, &s.letter_spacing),
        }
    }

    /// The style as one line of CSS: `font: 700 32px/120% Inter; letter-spacing: -0.02em`.
    pub fn style_short(&self, s: &TextStyleDef) -> String {
        let v = |f| self.style_value_css(s, f);
        let mut font = String::new();
        if s.italic {
            font.push_str("italic ");
        }
        font.push_str(&format!(
            "{} {}",
            v(StyleField::Weight),
            v(StyleField::Size)
        ));
        if s.line_height.is_some() {
            font.push_str(&format!("/{}", v(StyleField::LineHeight)));
        }
        font.push_str(&format!(" {}", v(StyleField::Family)));
        let mut out = format!("font: {font}");
        if s.letter_spacing != TokenValue::Number(0.0) {
            out.push_str(&format!(
                "; letter-spacing: {}",
                v(StyleField::LetterSpacing)
            ));
        }
        out
    }

    /// One class per style, `.heading-l { ... }`, or Tailwind v4 `@utility` blocks.
    pub fn style_sheet(&self, styles: &[&TextStyleDef], tailwind: bool) -> String {
        let mut out = String::new();
        for s in styles {
            let class = self.style_class(s);
            if tailwind {
                out.push_str(&format!("@utility {class} {{\n"));
            } else {
                out.push_str(&format!(".{} {{\n", selector_escape(&class)));
            }
            for f in StyleField::ALL {
                let key = css_kebab(f.css_key());
                out.push_str(&format!("  {key}: {};\n", self.style_value_css(s, f)));
            }
            out.push_str("}\n");
        }
        out
    }

    /// Parse one field for a style. Aliases must name a token of the field's kind.
    fn parse_style_field(&self, f: StyleField, raw: &str) -> Result<Option<TokenValue>, String> {
        let s = raw.trim();
        let kind = match f {
            StyleField::Italic => return Err("italic is not a token field".into()),
            StyleField::LineHeight if s == "normal" || s == "auto" => return Ok(None),
            StyleField::Family => TokenKind::FontFamily,
            StyleField::Size => TokenKind::FontSize,
            StyleField::Weight => TokenKind::FontWeight,
            StyleField::LineHeight => TokenKind::LineHeight,
            StyleField::LetterSpacing => TokenKind::LetterSpacing,
        };
        let s = if f == StyleField::LetterSpacing && s == "normal" {
            "0px"
        } else {
            s
        };
        self.parse_value(kind, s).map(Some).map_err(|e| {
            if super::tokens::var_name(s).is_some() {
                format!(
                    "{e} A text style's {} takes a {} token.",
                    f.label(),
                    kind.label().to_lowercase()
                )
            } else {
                e
            }
        })
    }

    /// `base` with `input` applied. Nothing is kept unless every field parses.
    fn styled(&self, base: &TextStyleDef, input: &StyleInput) -> Result<TextStyleDef, String> {
        let mut s = base.clone();
        let mut one = |f: StyleField, raw: &Option<String>| -> Result<(), String> {
            let Some(raw) = raw else { return Ok(()) };
            let v = self.parse_style_field(f, raw)?;
            match f {
                StyleField::Family => s.family = v.unwrap_or(s.family.clone()),
                StyleField::Size => s.size = v.unwrap_or(s.size.clone()),
                StyleField::Weight => s.weight = v.unwrap_or(s.weight.clone()),
                StyleField::LineHeight => s.line_height = v,
                StyleField::LetterSpacing => {
                    s.letter_spacing = v.unwrap_or(s.letter_spacing.clone())
                }
                StyleField::Italic => {}
            }
            Ok(())
        };
        one(StyleField::Family, &input.family)?;
        one(StyleField::Size, &input.size)?;
        one(StyleField::Weight, &input.weight)?;
        one(StyleField::LineHeight, &input.line_height)?;
        one(StyleField::LetterSpacing, &input.letter_spacing)?;
        if let Some(i) = &input.italic {
            s.italic = match i.trim().to_ascii_lowercase().as_str() {
                "italic" | "oblique" | "true" => true,
                "normal" | "false" => false,
                _ => return Err(format!("Invalid font style \"{i}\". Use italic or normal.")),
            };
        }
        Ok(s)
    }

    fn check_style_name(&self, id: &str, name: &str) -> Result<(), String> {
        let slug = style_slug(name);
        if slug.is_empty() {
            return Err(format!(
                "Text style name \"{name}\" needs a letter or number to work as a CSS class."
            ));
        }
        if let Some(other) = self
            .styles
            .iter()
            .find(|s| s.id != id && self.style_class(s) == slug)
        {
            return Err(format!(
                "The text style \"{}\" already uses the class .{slug}. Pick another name.",
                other.name
            ));
        }
        Ok(())
    }

    fn default_style(id: String, name: String) -> TextStyleDef {
        TextStyleDef {
            id,
            name,
            family: TokenValue::Text("Inter".into()),
            size: TokenValue::Number(16.0),
            weight: TokenValue::Number(400.0),
            italic: false,
            line_height: None,
            letter_spacing: TokenValue::Number(0.0),
            description: None,
            system: None,
        }
    }

    /// Add a text style and return its id. Missing fields take the defaults. Asking for a
    /// style that already exists with the same name and fields returns that one.
    pub fn create_style(
        &mut self,
        name: &str,
        input: &StyleInput,
        description: Option<&str>,
    ) -> Result<String, String> {
        let name = name.trim().trim_start_matches('.').to_string();
        let id = crate::project::new_id();
        let mut s = self.styled(&Self::default_style(id.clone(), name.clone()), input)?;
        if let Some(same) = self.styles.iter().find(|x| {
            x.system.is_none()
                && TextStyleDef {
                    id: x.id.clone(),
                    ..s.clone()
                } == **x
        }) {
            return Ok(same.id.clone());
        }
        self.check_style_name(&id, &name)?;
        super::tokens::check_description(description)?;
        s.description = description.filter(|d| !d.is_empty()).map(String::from);
        self.styles.push(s);
        Ok(id)
    }

    /// A new style that copies a text layer's settings, or the defaults, with an unused name.
    pub fn add_default_style(&mut self, from: Option<&Node>) -> String {
        let base = "Text style";
        let mut i = 1;
        while self.check_style_name("", &numbered(base, i)).is_err() {
            i += 1;
        }
        let id = crate::project::new_id();
        let mut s = Self::default_style(id.clone(), numbered(base, i));
        if let Some(n) = from
            && let Some(size) = n.kind.font_size()
        {
            let ts = &n.text_style;
            s.family = TokenValue::Text(ts.family.clone().unwrap_or_else(|| "Inter".into()));
            s.size = TokenValue::Number(size);
            s.weight = TokenValue::Number(ts.base_weight() as f64);
            s.italic = ts.italic;
            s.line_height = ts.line_height.map(TokenValue::Number);
            s.letter_spacing = match ts.letter_spacing {
                LetterSpacing::Px(px) => TokenValue::Number(px),
                LetterSpacing::Em { em } => TokenValue::Em(em),
            };
        }
        self.styles.push(s);
        id
    }

    /// Change a style's name, fields or description. An empty description clears it.
    /// Nothing changes unless the whole request is valid.
    pub fn update_style(
        &mut self,
        id: &str,
        name: Option<&str>,
        input: &StyleInput,
        description: Option<&str>,
    ) -> Result<(), String> {
        let old = self.style(id).ok_or("Text style not found")?;
        if let Some(why) = self.style_locked(id) {
            return Err(why);
        }
        let mut s = self.styled(old, input)?;
        if let Some(n) = name {
            s.name = n.trim().trim_start_matches('.').to_string();
        }
        self.check_style_name(id, &s.name)?;
        super::tokens::check_description(description)?;
        if let Some(d) = description {
            s.description = (!d.is_empty()).then(|| d.to_string());
        }
        let at = self.styles.iter().position(|x| x.id == id).unwrap();
        self.styles[at] = s;
        Ok(())
    }

    /// Remove styles. A design system's styles stay. Layers that used them keep their values.
    pub fn delete_styles(&mut self, ids: &[String]) {
        self.styles
            .retain(|s| !ids.contains(&s.id) || s.system.is_some());
    }

    /// Before tokens go: style fields that alias them take on the values they showed.
    pub(super) fn flatten_style_aliases(&mut self, gone: &HashSet<&str>) {
        let mut styles = std::mem::take(&mut self.styles);
        for s in &mut styles {
            for (kind, v) in s.values_mut() {
                if let TokenValue::Alias(a) = v
                    && gone.contains(a.as_str())
                {
                    *v = self.resolve(a).unwrap_or_else(|| kind.default_value());
                }
            }
        }
        self.styles = styles;
    }

    /// Styles as a design system publishes them: the local ones, with aliases to another
    /// system's tokens replaced by the values they show.
    pub fn publishable_styles(&self) -> Vec<TextStyleDef> {
        self.styles
            .iter()
            .filter(|s| s.system.is_none())
            .map(|s| {
                let mut s = s.clone();
                for (kind, v) in s.values_mut() {
                    if let TokenValue::Alias(a) = v
                        && self.get(a).is_none_or(|t| t.system.is_some())
                    {
                        *v = self.resolve(a).unwrap_or_else(|| kind.default_value());
                    }
                }
                s
            })
            .collect()
    }

    /// `published` as this project's read-only copies, aliasing the system's linked tokens.
    pub(super) fn linked_styles(system: &str, published: &[TextStyleDef]) -> Vec<TextStyleDef> {
        published
            .iter()
            .map(|s| {
                let mut s = TextStyleDef {
                    id: linked_id(system, &s.id),
                    system: Some(system.to_string()),
                    ..s.clone()
                };
                for (_, v) in s.values_mut() {
                    if let TokenValue::Alias(a) = v {
                        *a = linked_id(system, a);
                    }
                }
                s
            })
            .collect()
    }

    /// Classes of local styles that a system with `prefix` would also use.
    pub(super) fn style_clashes(&self, prefix: &str, published: &[TextStyleDef]) -> Vec<String> {
        let taken: HashSet<String> = self.styles.iter().map(|s| self.style_class(s)).collect();
        published
            .iter()
            .map(|s| format!("{prefix}-{}", style_slug(&s.name)))
            .filter(|c| taken.contains(c))
            .map(|c| format!(".{c}"))
            .collect()
    }

    /// Swap a system's styles for `new` (already linked). Returns the dropped ones as
    /// (id, `.class`).
    pub(super) fn replace_linked_styles(
        &mut self,
        system: &str,
        new: Vec<TextStyleDef>,
    ) -> Vec<(String, String)> {
        let ours = |s: &TextStyleDef| s.system.as_deref() == Some(system);
        let gone: Vec<(String, String)> = self
            .styles
            .iter()
            .filter(|s| ours(s) && !new.iter().any(|n| n.id == s.id))
            .map(|s| (s.id.clone(), format!(".{}", self.style_class(s))))
            .collect();
        self.styles
            .retain(|s| !gone.iter().any(|(id, _)| *id == s.id));
        for s in &mut self.styles {
            if let Some(n) = new.iter().find(|n| n.id == s.id) {
                *s = n.clone();
            }
        }
        let fresh: Vec<TextStyleDef> = new
            .into_iter()
            .filter(|n| self.style(&n.id).is_none())
            .collect();
        self.styles.extend(fresh);
        gone
    }

    /// Turn a system's styles into local ones that keep their classes.
    pub(super) fn detach_styles(&mut self, system: &str) {
        let classes: Vec<(String, String)> = self
            .styles
            .iter()
            .filter(|s| s.system.as_deref() == Some(system))
            .map(|s| (s.id.clone(), self.style_class(s)))
            .collect();
        for s in &mut self.styles {
            if let Some((_, class)) = classes.iter().find(|(id, _)| *id == s.id) {
                s.name = class.clone();
                s.system = None;
            }
        }
    }
}

fn css_kebab(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A class as a CSS selector. A leading digit must be escaped (`24px` is `\32 4px`).
fn selector_escape(class: &str) -> String {
    match class.chars().next() {
        Some(c) if c.is_ascii_digit() => format!("\\3{c} {}", &class[1..]),
        _ => class.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_starting_with_a_digit_are_escaped_in_css() {
        let mut s = TokenSet::default();
        let id = s
            .create_style("24px Semibold", &StyleInput::default(), None)
            .unwrap();
        let st = s.style(&id).unwrap();
        assert_eq!(s.style_class(st), "24px-semibold");
        assert!(
            s.style_sheet(&[st], false)
                .starts_with(".\\32 4px-semibold {")
        );
    }

    fn input(pairs: &[(StyleField, &str)]) -> StyleInput {
        let mut i = StyleInput::default();
        for (f, v) in pairs {
            i.set(*f, v.to_string());
        }
        i
    }

    fn heading() -> (TokenSet, String) {
        let mut s = TokenSet::default();
        s.create(TokenKind::FontFamily, "font-display", "Lora", None)
            .unwrap();
        s.create(TokenKind::FontSize, "size-l", "32px", None)
            .unwrap();
        let id = s
            .create_style(
                "Heading/L",
                &input(&[
                    (StyleField::Family, "var(--font-display)"),
                    (StyleField::Size, "var(--size-l)"),
                    (StyleField::Weight, "700"),
                    (StyleField::LineHeight, "120%"),
                    (StyleField::LetterSpacing, "-0.02em"),
                ]),
                None,
            )
            .unwrap();
        (s, id)
    }

    #[test]
    fn bad_fields_change_nothing() {
        let (mut s, id) = heading();
        let before = s.clone();
        let err = s.update_style(
            &id,
            Some("Title"),
            &input(&[
                (StyleField::Family, "Arial"),
                (StyleField::Size, "var(--font-display)"),
            ]),
            None,
        );
        assert!(err.is_err());
        assert_eq!(s, before);
    }

    #[test]
    fn classes_are_unique_and_create_is_idempotent() {
        let (mut s, id) = heading();
        let same = s
            .create_style(
                "Heading/L",
                &input(&[
                    (StyleField::Family, "var(--font-display)"),
                    (StyleField::Size, "var(--size-l)"),
                    (StyleField::Weight, "700"),
                    (StyleField::LineHeight, "120%"),
                    (StyleField::LetterSpacing, "-0.02em"),
                ]),
                None,
            )
            .unwrap();
        assert_eq!(same, id);
        assert!(
            s.create_style("heading l", &StyleInput::default(), None)
                .is_err()
        );
    }

    #[test]
    fn deleting_a_token_flattens_style_aliases() {
        let (mut s, id) = heading();
        let size = s.by_css_name("--size-l").unwrap().id.clone();
        s.delete(&[size]);
        assert_eq!(s.style(&id).unwrap().size, TokenValue::Number(32.0));
        assert!(s.resolve_style(&id).is_some());
    }

    #[test]
    fn new_styles_copy_a_text_layer() {
        let mut n = Node::new(
            crate::model::NodeId(1),
            NodeKind::Text {
                text: "Hi".into(),
                font_size: 20.0,
                auto_width: true,
                align: Default::default(),
            },
            crate::geom::Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        n.text_style.set_weight(600);
        let mut s = TokenSet::default();
        let a = s.add_default_style(Some(&n));
        let r = s.resolve_style(&a).unwrap();
        assert_eq!((r.size, r.weight), (20.0, 600.0));
    }
}
