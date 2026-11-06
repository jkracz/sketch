//! Design tokens: named values shared by every page of a project.
//!
//! Behaviour follows the notes in `analysis/design-tokens.md`. A node property bound to a token
//! keeps the resolved value in its usual field, so renderers and exporters never look at
//! tokens. `Node::tokens` only records which token owns the field, and one rule keeps the two
//! honest: a bound field always holds its token's value. The editor rewrites fields when a
//! token changes, and drops a binding when a field stops matching (see `editor/tokens.rs`).

use super::node::{Color, Node, NodeKind};
use super::paint::Paint;
use crate::mcp::css;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Limit on `Token::description`, in characters.
pub const MAX_DESCRIPTION: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenKind {
    Color,
    FontFamily,
    FontSize,
    FontWeight,
    LetterSpacing,
    LineHeight,
    Opacity,
    Breakpoint,
    Container,
    Spacing,
    Radius,
}

impl TokenKind {
    /// Listing order: grouped by kind, then in creation order within a kind.
    pub const ALL: [TokenKind; 11] = [
        TokenKind::Color,
        TokenKind::FontFamily,
        TokenKind::FontSize,
        TokenKind::FontWeight,
        TokenKind::LetterSpacing,
        TokenKind::LineHeight,
        TokenKind::Opacity,
        TokenKind::Breakpoint,
        TokenKind::Container,
        TokenKind::Spacing,
        TokenKind::Radius,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TokenKind::Color => "Color",
            TokenKind::FontFamily => "Font family",
            TokenKind::FontSize => "Font size",
            TokenKind::FontWeight => "Font weight",
            TokenKind::LetterSpacing => "Letter spacing",
            TokenKind::LineHeight => "Line height",
            TokenKind::Opacity => "Opacity",
            TokenKind::Breakpoint => "Breakpoint",
            TokenKind::Container => "Container",
            TokenKind::Spacing => "Spacing",
            TokenKind::Radius => "Radius",
        }
    }

    /// The name used by the MCP tools.
    pub fn key(self) -> &'static str {
        match self {
            TokenKind::Color => "color",
            TokenKind::FontFamily => "fontFamily",
            TokenKind::FontSize => "fontSize",
            TokenKind::FontWeight => "fontWeight",
            TokenKind::LetterSpacing => "letterSpacing",
            TokenKind::LineHeight => "lineHeight",
            TokenKind::Opacity => "opacity",
            TokenKind::Breakpoint => "breakpoint",
            TokenKind::Container => "container",
            TokenKind::Spacing => "spacing",
            TokenKind::Radius => "radius",
        }
    }

    pub fn from_key(s: &str) -> Option<TokenKind> {
        TokenKind::ALL.into_iter().find(|k| k.key() == s)
    }

    /// The value a token of this kind starts with in the Tokens panel.
    pub fn default_value(self) -> TokenValue {
        match self {
            TokenKind::Color => TokenValue::Color(Color::BLACK),
            TokenKind::FontFamily => TokenValue::Text("Inter".into()),
            TokenKind::FontSize => TokenValue::Number(16.0),
            TokenKind::FontWeight => TokenValue::Number(400.0),
            TokenKind::LetterSpacing => TokenValue::Number(0.0),
            TokenKind::LineHeight => TokenValue::Number(1.5),
            TokenKind::Opacity => TokenValue::Number(1.0),
            TokenKind::Breakpoint => TokenValue::Number(768.0),
            TokenKind::Container => TokenValue::Number(1200.0),
            TokenKind::Spacing => TokenValue::Number(8.0),
            TokenKind::Radius => TokenValue::Number(8.0),
        }
    }
}

/// What a token holds. Numbers are px, except line height (a multiple of the font size),
/// opacity (0 to 1) and font weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenValue {
    Color(Color),
    Number(f64),
    Text(String),
    /// Another token of the same kind, by id.
    Alias(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    /// What the user typed. See [`css_slug`] for the CSS variable it becomes.
    pub name: String,
    #[serde(rename = "type")]
    pub kind: TokenKind,
    pub value: TokenValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `name` as a CSS custom property name, without the leading dashes.
pub fn css_slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        match c {
            '&' => s.push_str("-and-"),
            '.' | '\'' | '"' | '–' | '—' | '‘' | '’' | '“' | '”' => s.push('-'),
            c if c.is_whitespace() => s.push('-'),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                s.push(c.to_ascii_lowercase())
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for c in s.chars() {
        if c != '-' || !out.ends_with('-') {
            out.push(c);
        }
    }
    out.trim_matches('-').to_string()
}

/// `base copy`, then `base copy 2`, `base copy 3`...
fn copy_name(name: &str) -> String {
    if let Some(base) = name.strip_suffix(" copy") {
        return format!("{base} copy 2");
    }
    if let Some((base, n)) = name.rsplit_once(" copy ")
        && let Ok(n) = n.parse::<u32>()
    {
        return format!("{base} copy {}", n + 1);
    }
    format!("{name} copy")
}

fn invalid(value: &str, kind: TokenKind) -> String {
    format!(
        "Invalid value \"{value}\" for token type \"{}\".",
        kind.key()
    )
}

/// Byte index of the `)` that closes the first `(` in `s`.
fn closing_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn var_name(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("var(")?.strip_suffix(')')?;
    Some(inner.split(',').next()?.trim())
}

/// Tokens copied from the Tokens panel. Pasted tokens are independent of their source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenClip {
    pub tokens: Vec<Token>,
    /// Resolved values of alias targets outside the copied set, keyed by token id.
    resolved: HashMap<String, TokenValue>,
    /// Name and kind of those targets, to find the same token in another project.
    targets: HashMap<String, (TokenKind, String)>,
}

/// One token's change, for undo. `Some((index, token))` is its place in the list on that side.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenEdit {
    id: String,
    before: Option<(usize, Token)>,
    after: Option<(usize, Token)>,
}

impl TokenEdit {
    /// Whether the token itself differs. A token that only moved because its neighbours
    /// changed isn't an edit, and recording it would let undo overwrite newer changes.
    fn changes_token(&self) -> bool {
        self.before.as_ref().map(|(_, t)| t) != self.after.as_ref().map(|(_, t)| t)
    }
}

/// The project's tokens. Vector order is creation order, and duplicates sit after their source.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenSet {
    tokens: Vec<Token>,
}

impl TokenSet {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn get(&self, id: &str) -> Option<&Token> {
        self.tokens.iter().find(|t| t.id == id)
    }

    /// Every token in listing order.
    pub fn sorted(&self) -> Vec<&Token> {
        TokenKind::ALL
            .into_iter()
            .flat_map(|k| self.of_kind(k))
            .collect()
    }

    pub fn of_kind(&self, kind: TokenKind) -> impl Iterator<Item = &Token> {
        self.tokens.iter().filter(move |t| t.kind == kind)
    }

    /// Find a token by CSS variable name, with or without the leading `--`. With duplicate
    /// names the first one in listing order wins.
    pub fn by_css_name(&self, name: &str) -> Option<&Token> {
        let want = name.trim().strip_prefix("--").unwrap_or(name.trim());
        self.sorted()
            .into_iter()
            .find(|t| css_slug(&t.name) == want)
    }

    pub fn css_name(&self, t: &Token) -> String {
        format!("--{}", css_slug(&t.name))
    }

    /// Follow aliases to a literal. None when a link is missing or the chain loops, which
    /// only a hand-edited file can produce.
    pub fn resolve(&self, id: &str) -> Option<TokenValue> {
        let mut seen = HashSet::new();
        let mut cur = id;
        loop {
            if !seen.insert(cur) {
                return None;
            }
            match &self.get(cur)?.value {
                TokenValue::Alias(next) => cur = next,
                v => return Some(v.clone()),
            }
        }
    }

    /// Would pointing token `id` at `target` make it depend on itself?
    fn would_cycle(&self, id: &str, target: &str) -> bool {
        let mut cur = target;
        let mut seen = HashSet::new();
        while seen.insert(cur) {
            if cur == id {
                return true;
            }
            match self.get(cur).map(|t| &t.value) {
                Some(TokenValue::Alias(next)) => cur = next,
                _ => return false,
            }
        }
        true
    }

    /// Parse user or agent input: a CSS value for `kind`, or `var(--other)` for an alias.
    pub fn parse_value(&self, kind: TokenKind, raw: &str) -> Result<TokenValue, String> {
        let bad = || invalid(raw, kind);
        let s = raw.trim();
        if let Some(name) = var_name(s) {
            let t = self
                .by_css_name(name)
                .filter(|t| t.kind == kind)
                .ok_or_else(bad)?;
            return Ok(TokenValue::Alias(t.id.clone()));
        }
        let size = || css::length(s, 16.0).ok_or_else(bad);
        let n = match kind {
            TokenKind::Color => return css::color(s).map(TokenValue::Color).ok_or_else(bad),
            TokenKind::FontFamily => {
                let name = s.trim_matches(['"', '\'']).trim();
                if name.is_empty() {
                    return Err(bad());
                }
                return Ok(TokenValue::Text(name.to_string()));
            }
            TokenKind::FontWeight => match s {
                "normal" => 400.0,
                "bold" => 700.0,
                _ => s
                    .parse::<f64>()
                    .ok()
                    .filter(|w| (1.0..=1000.0).contains(w))
                    .ok_or_else(bad)?,
            },
            TokenKind::LineHeight => css::percent(s)
                .or_else(|| s.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .ok_or_else(bad)?,
            TokenKind::Opacity => css::percent(s)
                .or_else(|| s.parse::<f64>().ok())
                .filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(bad)?,
            // `em` depends on the text it applies to, and a token has no text.
            TokenKind::LetterSpacing if s.ends_with("em") && !s.ends_with("rem") => {
                return Err(bad());
            }
            TokenKind::LetterSpacing => size()?,
            TokenKind::FontSize => Some(size()?).filter(|v| *v > 0.0).ok_or_else(bad)?,
            TokenKind::Breakpoint | TokenKind::Container | TokenKind::Spacing => size()?,
            TokenKind::Radius => Some(size()?).filter(|v| *v >= 0.0).ok_or_else(bad)?,
        };
        // Unit conversion can overflow, and JSON has no infinity: saving one would write
        // null, which stops the project from opening.
        if !n.is_finite() {
            return Err(bad());
        }
        Ok(TokenValue::Number(n))
    }

    fn check(&self, id: &str, name: &str, value: &TokenValue, raw: &str) -> Result<(), String> {
        if css_slug(name).is_empty() {
            return Err(format!(
                "Token name \"{name}\" needs a letter or number to work as a CSS variable."
            ));
        }
        if let TokenValue::Alias(target) = value
            && self.would_cycle(id, target)
        {
            return Err(format!("Circular token reference \"{raw}\"."));
        }
        Ok(())
    }

    /// Add a token and return its id. Asking for a token that already exists with the same
    /// kind, name and value returns that one, so agents can retry safely.
    pub fn create(
        &mut self,
        kind: TokenKind,
        name: &str,
        raw: &str,
        description: Option<&str>,
    ) -> Result<String, String> {
        let name = name.trim().trim_start_matches("--").to_string();
        let value = self.parse_value(kind, raw)?;
        if let Some(t) = self
            .tokens
            .iter()
            .find(|t| t.kind == kind && t.name == name && t.value == value)
        {
            return Ok(t.id.clone());
        }
        let id = crate::project::new_id();
        self.check(&id, &name, &value, raw)?;
        check_description(description)?;
        self.tokens.push(Token {
            id: id.clone(),
            name,
            kind,
            value,
            description: description.filter(|d| !d.is_empty()).map(String::from),
        });
        Ok(id)
    }

    /// Add a token with the kind's default value and an unused name (the Tokens panel's `+`).
    pub fn add_default(&mut self, kind: TokenKind) -> String {
        let base = kind.key().to_string();
        let taken = |n: &str| self.of_kind(kind).any(|t| t.name == n);
        let mut name = base.clone();
        let mut i = 2;
        while taken(&name) {
            name = format!("{base}-{i}");
            i += 1;
        }
        let id = crate::project::new_id();
        self.tokens.push(Token {
            id: id.clone(),
            name,
            kind,
            value: kind.default_value(),
            description: None,
        });
        id
    }

    /// Change a token's name, value or description. An empty description clears it. Nothing
    /// changes unless the whole request is valid.
    pub fn update(
        &mut self,
        id: &str,
        name: Option<&str>,
        raw: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), String> {
        let t = self.get(id).ok_or("Token not found")?;
        let kind = t.kind;
        let name = match name {
            Some(n) => n.trim().trim_start_matches("--").to_string(),
            None => t.name.clone(),
        };
        let value = match raw {
            Some(r) => self.parse_value(kind, r)?,
            None => t.value.clone(),
        };
        self.check(id, &name, &value, raw.unwrap_or_default())?;
        check_description(description)?;
        let t = self.tokens.iter_mut().find(|t| t.id == id).unwrap();
        t.name = name;
        t.value = value;
        if let Some(d) = description {
            t.description = (!d.is_empty()).then(|| d.to_string());
        }
        Ok(())
    }

    /// Set a literal value directly (color pickers and number drags can't produce bad input).
    pub fn set_value(&mut self, id: &str, value: TokenValue) {
        if let Some(t) = self.tokens.iter_mut().find(|t| t.id == id) {
            t.value = value;
        }
    }

    /// Remove tokens. A surviving alias to a removed token takes on the value it showed, so
    /// nothing in the project changes appearance.
    pub fn delete(&mut self, ids: &[String]) {
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let flat: Vec<(usize, TokenValue)> = self
            .tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| !gone.contains(t.id.as_str()))
            .filter(|(_, t)| matches!(&t.value, TokenValue::Alias(a) if gone.contains(a.as_str())))
            .filter_map(|(i, t)| Some((i, self.resolve(&t.id)?)))
            .collect();
        for (i, v) in flat {
            self.tokens[i].value = v;
        }
        self.tokens.retain(|t| !gone.contains(t.id.as_str()));
    }

    /// Copy tokens, each new one right after the last token of its kind in `ids`. Returns the
    /// new ids.
    pub fn duplicate(&mut self, ids: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids {
            let Some(i) = self.tokens.iter().position(|t| t.id == *id) else {
                continue;
            };
            let mut t = self.tokens[i].clone();
            t.id = crate::project::new_id();
            t.name = self.unused_name(t.kind, &t.name);
            out.push(t.id.clone());
            let kind = t.kind;
            let selected =
                |x: &Token| x.kind == kind && (ids.contains(&x.id) || out.contains(&x.id));
            let at = self.tokens.iter().rposition(selected).unwrap_or(i) + 1;
            self.tokens.insert(at, t);
        }
        out
    }

    fn unused_name(&self, kind: TokenKind, name: &str) -> String {
        let mut n = copy_name(name);
        while self.of_kind(kind).any(|t| t.name == n) {
            n = copy_name(&n);
        }
        n
    }

    /// Capture tokens for pasting, here or in another project.
    pub fn copy(&self, ids: &[String]) -> TokenClip {
        let mut clip = TokenClip::default();
        for t in self.sorted() {
            if !ids.contains(&t.id) {
                continue;
            }
            clip.tokens.push(t.clone());
            if let TokenValue::Alias(a) = &t.value
                && !ids.contains(a)
            {
                if let Some(v) = self.resolve(a) {
                    clip.resolved.insert(a.clone(), v);
                }
                if let Some(target) = self.get(a) {
                    clip.targets
                        .insert(a.clone(), (target.kind, target.name.clone()));
                }
            }
        }
        clip
    }

    /// Add copies of `clip`'s tokens and return the new ids. Names that are taken get a copy
    /// suffix. An alias to a token outside the clip finds a token with the same kind and name
    /// here, else becomes the value it had when copied.
    pub fn paste(&mut self, clip: &TokenClip) -> Vec<String> {
        let ids: HashMap<&str, String> = clip
            .tokens
            .iter()
            .map(|t| (t.id.as_str(), crate::project::new_id()))
            .collect();
        let mut out = Vec::new();
        for t in &clip.tokens {
            let value = match &t.value {
                TokenValue::Alias(a) => match ids.get(a.as_str()) {
                    Some(new) => TokenValue::Alias(new.clone()),
                    None => self.pasted_target(clip, a).unwrap_or_else(|| {
                        clip.resolved
                            .get(a)
                            .cloned()
                            .unwrap_or_else(|| t.kind.default_value())
                    }),
                },
                v => v.clone(),
            };
            let taken = self.of_kind(t.kind).any(|x| x.name == t.name);
            let name = if taken {
                self.unused_name(t.kind, &t.name)
            } else {
                t.name.clone()
            };
            self.tokens.push(Token {
                id: ids[t.id.as_str()].clone(),
                name,
                value,
                ..t.clone()
            });
            out.push(ids[t.id.as_str()].clone());
        }
        out
    }

    fn pasted_target(&self, clip: &TokenClip, id: &str) -> Option<TokenValue> {
        if self.get(id).is_some() {
            return Some(TokenValue::Alias(id.to_string()));
        }
        let (kind, name) = clip.targets.get(id)?;
        let t = self.of_kind(*kind).find(|t| t.name == *name)?;
        Some(TokenValue::Alias(t.id.clone()))
    }

    /// A token's value as CSS.
    pub fn css_value(&self, t: &Token) -> String {
        let num = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
        match &t.value {
            TokenValue::Alias(a) => match self.get(a) {
                Some(target) => format!("var({})", self.css_name(target)),
                None => String::new(),
            },
            TokenValue::Color(c) => css::color_css(*c),
            TokenValue::Text(s) if s.contains(' ') => format!("\"{s}\""),
            TokenValue::Text(s) => s.clone(),
            TokenValue::Number(v) => match t.kind {
                TokenKind::LineHeight | TokenKind::Opacity => format!("{}%", num(v * 100.0)),
                TokenKind::FontWeight => num(*v),
                _ => format!("{}px", num(*v)),
            },
        }
    }

    /// A stylesheet of `tokens`: `:root { ... }`, or `@theme { ... }` for Tailwind v4.
    pub fn stylesheet(&self, tokens: &[&Token], tailwind: bool) -> String {
        let mut s = String::from(if tailwind { "@theme {\n" } else { ":root {\n" });
        for t in tokens {
            s.push_str(&format!("  {}: {};\n", self.css_name(t), self.css_value(t)));
        }
        s.push_str("}\n");
        s
    }

    /// Replace `var(--token)` in styles with the token's value. A style whose whole value is
    /// one variable also gets a [`css::TOKEN_MARKER`] entry, which binds the node's property
    /// when `css::apply` reaches it. Unknown variables fall back to their default, if any.
    pub fn expand(&self, styles: &mut Vec<(String, String)>) {
        let mut bound = Vec::new();
        for (k, v) in styles.iter_mut() {
            if let Some(t) = var_name(v).and_then(|n| self.by_css_name(n))
                && let Some(lit) = self.resolve(&t.id)
            {
                bound.push((format!("{}{k}", css::TOKEN_MARKER), t.id.clone()));
                *v = self.literal_css(t.kind, lit);
                continue;
            }
            *v = self.expand_text(v);
        }
        styles.extend(bound);
    }

    /// A literal for `css::apply`. Numbers keep full precision: the node must end up with
    /// exactly the token's value or the binding would count as edited.
    fn literal_css(&self, kind: TokenKind, value: TokenValue) -> String {
        match value {
            TokenValue::Color(c) => css::color_css(c),
            TokenValue::Text(s) => s,
            TokenValue::Alias(_) => String::new(),
            TokenValue::Number(v) => match kind {
                TokenKind::LineHeight | TokenKind::Opacity | TokenKind::FontWeight => v.to_string(),
                _ => format!("{v}px"),
            },
        }
    }

    /// Replace every `var(...)` inside a longer value such as `1px solid var(--line)`.
    fn expand_text(&self, v: &str) -> String {
        let mut out = String::new();
        let mut rest = v;
        while let Some(i) = rest.find("var(") {
            out.push_str(&rest[..i]);
            let tail = &rest[i..];
            let Some(end) = closing_paren(tail) else {
                out.push_str(tail);
                return out;
            };
            let call = &tail[..=end];
            let lit = var_name(call)
                .and_then(|n| self.by_css_name(n))
                .and_then(|t| Some(self.literal_css(t.kind, self.resolve(&t.id)?)));
            match (lit, call[4..end].split_once(',')) {
                (Some(l), _) => out.push_str(&l),
                (None, Some((_, fallback))) => out.push_str(fallback.trim()),
                (None, None) => out.push_str(call),
            }
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        out
    }

    // ---------------------------------------------------------------------------- undo

    /// What changed between `before` and `self`.
    pub fn diff(&self, before: &TokenSet) -> Vec<TokenEdit> {
        let place = |s: &TokenSet, id: &str| {
            let i = s.tokens.iter().position(|t| t.id == id)?;
            Some((i, s.tokens[i].clone()))
        };
        let mut ids: Vec<&str> = before.tokens.iter().map(|t| t.id.as_str()).collect();
        for t in &self.tokens {
            if !ids.contains(&t.id.as_str()) {
                ids.push(&t.id);
            }
        }
        ids.into_iter()
            .map(|id| TokenEdit {
                id: id.to_string(),
                before: place(before, id),
                after: place(self, id),
            })
            .filter(TokenEdit::changes_token)
            .collect()
    }

    /// Apply edits forward or back. Only the listed tokens change, so undoing on one page
    /// keeps token edits made since on another.
    pub fn apply_edits(&mut self, edits: &[TokenEdit], forward: bool) {
        for e in edits {
            self.tokens.retain(|t| t.id != e.id);
        }
        let mut back: Vec<&(usize, Token)> = edits
            .iter()
            .filter_map(|e| if forward { &e.after } else { &e.before }.as_ref())
            .collect();
        back.sort_by_key(|(i, _)| *i);
        for (i, t) in back {
            self.tokens.insert((*i).min(self.tokens.len()), t.clone());
        }
    }
}

/// Fold `newer` into `older`, keeping the oldest before-state of each token.
pub fn merge_edits(older: &mut Vec<TokenEdit>, newer: Vec<TokenEdit>) {
    for e in newer {
        match older.iter_mut().find(|x| x.id == e.id) {
            Some(x) => x.after = e.after,
            None => older.push(e),
        }
    }
    older.retain(TokenEdit::changes_token);
}

fn check_description(d: Option<&str>) -> Result<(), String> {
    match d {
        Some(d) if d.chars().count() > MAX_DESCRIPTION => Err(format!(
            "Token descriptions can be at most {MAX_DESCRIPTION} characters."
        )),
        _ => Ok(()),
    }
}

/// A node property a token can own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenProp {
    /// The top fill layer.
    Fill,
    Stroke,
    Radius,
    Opacity,
    FontSize,
    FontWeight,
    LineHeight,
    LetterSpacing,
    Gap,
    /// All four sides.
    Padding,
}

impl TokenProp {
    pub const ALL: [TokenProp; 10] = [
        TokenProp::Fill,
        TokenProp::Stroke,
        TokenProp::Radius,
        TokenProp::Opacity,
        TokenProp::FontSize,
        TokenProp::FontWeight,
        TokenProp::LineHeight,
        TokenProp::LetterSpacing,
        TokenProp::Gap,
        TokenProp::Padding,
    ];

    /// The kind of token this property accepts.
    pub fn kind(self) -> TokenKind {
        match self {
            TokenProp::Fill | TokenProp::Stroke => TokenKind::Color,
            TokenProp::Radius => TokenKind::Radius,
            TokenProp::Opacity => TokenKind::Opacity,
            TokenProp::FontSize => TokenKind::FontSize,
            TokenProp::FontWeight => TokenKind::FontWeight,
            TokenProp::LineHeight => TokenKind::LineHeight,
            TokenProp::LetterSpacing => TokenKind::LetterSpacing,
            TokenProp::Gap | TokenProp::Padding => TokenKind::Spacing,
        }
    }

    /// The field name the inspector uses for edits, which also sets the instance override.
    pub fn field(self) -> &'static str {
        match self {
            TokenProp::Fill => "fills",
            TokenProp::Stroke => "stroke",
            TokenProp::Radius => "radius",
            TokenProp::Opacity => "opacity",
            TokenProp::FontSize => "font size",
            TokenProp::FontWeight => "bold",
            TokenProp::LineHeight => "line height",
            TokenProp::LetterSpacing => "letter spacing",
            TokenProp::Gap | TokenProp::Padding => "layout",
        }
    }

    /// The CSS property that carries this on `n`.
    pub fn css_key(self, n: &Node) -> &'static str {
        match self {
            TokenProp::Fill if matches!(n.kind, NodeKind::Text { .. }) => "color",
            TokenProp::Fill => "backgroundColor",
            TokenProp::Stroke => "borderColor",
            TokenProp::Radius => "borderRadius",
            TokenProp::Opacity => "opacity",
            TokenProp::FontSize => "fontSize",
            TokenProp::FontWeight => "fontWeight",
            TokenProp::LineHeight => "lineHeight",
            TokenProp::LetterSpacing => "letterSpacing",
            TokenProp::Gap => "gap",
            TokenProp::Padding => "padding",
        }
    }

    /// The property a CSS key binds on `n`, if it binds one.
    pub fn from_css_key(key: &str, n: &Node) -> Option<TokenProp> {
        let text = n.kind.text().is_some();
        // A sticky's fill is its paper; `color` is the writing on it.
        let sticky = matches!(n.kind, NodeKind::Sticky { .. });
        Some(match key {
            "color" | "fill" if text && !sticky => TokenProp::Fill,
            "backgroundColor" | "background" if sticky => TokenProp::Fill,
            "backgroundColor" | "background" | "fill" if !text => TokenProp::Fill,
            "borderColor" | "stroke" => TokenProp::Stroke,
            "borderRadius" => TokenProp::Radius,
            "opacity" => TokenProp::Opacity,
            "fontSize" if text => TokenProp::FontSize,
            "fontWeight" if text => TokenProp::FontWeight,
            "lineHeight" if text => TokenProp::LineHeight,
            "letterSpacing" if text => TokenProp::LetterSpacing,
            "gap" if n.layout.is_some() => TokenProp::Gap,
            "padding" if n.layout.is_some() => TokenProp::Padding,
            _ => return None,
        })
    }

    /// Whether `n` has this property at all.
    pub fn applies_to(self, n: &Node) -> bool {
        let is_text = matches!(n.kind, NodeKind::Text { .. });
        match self {
            TokenProp::Fill | TokenProp::Stroke | TokenProp::Radius | TokenProp::Opacity => true,
            TokenProp::FontSize => n.kind.font_size().is_some(),
            TokenProp::FontWeight | TokenProp::LineHeight | TokenProp::LetterSpacing => is_text,
            TokenProp::Gap | TokenProp::Padding => n.layout.is_some(),
        }
    }

    /// Does `n` already show `v`?
    pub fn holds(self, n: &Node, v: &TokenValue) -> bool {
        match (self, v) {
            (TokenProp::Fill, TokenValue::Color(c)) => n.style.fill() == Some(*c),
            (TokenProp::Stroke, TokenValue::Color(c)) => n.style.stroke == Some(*c),
            (TokenProp::Radius, TokenValue::Number(r)) => n.style.corner_radii() == [*r; 4],
            (TokenProp::Opacity, TokenValue::Number(o)) => n.style.opacity == *o,
            (TokenProp::FontSize, TokenValue::Number(s)) => n.kind.font_size() == Some(*s),
            (TokenProp::FontWeight, TokenValue::Number(w)) => {
                n.text_style.base_weight() == *w as u16
            }
            (TokenProp::LineHeight, TokenValue::Number(l)) => n.text_style.line_height == Some(*l),
            (TokenProp::LetterSpacing, TokenValue::Number(s)) => n.text_style.letter_spacing == *s,
            (TokenProp::Gap, TokenValue::Number(g)) => n.layout.is_some_and(|l| l.gap == *g),
            (TokenProp::Padding, TokenValue::Number(p)) => {
                n.layout.is_some_and(|l| l.padding == [*p; 4])
            }
            _ => false,
        }
    }

    /// Make `n` show `v`.
    pub fn write(self, n: &mut Node, v: &TokenValue) {
        match (self, v) {
            (TokenProp::Fill, TokenValue::Color(c)) => match n.style.fills.last_mut() {
                Some(Paint::Solid { color }) => *color = *c,
                _ => n.style.fills.push(Paint::solid(*c)),
            },
            (TokenProp::Stroke, TokenValue::Color(c)) => n.style.stroke = Some(*c),
            (TokenProp::Radius, TokenValue::Number(r)) => {
                n.style.corner_radius = *r;
                n.style.radii = None;
            }
            (TokenProp::Opacity, TokenValue::Number(o)) => n.style.opacity = *o,
            (TokenProp::FontSize, TokenValue::Number(s)) => {
                if let NodeKind::Text { font_size, .. } | NodeKind::Sticky { font_size, .. } =
                    &mut n.kind
                {
                    *font_size = *s;
                }
            }
            (TokenProp::FontWeight, TokenValue::Number(w)) => n.text_style.set_weight(*w as u16),
            (TokenProp::LineHeight, TokenValue::Number(l)) => n.text_style.line_height = Some(*l),
            (TokenProp::LetterSpacing, TokenValue::Number(s)) => n.text_style.letter_spacing = *s,
            (TokenProp::Gap, TokenValue::Number(g)) => {
                if let Some(l) = &mut n.layout {
                    l.gap = *g;
                }
            }
            (TokenProp::Padding, TokenValue::Number(p)) => {
                if let Some(l) = &mut n.layout {
                    l.padding = [*p; 4];
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_weight_tokens_keep_the_exact_weight() {
        let mut n = Node::new(
            crate::model::NodeId(1),
            NodeKind::Text {
                text: "Hi".into(),
                font_size: 16.0,
                auto_width: true,
                align: Default::default(),
            },
            crate::geom::Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let medium = TokenValue::Number(500.0);
        let bold = TokenValue::Number(700.0);
        TokenProp::FontWeight.write(&mut n, &medium);
        assert_eq!(n.text_style.base_weight(), 500);
        assert!(TokenProp::FontWeight.holds(&n, &medium));
        assert!(!TokenProp::FontWeight.holds(&n, &bold));
        // Text that only has the bold flag counts as exactly 700, and regular text as 400.
        TokenProp::FontWeight.write(&mut n, &bold);
        assert_eq!(n.text_style.weight, None);
        assert!(TokenProp::FontWeight.holds(&n, &bold));
        assert!(!TokenProp::FontWeight.holds(&n, &TokenValue::Number(600.0)));
        TokenProp::FontWeight.write(&mut n, &TokenValue::Number(400.0));
        assert!(!TokenProp::FontWeight.holds(&n, &TokenValue::Number(300.0)));
        // A fractional token still matches what `write` stored.
        let fractional = TokenValue::Number(550.5);
        TokenProp::FontWeight.write(&mut n, &fractional);
        assert!(TokenProp::FontWeight.holds(&n, &fractional));
    }

    fn set() -> (TokenSet, String, String) {
        let mut s = TokenSet::default();
        let red = s
            .create(TokenKind::Color, "--red", "#ff0000", None)
            .unwrap();
        let brand = s
            .create(TokenKind::Color, "--brand", "var(--red)", None)
            .unwrap();
        (s, red, brand)
    }

    #[test]
    fn names_become_css_variables() {
        assert_eq!(css_slug("Primary Blue"), "primary-blue");
        assert_eq!(css_slug("Rock & Roll"), "rock-and-roll");
        assert_eq!(css_slug("spacing.4 / large"), "spacing-4-large");
        assert_eq!(css_slug("--Already--Dashed--"), "already-dashed");
        assert_eq!(css_slug("snake_case"), "snake_case");
        assert_eq!(css_slug("!!!"), "");
    }

    #[test]
    fn values_parse_by_kind() {
        let s = TokenSet::default();
        let n = TokenValue::Number;
        assert_eq!(s.parse_value(TokenKind::Spacing, "8px"), Ok(n(8.0)));
        assert_eq!(s.parse_value(TokenKind::Spacing, "1rem"), Ok(n(16.0)));
        assert_eq!(s.parse_value(TokenKind::Opacity, "50%"), Ok(n(0.5)));
        assert_eq!(s.parse_value(TokenKind::Opacity, "0.25"), Ok(n(0.25)));
        assert_eq!(s.parse_value(TokenKind::LineHeight, "150%"), Ok(n(1.5)));
        assert_eq!(s.parse_value(TokenKind::FontWeight, "bold"), Ok(n(700.0)));
        assert_eq!(
            s.parse_value(TokenKind::LetterSpacing, "0.1rem"),
            Ok(n(1.6))
        );
        assert_eq!(
            s.parse_value(TokenKind::FontFamily, "'Inter Tight'"),
            Ok(TokenValue::Text("Inter Tight".into()))
        );
        for (k, v) in [
            (TokenKind::Color, "nope"),
            (TokenKind::Opacity, "1.5"),
            (TokenKind::Radius, "-4px"),
            (TokenKind::FontSize, "0"),
            (TokenKind::LetterSpacing, "0.1em"),
            (TokenKind::LineHeight, "24px"),
            (TokenKind::FontWeight, "9000"),
            (TokenKind::Spacing, "var(--missing)"),
            (TokenKind::Spacing, "1e308rem"),
            (TokenKind::FontSize, "1e308pt"),
            (TokenKind::Radius, "1e308rem"),
        ] {
            assert!(s.parse_value(k, v).is_err(), "{k:?} {v}");
        }
    }

    #[test]
    fn create_is_idempotent_but_allows_duplicate_names() {
        let (mut s, red, _) = set();
        let again = s
            .create(TokenKind::Color, "--red", "#ff0000", None)
            .unwrap();
        assert_eq!(again, red);
        let other = s
            .create(TokenKind::Color, "--red", "#00ff00", None)
            .unwrap();
        assert_ne!(other, red);
        assert_eq!(s.by_css_name("--red").unwrap().id, red, "first one wins");
    }

    #[test]
    fn aliases_resolve_and_need_the_same_kind() {
        let (mut s, _, brand) = set();
        assert_eq!(
            s.resolve(&brand),
            Some(TokenValue::Color(Color::rgb(255, 0, 0)))
        );
        assert!(
            s.create(TokenKind::Spacing, "--x", "var(--red)", None)
                .is_err()
        );
    }

    #[test]
    fn cycles_are_refused() {
        let (mut s, red, brand) = set();
        let err = s
            .update(&red, None, Some("var(--brand)"), None)
            .unwrap_err();
        assert!(err.contains("Circular"), "{err}");
        assert!(s.update(&brand, None, Some("var(--brand)"), None).is_err());
        assert!(matches!(s.get(&red).unwrap().value, TokenValue::Color(_)));
    }

    #[test]
    fn a_failed_update_changes_nothing() {
        let (mut s, red, _) = set();
        let before = s.clone();
        assert!(s.update(&red, Some("new"), Some("nope"), None).is_err());
        assert_eq!(s, before);
    }

    #[test]
    fn listing_groups_by_kind_in_creation_order() {
        let mut s = TokenSet::default();
        s.create(TokenKind::Radius, "r", "4px", None).unwrap();
        s.create(TokenKind::Color, "b", "#000000", None).unwrap();
        s.create(TokenKind::Color, "a", "#ffffff", None).unwrap();
        let names: Vec<&str> = s.sorted().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["b", "a", "r"]);
    }

    #[test]
    fn deleting_flattens_aliases_through_the_deleted_set() {
        let (mut s, red, brand) = set();
        let cta = s
            .create(TokenKind::Color, "--cta", "var(--brand)", None)
            .unwrap();
        s.delete(&[red, brand]);
        assert_eq!(
            s.get(&cta).unwrap().value,
            TokenValue::Color(Color::rgb(255, 0, 0))
        );
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn duplicates_sit_after_the_source_with_copy_names() {
        let mut s = TokenSet::default();
        let a = s.create(TokenKind::Spacing, "a", "4px", None).unwrap();
        s.create(TokenKind::Spacing, "b", "8px", None).unwrap();
        let first = s.duplicate(std::slice::from_ref(&a));
        let second = s.duplicate(std::slice::from_ref(&a));
        let third = s.duplicate(&second);
        assert_eq!(s.get(&first[0]).unwrap().name, "a copy");
        assert_eq!(s.get(&second[0]).unwrap().name, "a copy 2");
        assert_eq!(s.get(&third[0]).unwrap().name, "a copy 3");
        let names: Vec<&str> = s.sorted().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "a copy 2", "a copy 3", "a copy", "b"]);
    }

    #[test]
    fn stylesheets_use_css_values_and_keep_aliases() {
        let mut s = TokenSet::default();
        s.create(TokenKind::Color, "Primary Blue", "#0000ff", None)
            .unwrap();
        s.create(TokenKind::Color, "link", "var(--primary-blue)", None)
            .unwrap();
        s.create(TokenKind::Opacity, "dim", "0.5", None).unwrap();
        s.create(TokenKind::LineHeight, "body", "1.5", None)
            .unwrap();
        s.create(TokenKind::Spacing, "s", "8px", None).unwrap();
        s.create(TokenKind::FontWeight, "w", "700", None).unwrap();
        s.create(TokenKind::FontFamily, "f", "Inter Tight", None)
            .unwrap();
        let all = s.sorted();
        let css = s.stylesheet(&all, false);
        assert!(css.starts_with(":root {\n"), "{css}");
        for line in [
            "--primary-blue: #0000ff;",
            "--link: var(--primary-blue);",
            "--dim: 50%;",
            "--body: 150%;",
            "--s: 8px;",
            "--w: 700;",
            "--f: \"Inter Tight\";",
        ] {
            assert!(css.contains(line), "{line} missing from {css}");
        }
        assert!(s.stylesheet(&all, true).starts_with("@theme {\n"));
    }

    #[test]
    fn paste_keeps_copies_independent_and_remaps_aliases() {
        let (s, red, brand) = set();
        let clip = s.copy(&[red.clone(), brand.clone()]);
        let mut other = TokenSet::default();
        let new = other.paste(&clip);
        assert_eq!(other.len(), 2);
        assert!(!new.contains(&red));
        let pasted = other.by_css_name("--brand").unwrap();
        assert_eq!(pasted.value, TokenValue::Alias(new[0].clone()));
    }

    #[test]
    fn paste_finds_same_named_target_or_flattens() {
        let (s, _, brand) = set();
        let clip = s.copy(std::slice::from_ref(&brand));

        let mut with_target = TokenSet::default();
        let here = with_target
            .create(TokenKind::Color, "red", "#00ff00", None)
            .unwrap();
        with_target.paste(&clip);
        assert_eq!(
            with_target.by_css_name("--brand").unwrap().value,
            TokenValue::Alias(here)
        );

        let mut empty = TokenSet::default();
        empty.paste(&clip);
        assert_eq!(
            empty.by_css_name("--brand").unwrap().value,
            TokenValue::Color(Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn paste_renames_on_collision() {
        let (mut s, red, _) = set();
        let clip = s.copy(std::slice::from_ref(&red));
        s.paste(&clip);
        assert!(s.by_css_name("--red-copy").is_some());
    }

    #[test]
    fn expand_replaces_variables_and_marks_whole_values() {
        let (s, _, brand) = set();
        let mut styles = vec![
            ("backgroundColor".to_string(), "var(--brand)".to_string()),
            ("border".to_string(), "1px solid var(--red)".to_string()),
            ("color".to_string(), "var(--nope, #123456)".to_string()),
        ];
        s.expand(&mut styles);
        assert_eq!(styles[0].1, "#ff0000");
        assert_eq!(styles[1].1, "1px solid #ff0000");
        assert_eq!(styles[2].1, "#123456");
        let marker = format!("{}backgroundColor", css::TOKEN_MARKER);
        assert!(styles.contains(&(marker, brand)));
        assert_eq!(styles.len(), 4, "only whole values bind: {styles:?}");
    }

    #[test]
    fn expand_handles_fallbacks_with_parentheses() {
        let (s, _, _) = set();
        let mut styles = vec![
            (
                "border".to_string(),
                "1px solid var(--red, rgb(1, 2, 3))".to_string(),
            ),
            (
                "outline".to_string(),
                "1px solid var(--no, rgb(1, 2, 3))".to_string(),
            ),
            ("color".to_string(), "var(--no, rgb(1, 2, 3))".to_string()),
        ];
        s.expand(&mut styles);
        assert_eq!(styles[0].1, "1px solid #ff0000");
        assert_eq!(styles[1].1, "1px solid rgb(1, 2, 3)");
        assert_eq!(styles[2].1, "rgb(1, 2, 3)");
    }

    #[test]
    fn edits_undo_one_token_without_touching_others() {
        let (mut s, red, _) = set();
        let before = s.clone();
        s.update(&red, None, Some("#00ff00"), None).unwrap();
        let edits = s.diff(&before);
        assert_eq!(edits.len(), 1);
        // An edit from another page lands in between.
        let added = s.create(TokenKind::Spacing, "s", "8px", None).unwrap();
        s.apply_edits(&edits, false);
        assert_eq!(
            s.get(&red).unwrap().value,
            TokenValue::Color(Color::rgb(255, 0, 0))
        );
        assert!(s.get(&added).is_some());
    }

    #[test]
    fn moved_neighbours_are_not_edits() {
        let mut s = TokenSet::default();
        let a = s.create(TokenKind::Spacing, "a", "4px", None).unwrap();
        let b = s.create(TokenKind::Spacing, "b", "8px", None).unwrap();
        let before = s.clone();
        s.delete(std::slice::from_ref(&a));
        let edits = s.diff(&before);
        assert_eq!(edits.len(), 1, "only the deleted token: {edits:?}");
        assert_eq!(edits[0].id, a);

        // Merged edits follow the same rule: adding a token in front of `b` and removing it
        // again leaves nothing to undo.
        let start = s.clone();
        let c = s.create(TokenKind::Spacing, "c", "2px", None).unwrap();
        let mut merged = s.diff(&start);
        let added = s.clone();
        s.delete(std::slice::from_ref(&c));
        merge_edits(&mut merged, s.diff(&added));
        assert!(merged.is_empty(), "{merged:?}");
        assert!(s.get(&b).is_some());
    }

    #[test]
    fn deleted_tokens_come_back_in_place() {
        let mut s = TokenSet::default();
        let a = s.create(TokenKind::Spacing, "a", "4px", None).unwrap();
        let b = s.create(TokenKind::Spacing, "b", "8px", None).unwrap();
        s.create(TokenKind::Spacing, "c", "16px", None).unwrap();
        let before = s.clone();
        s.delete(&[a, b]);
        let edits = s.diff(&before);
        s.apply_edits(&edits, false);
        assert_eq!(s, before);
        s.apply_edits(&edits, true);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn file_form_roundtrips() {
        let (s, _, _) = set();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"type\":\"color\""), "{json}");
        let back: TokenSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
