//! Design tokens: named values shared by every page of a project.
//!
//! Behaviour follows the notes in `analysis/design-tokens.md`. A node property bound to a token
//! keeps the resolved value in its usual field, so renderers and exporters never look at
//! tokens. `Node::tokens` only records which token owns the field, and one rule keeps the two
//! honest: a bound field always holds its token's value. The editor rewrites fields when a
//! token changes, and drops a binding when a field stops matching (see `editor/tokens.rs`).

use super::components::LinkedComponent;
use super::document::Document;
use super::effect_styles::EffectStyleDef;
use super::node::{Color, Node, NodeId, NodeKind};
use super::paint::{LetterSpacing, Paint};
use super::text_styles::TextStyleDef;
use crate::mcp::css;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

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
    BorderWidth,
}

impl TokenKind {
    /// Listing order: grouped by kind, then in creation order within a kind.
    pub const ALL: [TokenKind; 12] = [
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
        TokenKind::BorderWidth,
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
            TokenKind::BorderWidth => "Border width",
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
            TokenKind::BorderWidth => "borderWidth",
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
            TokenKind::BorderWidth => TokenValue::Number(1.0),
        }
    }
}

/// What a token holds. Numbers are px, except line height (a multiple of the font size),
/// opacity (0 to 1) and font weight. Letter spacing can also be em.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenValue {
    Color(Color),
    Number(f64),
    Text(String),
    /// Letter spacing relative to the font size of the text it's bound to.
    Em(f64),
    /// Another token of the same kind, by id.
    Alias(String),
}

/// A theme such as Light or Dark. A file and each design system have a list of them, and a
/// frame picks one for itself and its children (`Node::mode`). Modes match across lists by
/// name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub id: String,
    pub name: String,
    /// Ids of design system modes of the same name that this one took over when the system
    /// was detached. Frames that picked them show this mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aka: Vec<String>,
}

impl Mode {
    pub(crate) fn new(id: &str, name: &str) -> Mode {
        Mode {
            id: id.into(),
            name: name.into(),
            aka: Vec::new(),
        }
    }

    pub fn defaults() -> Vec<Mode> {
        vec![Mode::new("light", "Light"), Mode::new("dark", "Dark")]
    }

    /// The one mode a file saved before modes existed opens with.
    pub fn single() -> Vec<Mode> {
        vec![Mode::new("light", "Light")]
    }
}

impl Keyed for Mode {
    fn key(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    /// What the user typed. See [`css_slug`] for the CSS variable it becomes.
    pub name: String,
    #[serde(rename = "type")]
    pub kind: TokenKind,
    /// The value in the first mode, and in every mode without one of its own.
    pub value: TokenValue,
    /// Values for the other modes of the token's file or design system, by mode id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, TokenValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The design system this token comes from, by id. Such a token is a read-only copy of
    /// the published one; see [`SystemLink`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

/// A published design system this project uses. Its tokens sit in the [`TokenSet`] with
/// `Token::system` set to `id`, under ids from [`linked_id`], and their CSS names start with
/// `prefix`. A source project keeps one too, describing what it last published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemLink {
    pub id: String,
    pub name: String,
    pub prefix: String,
    /// The published version this copy holds, counting from 1.
    pub release: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How to use the system, in Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The system's modes, under ids from [`linked_id`]. Empty for one published before
    /// modes existed, whose tokens have one value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<Mode>,
}

/// The id a published token gets in a project that uses its system. Ids are only unique per
/// project, and a duplicated project shares them, so the system id keeps them apart. The id
/// doesn't change between versions, so bindings carry over.
pub fn linked_id(system: &str, token: &str) -> String {
    format!("{system}/{token}")
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

/// Whether two mode names are the same mode. They match as `data-theme` values do, so "Dark"
/// and "dark" are one mode and can't write two CSS blocks.
fn same_mode(a: &str, b: &str) -> bool {
    css_slug(a) == css_slug(b)
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

pub(super) fn var_name(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("var(")?.strip_suffix(')')?;
    Some(inner.split(',').next()?.trim())
}

/// Tokens copied from the Tokens panel. Pasted tokens are independent of their source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenClip {
    /// Their `modes` hold a value for every mode a frame could pick, keyed by the mode's
    /// `data-theme` name, to match the modes of another project.
    pub tokens: Vec<Token>,
    /// Resolved values of alias targets outside the copied set, keyed by token id, then
    /// by `data-theme` name, first mode first.
    resolved: HashMap<String, Vec<(String, TokenValue)>>,
    /// Kind and CSS name (without dashes) of those targets, to find the same token in
    /// another project.
    targets: HashMap<String, (TokenKind, String)>,
}

/// Something with an id, kept in a list that undo restores item by item.
pub trait Keyed: Clone + PartialEq {
    fn key(&self) -> &str;
}

impl Keyed for Token {
    fn key(&self) -> &str {
        &self.id
    }
}

impl Keyed for SystemLink {
    fn key(&self) -> &str {
        &self.id
    }
}

/// One item's change, for undo. `Some((index, item))` is its place in the list on that side.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit<T> {
    id: String,
    before: Option<(usize, T)>,
    after: Option<(usize, T)>,
}

impl<T: Keyed> Edit<T> {
    /// Whether the item itself differs. One that only moved because its neighbours changed
    /// isn't an edit, and recording it would let undo overwrite newer changes.
    fn changes(&self) -> bool {
        self.before.as_ref().map(|(_, t)| t) != self.after.as_ref().map(|(_, t)| t)
    }
}

/// A change to one token, mode, text style, effect style or design system link.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenEdit {
    Token(Edit<Token>),
    Mode(Edit<Mode>),
    System(Edit<SystemLink>),
    Style(Box<Edit<TextStyleDef>>),
    EffectStyle(Edit<EffectStyleDef>),
    Component(Edit<LinkedComponent>),
}

impl TokenEdit {
    fn changes(&self) -> bool {
        match self {
            TokenEdit::Token(e) => e.changes(),
            TokenEdit::Mode(e) => e.changes(),
            TokenEdit::System(e) => e.changes(),
            TokenEdit::Style(e) => e.changes(),
            TokenEdit::EffectStyle(e) => e.changes(),
            TokenEdit::Component(e) => e.changes(),
        }
    }

    /// Fold a newer edit of the same item into this one. False when it's another item.
    fn absorb(&mut self, newer: &TokenEdit) -> bool {
        match (self, newer) {
            (TokenEdit::Token(a), TokenEdit::Token(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            (TokenEdit::System(a), TokenEdit::System(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            (TokenEdit::Mode(a), TokenEdit::Mode(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            (TokenEdit::Style(a), TokenEdit::Style(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            (TokenEdit::EffectStyle(a), TokenEdit::EffectStyle(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            (TokenEdit::Component(a), TokenEdit::Component(b)) if a.id == b.id => {
                a.after = b.after.clone();
                true
            }
            _ => false,
        }
    }
}

fn diff_list<T: Keyed>(before: &[T], after: &[T]) -> Vec<Edit<T>> {
    let place = |s: &[T], id: &str| {
        let i = s.iter().position(|t| t.key() == id)?;
        Some((i, s[i].clone()))
    };
    let mut ids: Vec<&str> = before.iter().map(Keyed::key).collect();
    for t in after {
        if !ids.contains(&t.key()) {
            ids.push(t.key());
        }
    }
    ids.into_iter()
        .map(|id| Edit {
            id: id.to_string(),
            before: place(before, id),
            after: place(after, id),
        })
        .filter(Edit::changes)
        .collect()
}

fn apply_list<'a, T: Keyed + 'a>(
    list: &mut Vec<T>,
    edits: impl Iterator<Item = &'a Edit<T>> + Clone,
    forward: bool,
) {
    for e in edits.clone() {
        list.retain(|t| t.key() != e.id);
    }
    let mut back: Vec<&(usize, T)> = edits
        .filter_map(|e| if forward { &e.after } else { &e.before }.as_ref())
        .collect();
    back.sort_by_key(|(i, _)| *i);
    for (i, t) in back {
        list.insert((*i).min(list.len()), t.clone());
    }
}

/// The project's tokens, modes, text styles and effect styles, and the design systems it
/// uses. Vector order is creation order, and duplicates sit after their source. Only the
/// tokens are serialized here: the project manifest keeps the modes, system links and styles
/// in fields of their own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<Token>", into = "Vec<Token>")]
pub struct TokenSet {
    pub(super) tokens: Vec<Token>,
    /// Never empty. The first is the default for layers outside any frame with a mode.
    pub(super) modes: Vec<Mode>,
    pub(super) systems: Vec<SystemLink>,
    pub(super) styles: Vec<TextStyleDef>,
    pub(super) effect_styles: Vec<EffectStyleDef>,
    pub(super) components: Vec<LinkedComponent>,
}

impl Default for TokenSet {
    fn default() -> TokenSet {
        TokenSet {
            tokens: Vec::new(),
            modes: Mode::defaults(),
            systems: Vec::new(),
            styles: Vec::new(),
            effect_styles: Vec::new(),
            components: Vec::new(),
        }
    }
}

impl From<Vec<Token>> for TokenSet {
    fn from(tokens: Vec<Token>) -> TokenSet {
        TokenSet {
            tokens,
            ..TokenSet::default()
        }
    }
}

impl From<TokenSet> for Vec<Token> {
    fn from(s: TokenSet) -> Vec<Token> {
        s.tokens
    }
}

impl TokenSet {
    /// A token set as a project stores it: tokens and system links apart.
    pub fn with_systems(mut self, systems: Vec<SystemLink>) -> TokenSet {
        self.systems = systems;
        self
    }

    /// A token set as a project stores it: modes apart. An empty list keeps the default.
    pub fn with_modes(mut self, modes: Vec<Mode>) -> TokenSet {
        if !modes.is_empty() {
            self.modes = modes;
        }
        self
    }

    /// This file's modes. The first is the default.
    pub fn modes(&self) -> &[Mode] {
        &self.modes
    }

    /// The modes `t` keeps values for: its design system's, or this file's.
    pub fn modes_of(&self, t: &Token) -> &[Mode] {
        match &t.system {
            Some(s) => self.system(s).map_or(&[], |l| &l.modes),
            None => &self.modes,
        }
    }

    /// The modes a frame can pick: this file's, then each design system's that the file has
    /// no mode of the same name for.
    pub fn choices(&self) -> Vec<&Mode> {
        let mut out: Vec<&Mode> = self.modes.iter().collect();
        for m in self.systems.iter().flat_map(|s| &s.modes) {
            if !out.iter().any(|o| same_mode(&o.name, &m.name)) {
                out.push(m);
            }
        }
        out
    }

    pub fn mode_name(&self, id: &str) -> Option<&str> {
        let all = self
            .modes
            .iter()
            .chain(self.systems.iter().flat_map(|s| &s.modes));
        // A live mode wins over one a local mode stands in for.
        let all: Vec<&Mode> = all.collect();
        (all.iter().find(|m| m.id == id))
            .or_else(|| all.iter().find(|m| m.aka.iter().any(|a| a == id)))
            .map(|m| m.name.as_str())
    }

    /// A mode a frame can pick, by name ignoring case or by its `data-theme` name.
    pub fn find_choice(&self, name: &str) -> Option<&Mode> {
        let name = name.trim();
        let slug = css_slug(name);
        self.choices()
            .into_iter()
            .find(|m| m.name.eq_ignore_ascii_case(name) || css_slug(&m.name) == slug)
    }

    /// The mode layer `id` shows: that of the nearest frame, itself included, that picks one.
    /// None is the default mode.
    pub fn mode_at(&self, doc: &Document, id: NodeId) -> Option<&str> {
        let mut cur = Some(id);
        while let Some(n) = cur.and_then(|c| doc.get(c)) {
            if let Some(name) = n.mode.as_deref().and_then(|m| self.mode_name(m)) {
                return Some(name);
            }
            cur = n.parent;
        }
        None
    }

    fn default_mode(&self) -> &str {
        self.modes.first().map_or("", |m| m.name.as_str())
    }

    /// `t`'s own value in the mode named `mode` (None for the default), aliases unfollowed.
    /// A design system without a mode of that name gives its first value.
    pub fn value_in<'a>(&self, t: &'a Token, mode: Option<&str>) -> &'a TokenValue {
        self.pick(self.modes_of(t), &t.value, &t.modes, mode)
    }

    /// The value for the mode named `mode` from a first value and other modes' values keyed
    /// by the ids in `list`, as tokens and style fields hold them.
    pub(super) fn pick<'a>(
        &self,
        list: &[Mode],
        value: &'a TokenValue,
        modes: &'a BTreeMap<String, TokenValue>,
        mode: Option<&str>,
    ) -> &'a TokenValue {
        let name = mode.unwrap_or(self.default_mode());
        list.iter()
            .find(|m| same_mode(&m.name, name))
            .and_then(|m| modes.get(&m.id))
            .unwrap_or(value)
    }

    /// Add Dark if absent, otherwise a numbered mode. Return its id.
    pub fn add_mode(&mut self) -> String {
        let taken = |n: &str| self.modes.iter().any(|m| css_slug(&m.name) == css_slug(n));
        let name = match taken("Dark") {
            false => "Dark".to_string(),
            true => (self.modes.len() + 1..)
                .map(|i| format!("Mode {i}"))
                .find(|n| !taken(n))
                .unwrap_or_default(),
        };
        let id = crate::project::new_id();
        self.modes.push(Mode {
            id: id.clone(),
            name,
            aka: Vec::new(),
        });
        id
    }

    /// Rename a mode. Names must differ as `data-theme` values too.
    pub fn rename_mode(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        let slug = css_slug(name);
        if slug.is_empty() {
            return Err(format!("Mode name \"{name}\" needs a letter or number."));
        }
        if self
            .modes
            .iter()
            .any(|m| m.id != id && css_slug(&m.name) == slug)
        {
            return Err(format!("This file already has a mode called \"{name}\"."));
        }
        let m = self
            .modes
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or("Mode not found")?;
        m.name = name.to_string();
        Ok(())
    }

    /// Remove a mode and its values. Every other mode keeps showing what it did, so removing
    /// the first moves the next one's values into `Token::value`. A file keeps one mode.
    pub fn delete_mode(&mut self, id: &str) -> Result<(), String> {
        let at = self
            .modes
            .iter()
            .position(|m| m.id == id)
            .ok_or("Mode not found")?;
        if self.modes.len() == 1 {
            return Err("A file needs at least one mode.".into());
        }
        let rest: Vec<String> = self.modes[1..].iter().map(|m| m.id.clone()).collect();
        let drop = |value: &mut TokenValue, modes: &mut BTreeMap<String, TokenValue>| {
            modes.remove(id);
            if at == 0 {
                let old = value.clone();
                if let Some(v) = modes.remove(&rest[0]) {
                    *value = v;
                }
                for m in &rest[1..] {
                    if !modes.contains_key(m) && old != *value {
                        modes.insert(m.clone(), old.clone());
                    }
                }
            }
        };
        for t in self.tokens.iter_mut().filter(|t| t.system.is_none()) {
            drop(&mut t.value, &mut t.modes);
        }
        for s in self.styles.iter_mut().filter(|s| s.system.is_none()) {
            s.each_field(|_, v, modes| drop(v, modes));
        }
        self.modes.remove(at);
        Ok(())
    }

    /// The design systems this project uses, in the order they were added.
    pub fn systems(&self) -> &[SystemLink] {
        &self.systems
    }

    pub fn system(&self, id: &str) -> Option<&SystemLink> {
        self.systems.iter().find(|s| s.id == id)
    }

    /// Why a token can't be edited here: it belongs to a design system.
    pub fn locked(&self, id: &str) -> Option<String> {
        let t = self.get(id)?;
        let sys = self.system(t.system.as_deref()?)?;
        Some(format!(
            "\"{}\" belongs to the {} design system, so it can't be changed in this file. \
             Alias it from a local token, or detach the system.",
            self.css_name(t),
            sys.name
        ))
    }

    /// The CSS variable name of `t`, without the leading dashes. A design system's tokens
    /// start with its prefix.
    pub fn slug(&self, t: &Token) -> String {
        match t.system.as_deref().and_then(|s| self.system(s)) {
            Some(sys) => format!("{}-{}", sys.prefix, css_slug(&t.name)),
            None => css_slug(&t.name),
        }
    }

    /// No tokens. There may still be text and effect styles.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// No tokens and no styles.
    pub fn is_blank(&self) -> bool {
        self.tokens.is_empty() && self.styles.is_empty() && self.effect_styles.is_empty()
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
        self.sorted().into_iter().find(|t| self.slug(t) == want)
    }

    pub fn css_name(&self, t: &Token) -> String {
        format!("--{}", self.slug(t))
    }

    /// Follow aliases to a literal in the default mode.
    pub fn resolve(&self, id: &str) -> Option<TokenValue> {
        self.resolve_in(id, None)
    }

    /// Follow aliases to a literal in the mode named `mode`, each link taking its value in
    /// that mode. None when a link is missing or the chain loops, which only a hand-edited
    /// file can produce.
    pub fn resolve_in(&self, id: &str, mode: Option<&str>) -> Option<TokenValue> {
        let mut seen = HashSet::new();
        let mut cur = id;
        loop {
            if !seen.insert(cur) {
                return None;
            }
            match self.value_in(self.get(cur)?, mode) {
                TokenValue::Alias(next) => cur = next,
                v => return Some(v.clone()),
            }
        }
    }

    /// Every value token `id` shows in the modes a frame can pick.
    pub fn resolve_any(&self, id: &str) -> Vec<TokenValue> {
        let mut out = Vec::new();
        for m in self.choices() {
            if let Some(v) = self.resolve_in(id, Some(&m.name))
                && !out.contains(&v)
            {
                out.push(v);
            }
        }
        out
    }

    fn would_cycle(&self, id: &str, target: &str, mode: Option<&str>) -> bool {
        let mut cur = target;
        let mut seen = HashSet::new();
        while seen.insert(cur) {
            if cur == id {
                return true;
            }
            match self.get(cur).map(|t| self.value_in(t, mode)) {
                Some(TokenValue::Alias(next)) => cur = next,
                _ => return false,
            }
        }
        true
    }

    fn first_value_modes(&self, id: &str) -> Vec<String> {
        let own = self.get(id).map(|t| (self.modes_of(t), &t.modes));
        self.choices()
            .into_iter()
            .filter(|m| {
                own.is_none_or(|(modes, values)| {
                    !modes
                        .iter()
                        .any(|o| same_mode(&o.name, &m.name) && values.contains_key(&o.id))
                })
            })
            .map(|m| m.name.clone())
            .collect()
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
            TokenKind::LetterSpacing => match css::letter_spacing(s).ok_or_else(bad)? {
                LetterSpacing::Px(px) => px,
                LetterSpacing::Em { em } => return Ok(TokenValue::Em(em)),
            },
            TokenKind::FontSize => Some(size()?).filter(|v| *v > 0.0).ok_or_else(bad)?,
            TokenKind::Breakpoint | TokenKind::Container | TokenKind::Spacing => size()?,
            TokenKind::Radius | TokenKind::BorderWidth => {
                Some(size()?).filter(|v| *v >= 0.0).ok_or_else(bad)?
            }
        };
        // Unit conversion can overflow, and JSON has no infinity: saving one would write
        // null, which stops the project from opening.
        if !n.is_finite() {
            return Err(bad());
        }
        Ok(TokenValue::Number(n))
    }

    fn check(&self, id: &str, name: &str, value: &TokenValue, raw: &str) -> Result<(), String> {
        let slug = css_slug(name);
        if slug.is_empty() {
            return Err(format!(
                "Token name \"{name}\" needs a letter or number to work as a CSS variable."
            ));
        }
        if let Some(t) = self
            .tokens
            .iter()
            .find(|t| t.system.is_some() && t.id != id && self.slug(t) == slug)
            && let Some(sys) = self.system(t.system.as_deref().unwrap_or_default())
        {
            return Err(format!(
                "\"--{slug}\" is a token of the {} design system. Pick another name.",
                sys.name
            ));
        }
        if let Some(s) = self.effect_var_taken(&slug, "") {
            return Err(format!(
                "\"--{slug}\" belongs to the effect style \"{}\". Pick another name.",
                s.name
            ));
        }
        if let TokenValue::Alias(target) = value
            && (self.first_value_modes(id).iter()).any(|m| self.would_cycle(id, target, Some(m)))
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
        self.create_in_modes(kind, name, raw, &[], description)
    }

    /// [`create`](Self::create) with values for other modes, as (mode id, input) pairs.
    pub fn create_in_modes(
        &mut self,
        kind: TokenKind,
        name: &str,
        raw: &str,
        modes: &[(String, String)],
        description: Option<&str>,
    ) -> Result<String, String> {
        let name = name.trim().trim_start_matches("--").to_string();
        let value = self.parse_value(kind, raw)?;
        let mut values = BTreeMap::new();
        for (mode, raw) in modes {
            if !self.modes.iter().skip(1).any(|m| m.id == *mode) {
                return Err("Mode not found".into());
            }
            let v = self.parse_value(kind, raw)?;
            if v != value {
                values.insert(mode.clone(), v);
            }
        }
        if let Some(t) = self.tokens.iter().find(|t| {
            t.system.is_none()
                && t.kind == kind
                && t.name == name
                && t.value == value
                && t.modes == values
        }) {
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
            modes: values,
            description: description.filter(|d| !d.is_empty()).map(String::from),
            system: None,
        });
        Ok(id)
    }

    /// Add a token with the kind's default value and an unused name (the Tokens panel's `+`).
    pub fn add_default(&mut self, kind: TokenKind) -> String {
        let base = kind.key().to_string();
        let mut name = base.clone();
        let mut i = 2;
        while self.name_taken(kind, &name) {
            name = format!("{base}-{i}");
            i += 1;
        }
        let id = crate::project::new_id();
        self.tokens.push(Token {
            id: id.clone(),
            name,
            kind,
            value: kind.default_value(),
            modes: BTreeMap::new(),
            description: None,
            system: None,
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
        if let Some(why) = self.locked(id) {
            return Err(why);
        }
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
        if self.locked(id).is_some() {
            return;
        }
        if let Some(t) = self.tokens.iter_mut().find(|t| t.id == id) {
            t.value = value;
        }
    }

    /// Set token `id`'s value in one of this file's modes from user or agent input. None, or
    /// the first mode's value, clears it so the mode follows the first.
    pub fn update_in_mode(
        &mut self,
        id: &str,
        mode: &str,
        raw: Option<&str>,
    ) -> Result<(), String> {
        let t = self.get(id).ok_or("Token not found")?;
        if let Some(why) = self.locked(id) {
            return Err(why);
        }
        let at = self
            .modes
            .iter()
            .position(|m| m.id == mode)
            .ok_or("Mode not found")?;
        if at == 0 {
            let raw = raw.ok_or("The first mode's value can't be cleared.")?;
            return self.update(id, None, Some(raw), None);
        }
        let value = raw.map(|r| self.parse_value(t.kind, r)).transpose()?;
        // Clearing falls back to the first value, which may be an alias too.
        if let TokenValue::Alias(target) = value.as_ref().unwrap_or(&t.value)
            && self.would_cycle(id, target, Some(&self.modes[at].name))
        {
            return Err(format!(
                "Circular token reference \"{}\".",
                raw.unwrap_or_default()
            ));
        }
        self.write_in_mode(id, mode, value);
        Ok(())
    }

    /// Set a literal for one mode directly, like [`set_value`](Self::set_value).
    pub fn set_value_in(&mut self, id: &str, mode: &str, value: TokenValue) {
        if self.modes.first().is_some_and(|m| m.id == mode) {
            return self.set_value(id, value);
        }
        if self.locked(id).is_none() && self.modes.iter().any(|m| m.id == mode) {
            self.write_in_mode(id, mode, Some(value));
        }
    }

    fn write_in_mode(&mut self, id: &str, mode: &str, value: Option<TokenValue>) {
        let Some(t) = self.tokens.iter_mut().find(|t| t.id == id) else {
            return;
        };
        match value {
            Some(v) if v != t.value => t.modes.insert(mode.to_string(), v),
            _ => t.modes.remove(mode),
        };
    }

    /// Remove tokens. A surviving alias to a removed token takes on the value it showed, so
    /// nothing in the project changes appearance. A design system's tokens stay.
    pub fn delete(&mut self, ids: &[String]) {
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| self.locked(id).is_none())
            .cloned()
            .collect();
        self.remove(&ids);
    }

    fn remove(&mut self, ids: &[String]) {
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.flatten_style_aliases(&gone);
        self.flatten_effect_style_aliases(&gone);
        let flat: Vec<Token> = self
            .tokens
            .iter()
            .filter(|t| !gone.contains(t.id.as_str()))
            .map(|t| self.flattened(t, |a| gone.contains(a)))
            .collect();
        self.tokens = flat;
    }

    /// Replace aliases to removed tokens with literals, preserving each mode's appearance.
    fn flattened(&self, t: &Token, gone: impl Fn(&str) -> bool) -> Token {
        let mut out = t.clone();
        self.flatten_value(
            t.kind,
            self.modes_of(t),
            &mut out.value,
            &mut out.modes,
            &gone,
        );
        out
    }

    /// Replace aliases to removed tokens in a first value and the values of the other modes
    /// in `list`, preserving each mode's appearance.
    pub(super) fn flatten_value(
        &self,
        kind: TokenKind,
        list: &[Mode],
        value: &mut TokenValue,
        modes: &mut BTreeMap<String, TokenValue>,
        gone: &impl Fn(&str) -> bool,
    ) {
        let hit = |v: &TokenValue| matches!(v, TokenValue::Alias(a) if gone(a));
        if !hit(value) && !modes.values().any(hit) {
            return;
        }
        let flat = |v: &TokenValue, mode: Option<&Mode>| match v {
            TokenValue::Alias(a) if gone(a) => self
                .resolve_in(a, mode.map(|m| m.name.as_str()))
                .unwrap_or_else(|| kind.default_value()),
            v => v.clone(),
        };
        let first = flat(value, list.first());
        *modes = (list.iter().skip(1))
            .filter_map(|m| {
                let v = flat(modes.get(&m.id).unwrap_or(value), Some(m));
                (v != first).then(|| (m.id.clone(), v))
            })
            .collect();
        *value = first;
    }

    /// Copy tokens, each new one right after the last token of its kind in `ids`. Returns the
    /// new ids.
    pub fn duplicate(&mut self, ids: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids {
            let Some(i) = self.tokens.iter().position(|t| t.id == *id) else {
                continue;
            };
            let names = self.modes_needed(&self.tokens[i]);
            self.promote_modes(&names);
            let mut t = self.local_copy(&self.tokens[i]);
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

    /// Names of the modes a frame can pick in which `t` shows something other than its
    /// default value. A local copy keeps them only if they are this file's own.
    fn modes_needed(&self, t: &Token) -> Vec<String> {
        let fallback = self.value_in(t, None);
        (self.choices().into_iter())
            .filter(|m| self.value_in(t, Some(&m.name)) != fallback)
            .map(|m| m.name.clone())
            .collect()
    }

    /// Make design system modes named in `names` this file's own, so local tokens can hold
    /// values for them. Each gets a fresh id and stands in for the system's (`Mode::aka`), so
    /// frames that picked it keep it once the system is gone, and the two stay apart while
    /// it's live.
    fn promote_modes(&mut self, names: &[String]) {
        let missing: Vec<Mode> = (self.systems.iter().flat_map(|s| &s.modes))
            .filter(|m| names.iter().any(|n| same_mode(n, &m.name)))
            .filter(|m| !self.modes.iter().any(|x| same_mode(&x.name, &m.name)))
            .cloned()
            .collect();
        for m in missing {
            if !self.modes.iter().any(|x| same_mode(&x.name, &m.name)) {
                self.modes.push(Mode {
                    id: crate::project::new_id(),
                    name: m.name.clone(),
                    aka: Vec::new(),
                });
                self.stand_in(&m);
            }
        }
    }

    /// Make this file's mode named like system mode `m` the one that stands in for it. Only
    /// one local mode stands in for a given system mode: an older one may have been renamed.
    fn stand_in(&mut self, m: &Mode) {
        for x in &mut self.modes {
            x.aka.retain(|a| *a != m.id);
        }
        if let Some(local) =
            (self.modes.iter_mut()).find(|x| x.id != m.id && same_mode(&x.name, &m.name))
        {
            local.aka.push(m.id.clone());
        }
    }

    /// Keep a system token's CSS name, and give each local mode the system's value for the
    /// mode of the same name.
    fn local_copy(&self, t: &Token) -> Token {
        if t.system.is_none() {
            return t.clone();
        }
        let (value, modes) = self.localized(self.modes_of(t), &t.value, &t.modes);
        Token {
            name: self.slug(t),
            value,
            modes,
            system: None,
            ..t.clone()
        }
    }

    /// A first value and other modes' values keyed by the ids in `list`, rekeyed to this
    /// file's modes by name.
    pub(super) fn localized(
        &self,
        list: &[Mode],
        value: &TokenValue,
        modes: &BTreeMap<String, TokenValue>,
    ) -> (TokenValue, BTreeMap<String, TokenValue>) {
        let first = self.modes.first().map(|m| m.name.as_str());
        let base = self.pick(list, value, modes, first).clone();
        let rest = (self.modes.iter().skip(1))
            .map(|m| {
                (
                    m.id.clone(),
                    self.pick(list, value, modes, Some(&m.name)).clone(),
                )
            })
            .filter(|(_, v)| *v != base)
            .collect();
        (base, rest)
    }

    fn unused_name(&self, kind: TokenKind, name: &str) -> String {
        let mut n = copy_name(name);
        while self.name_taken(kind, &n) {
            n = copy_name(&n);
        }
        n
    }

    /// Whether a local token of `kind`, any design system token, or an effect style uses
    /// `name`'s CSS name.
    fn name_taken(&self, kind: TokenKind, name: &str) -> bool {
        let slug = css_slug(name);
        self.effect_var_taken(&slug, "").is_some()
            || self
                .tokens
                .iter()
                .any(|t| (t.kind == kind || t.system.is_some()) && self.slug(t) == slug)
    }

    /// Capture tokens for pasting, here or in another project.
    pub fn copy(&self, ids: &[String]) -> TokenClip {
        let mut clip = TokenClip::default();
        for t in self.sorted() {
            if !ids.contains(&t.id) {
                continue;
            }
            let mut c = self.local_copy(t);
            c.modes = (self.choices().into_iter())
                .map(|m| (css_slug(&m.name), self.value_in(t, Some(&m.name)).clone()))
                .collect();
            clip.tokens.push(c);
            for v in std::iter::once(&t.value).chain(t.modes.values()) {
                if let TokenValue::Alias(a) = v
                    && !ids.contains(a)
                {
                    let values = (self.choices().into_iter())
                        .filter_map(|m| {
                            Some((css_slug(&m.name), self.resolve_in(a, Some(&m.name))?))
                        })
                        .collect();
                    clip.resolved.insert(a.clone(), values);
                    if let Some(target) = self.get(a) {
                        clip.targets
                            .insert(a.clone(), (target.kind, self.slug(target)));
                    }
                }
            }
        }
        clip
    }

    /// Add copies of `clip`'s tokens and return the new ids. Names that are taken get a copy
    /// suffix. An alias to a token outside the clip finds a token with the same kind and name
    /// here, else becomes the value it had when copied.
    pub fn paste(&mut self, clip: &TokenClip) -> Vec<String> {
        // Modes a frame here can pick, from a design system, that pasted values need.
        let needed: Vec<String> = (clip.tokens.iter())
            .flat_map(|t| t.modes.iter().filter(|(_, v)| **v != t.value))
            .map(|(slug, _)| slug.clone())
            .collect();
        self.promote_modes(&needed);
        let ids: HashMap<&str, String> = clip
            .tokens
            .iter()
            .map(|t| (t.id.as_str(), crate::project::new_id()))
            .collect();
        let mut out = Vec::new();
        for t in &clip.tokens {
            // An alias to a target left behind becomes the value it showed in that mode.
            let value_of = |v: &TokenValue, mode: &str| match v {
                TokenValue::Alias(a) => match ids.get(a.as_str()) {
                    Some(new) => TokenValue::Alias(new.clone()),
                    None => self.pasted_target(clip, a).unwrap_or_else(|| {
                        let values = clip.resolved.get(a).map_or(&[][..], |v| v.as_slice());
                        (values
                            .iter()
                            .find(|(m, _)| *m == css_slug(mode))
                            .or(values.first()))
                        .map_or_else(|| t.kind.default_value(), |(_, v)| v.clone())
                    }),
                },
                v => v.clone(),
            };
            let own = |m: &Mode| t.modes.get(&css_slug(&m.name)).unwrap_or(&t.value);
            let value = match self.modes.first() {
                Some(m) => value_of(own(m), &m.name),
                None => value_of(&t.value, ""),
            };
            let modes = (self.modes.iter().skip(1))
                .map(|m| (m.id.clone(), value_of(own(m), &m.name)))
                .filter(|(_, v)| *v != value)
                .collect();
            let name = if self.name_taken(t.kind, &t.name) {
                self.unused_name(t.kind, &t.name)
            } else {
                t.name.clone()
            };
            self.tokens.push(Token {
                id: ids[t.id.as_str()].clone(),
                name,
                value,
                modes,
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
        let t = self.of_kind(*kind).find(|t| self.slug(t) == *name)?;
        Some(TokenValue::Alias(t.id.clone()))
    }

    /// A token's value as CSS, in its first mode.
    pub fn css_value(&self, t: &Token) -> String {
        self.value_css(t.kind, &t.value)
    }

    /// A token's value as CSS in the mode named `mode`.
    pub fn css_value_in(&self, t: &Token, mode: Option<&str>) -> String {
        self.value_css(t.kind, self.value_in(t, mode))
    }

    /// `t`'s own values in the modes after its first, as (mode name, CSS).
    pub fn own_mode_values(&self, t: &Token) -> Vec<(String, String)> {
        (self.modes_of(t).iter().skip(1))
            .filter_map(|m| Some((m.name.clone(), self.value_css(t.kind, t.modes.get(&m.id)?))))
            .collect()
    }

    /// Include aliases whose targets change, since CSS resolves `var()` where it is set.
    fn varies_in<'a>(&self, tokens: &[&'a Token], mode: &str) -> Vec<&'a Token> {
        let m = Some(mode);
        (tokens.iter().copied())
            .filter(|t| {
                self.value_in(t, m) != self.value_in(t, None)
                    || self.resolve_in(&t.id, m) != self.resolve_in(&t.id, None)
            })
            .collect()
    }

    /// A value of a token of `kind` as CSS. An alias becomes `var(--target)`.
    pub fn value_css(&self, kind: TokenKind, value: &TokenValue) -> String {
        let num = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
        match value {
            TokenValue::Alias(a) => match self.get(a) {
                Some(target) => format!("var({})", self.css_name(target)),
                None => String::new(),
            },
            TokenValue::Color(c) => css::color_css(*c),
            TokenValue::Text(s) if s.contains(' ') => format!("\"{s}\""),
            TokenValue::Text(s) => s.clone(),
            TokenValue::Em(v) => format!("{}em", css::round_em(*v)),
            TokenValue::Number(v) => match kind {
                TokenKind::LineHeight | TokenKind::Opacity => format!("{}%", num(v * 100.0)),
                TokenKind::FontWeight => num(*v),
                _ => format!("{}px", num(*v)),
            },
        }
    }

    /// A stylesheet of `tokens`: `:root { ... }`, or `@theme { ... }` for Tailwind v4. Each
    /// other mode adds a `[data-theme="dark"] { ... }` block. Each mode's block sets every
    /// token that differs in any mode, so a frame of one mode inside another turns back.
    pub fn stylesheet(&self, tokens: &[&Token], tailwind: bool) -> String {
        self.stylesheet_with(tokens, &[], tailwind)
    }

    /// [`stylesheet`](Self::stylesheet) with the variables of `effects` after the tokens.
    pub fn stylesheet_with(
        &self,
        tokens: &[&Token],
        effects: &[&EffectStyleDef],
        tailwind: bool,
    ) -> String {
        /// A variable, its value in a mode, and whether any mode changes what it shows.
        type Decl<'a> = (String, Box<dyn Fn(Option<&str>) -> String + 'a>, bool);
        let others: Vec<&str> = (self.choices().into_iter().skip(1))
            .map(|m| m.name.as_str())
            .collect();
        let mut decls: Vec<Decl> = Vec::new();
        for t in tokens.iter().copied() {
            let varies = others.iter().any(|m| !self.varies_in(&[t], m).is_empty());
            let value = Box::new(move |mode: Option<&str>| self.css_value_in(t, mode));
            decls.push((self.css_name(t), value, varies));
        }
        for s in effects.iter().copied() {
            let varies = self.effect_style_varies(s);
            for (name, css) in self.effect_style_decls(s) {
                decls.push((name, Box::new(move |_| css.clone()), varies));
            }
        }
        let block = |selector: &str, varied: bool, mode: Option<&str>| {
            let mut s = format!("{selector} {{\n");
            for (name, value, _) in decls.iter().filter(|d| d.2 || !varied) {
                s.push_str(&format!("  {name}: {};\n", value(mode)));
            }
            s.push_str("}\n");
            s
        };
        let theme = |name: &str| format!("[data-theme=\"{}\"]", css_slug(name));
        let any_varied = decls.iter().any(|d| d.2);
        let first = theme(self.default_mode());
        let root = match (tailwind, any_varied) {
            (true, _) => "@theme".to_string(),
            (false, false) => ":root".to_string(),
            (false, true) => format!(":root,\n{first}"),
        };
        let mut s = block(&root, false, None);
        if !any_varied {
            return s;
        }
        if tailwind {
            s.push_str(&block(&first, true, None));
        }
        for name in others {
            s.push_str(&block(&theme(name), true, Some(name)));
        }
        s
    }

    /// Replace `var(--token)` in styles with the token's value. A style whose whole value is
    /// one variable also gets a [`css::TOKEN_MARKER`] entry, which binds the node's property
    /// when `css::apply` reaches it. So does each shadow in a list that names a color token.
    /// `effectStyle`, or an effect property whose whole value is an effect style's variable,
    /// becomes a [`css::EFFECT_STYLE_MARKER`] entry. Unknown variables fall back to their
    /// default, if any.
    pub fn expand(&self, styles: &mut Vec<(String, String)>) {
        self.expand_in(styles, None);
    }

    /// [`expand`](Self::expand) with each token's value in the mode named `mode`.
    pub fn expand_in(&self, styles: &mut Vec<(String, String)>, mode: Option<&str>) {
        self.expand_styles(styles, mode);
        let mut bound = Vec::new();
        for (k, v) in styles.iter_mut() {
            if k.starts_with(css::TOKEN_MARKER) {
                continue;
            }
            if let Some(marker) = self.effect_style_marker(k, v, mode) {
                *k = format!("{}{k}", css::EFFECT_STYLE_MARKER);
                *v = marker;
                continue;
            }
            if let Some(t) = var_name(v).and_then(|n| self.by_css_name(n))
                && let Some(lit) = self.resolve_in(&t.id, mode)
            {
                bound.push((format!("{}{k}", css::TOKEN_MARKER), t.id.clone()));
                *v = self.literal_css(t.kind, lit);
                continue;
            }
            if matches!(k.as_str(), "boxShadow" | "textShadow") {
                for (i, shadow) in css::split_top(v, ',').iter().enumerate() {
                    let color = (css::split_ws(shadow).iter())
                        .filter_map(|part| self.by_css_name(var_name(part)?))
                        .find(|t| t.kind == TokenKind::Color);
                    if let Some(t) = color {
                        bound.push((format!("{}{k}:{i}", css::TOKEN_MARKER), t.id.clone()));
                    }
                }
            }
            *v = self.expand_text(v, mode);
        }
        styles.extend(bound);
    }

    /// A literal for `css::apply`. Numbers keep full precision: the node must end up with
    /// exactly the token's value or the binding would count as edited.
    fn literal_css(&self, kind: TokenKind, value: TokenValue) -> String {
        match value {
            TokenValue::Color(c) => css::color_css(c),
            TokenValue::Text(s) => s,
            TokenValue::Em(v) => format!("{v}em"),
            TokenValue::Alias(_) => String::new(),
            TokenValue::Number(v) => match kind {
                TokenKind::LineHeight | TokenKind::Opacity | TokenKind::FontWeight => v.to_string(),
                _ => format!("{v}px"),
            },
        }
    }

    /// Replace every `var(...)` inside a longer value such as `1px solid var(--line)`.
    fn expand_text(&self, v: &str, mode: Option<&str>) -> String {
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
            let name = var_name(call);
            let lit = name
                .and_then(|n| self.by_css_name(n))
                .and_then(|t| Some(self.literal_css(t.kind, self.resolve_in(&t.id, mode)?)))
                .or_else(|| self.effect_var_literal(name?, mode));
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
        let systems = diff_list(&before.systems, &self.systems);
        let modes = diff_list(&before.modes, &self.modes);
        let tokens = diff_list(&before.tokens, &self.tokens);
        let styles = diff_list(&before.styles, &self.styles);
        let effects = diff_list(&before.effect_styles, &self.effect_styles);
        let components = diff_list(&before.components, &self.components);
        systems
            .into_iter()
            .map(TokenEdit::System)
            .chain(modes.into_iter().map(TokenEdit::Mode))
            .chain(tokens.into_iter().map(TokenEdit::Token))
            .chain(styles.into_iter().map(|e| TokenEdit::Style(Box::new(e))))
            .chain(effects.into_iter().map(TokenEdit::EffectStyle))
            .chain(components.into_iter().map(TokenEdit::Component))
            .collect()
    }

    /// Apply edits forward or back. Only the listed items change, so undoing on one page
    /// keeps token edits made since on another.
    pub fn apply_edits(&mut self, edits: &[TokenEdit], forward: bool) {
        let systems = edits.iter().filter_map(|e| match e {
            TokenEdit::System(e) => Some(e),
            _ => None,
        });
        apply_list(&mut self.systems, systems, forward);
        let modes = edits.iter().filter_map(|e| match e {
            TokenEdit::Mode(e) => Some(e),
            _ => None,
        });
        apply_list(&mut self.modes, modes, forward);
        let tokens = edits.iter().filter_map(|e| match e {
            TokenEdit::Token(e) => Some(e),
            _ => None,
        });
        apply_list(&mut self.tokens, tokens, forward);
        let styles = edits.iter().filter_map(|e| match e {
            TokenEdit::Style(e) => Some(&**e),
            _ => None,
        });
        apply_list(&mut self.styles, styles, forward);
        let effects = edits.iter().filter_map(|e| match e {
            TokenEdit::EffectStyle(e) => Some(e),
            _ => None,
        });
        apply_list(&mut self.effect_styles, effects, forward);
        let components = edits.iter().filter_map(|e| match e {
            TokenEdit::Component(e) => Some(e),
            _ => None,
        });
        apply_list(&mut self.components, components, forward);
    }

    // ------------------------------------------------------------------ design systems

    /// Tokens as a design system publishes them: the local ones, with aliases to another
    /// system's tokens replaced by the values they show, so systems never depend on each
    /// other.
    pub fn publishable(&self) -> Vec<Token> {
        let elsewhere = |a: &str| self.get(a).is_none_or(|x| x.system.is_some());
        self.tokens
            .iter()
            .filter(|t| t.system.is_none())
            .map(|t| self.flattened(t, elsewhere))
            .collect()
    }

    /// `published` as this project's read-only copies.
    fn linked(system: &str, published: &[Token]) -> Vec<Token> {
        let value = |v: &TokenValue| match v {
            TokenValue::Alias(a) => TokenValue::Alias(linked_id(system, a)),
            v => v.clone(),
        };
        published
            .iter()
            .map(|t| Token {
                id: linked_id(system, &t.id),
                value: value(&t.value),
                modes: (t.modes.iter())
                    .map(|(m, v)| (linked_id(system, m), value(v)))
                    .collect(),
                system: Some(system.to_string()),
                ..t.clone()
            })
            .collect()
    }

    /// Start using a design system. Fails, changing nothing, when the project already uses
    /// it or another system with the same prefix, or when one of its CSS names is taken.
    pub fn attach(
        &mut self,
        link: SystemLink,
        published: &[Token],
        styles: &[TextStyleDef],
        effects: &[EffectStyleDef],
    ) -> Result<(), String> {
        if self.system(&link.id).is_some() {
            return Err(format!(
                "This file already uses the {} design system.",
                link.name
            ));
        }
        if let Some(other) = self.systems.iter().find(|s| s.prefix == link.prefix) {
            return Err(format!(
                "This file already uses {}, which has the same prefix \"{}\".",
                other.name, link.prefix
            ));
        }
        let taken: HashSet<String> = self.tokens.iter().map(|t| self.slug(t)).collect();
        let mut clashes: Vec<String> = published
            .iter()
            .map(|t| format!("{}-{}", link.prefix, css_slug(&t.name)))
            .filter(|s| taken.contains(s))
            .map(|s| format!("--{s}"))
            .collect();
        clashes.extend(
            (published.iter())
                .map(|t| format!("{}-{}", link.prefix, css_slug(&t.name)))
                .filter(|s| self.effect_var_taken(s, "").is_some())
                .map(|s| format!("--{s}")),
        );
        clashes.extend(self.style_clashes(&link.prefix, styles));
        clashes.extend(self.effect_style_clashes(&link.prefix, effects));
        clashes.dedup();
        if !clashes.is_empty() {
            let more = clashes.len().saturating_sub(5);
            clashes.truncate(5);
            let mut list = clashes.join(", ");
            if more > 0 {
                list.push_str(&format!(" and {more} more"));
            }
            return Err(format!(
                "{} has tokens whose names this file already uses: {list}. Rename those first.",
                link.name
            ));
        }
        self.tokens.extend(Self::linked(&link.id, published));
        self.styles.extend(Self::linked_styles(&link.id, styles));
        self.effect_styles
            .extend(Self::linked_effect_styles(&link.id, effects));
        self.systems.push(link);
        Ok(())
    }

    /// Switch a design system to another published version. Tokens keep their place, new
    /// ones go at the end, and a local alias to a dropped token takes on the value it
    /// showed. Returns the dropped tokens and styles as (id, CSS name or `.class`). Layers
    /// bound to them keep their values; their bindings point at nothing and go when the layer
    /// is next edited, or come back if the update is undone.
    pub fn update_system(
        &mut self,
        link: SystemLink,
        published: &[Token],
        styles: &[TextStyleDef],
        effects: &[EffectStyleDef],
    ) -> Vec<(String, String)> {
        let Some(at) = self.systems.iter().position(|s| s.id == link.id) else {
            return Vec::new();
        };
        let new = Self::linked(&link.id, published);
        let ours = |t: &Token| t.system.as_deref() == Some(link.id.as_str());
        let gone: Vec<(String, String)> = self
            .tokens
            .iter()
            .filter(|t| ours(t) && !new.iter().any(|n| n.id == t.id))
            .map(|t| (t.id.clone(), self.css_name(t)))
            .collect();
        let ids: Vec<String> = gone.iter().map(|(id, _)| id.clone()).collect();
        self.remove(&ids);
        for t in &mut self.tokens {
            if let Some(n) = new.iter().find(|n| n.id == t.id) {
                *t = n.clone();
            }
        }
        let fresh: Vec<Token> = new
            .into_iter()
            .filter(|n| self.get(&n.id).is_none())
            .collect();
        self.tokens.extend(fresh);
        let mut gone = gone;
        gone.extend(self.replace_linked_styles(&link.id, Self::linked_styles(&link.id, styles)));
        let effects = Self::linked_effect_styles(&link.id, effects);
        gone.extend(self.replace_linked_effect_styles(&link.id, effects));
        self.systems[at] = link;
        gone
    }

    /// Stop using a design system. Local aliases to its tokens take on the values they
    /// showed. Returns the removed token and style ids.
    pub fn remove_system(&mut self, id: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .tokens
            .iter()
            .filter(|t| t.system.as_deref() == Some(id))
            .map(|t| t.id.clone())
            .collect();
        let mut styles = self.replace_linked_styles(id, Vec::new());
        styles.extend(self.replace_linked_effect_styles(id, Vec::new()));
        self.replace_components(id, Vec::new());
        self.remove(&ids);
        self.systems.retain(|s| s.id != id);
        ids.extend(styles.into_iter().map(|(id, _)| id));
        ids
    }

    /// Turn a design system's tokens into local ones that keep their CSS names, so bindings
    /// and `var()` references keep working. Its modes this file lacks become the file's, under
    /// the same ids so frames that picked them keep them.
    pub fn detach_system(&mut self, id: &str) {
        let theirs: Vec<Mode> = (self.system(id).into_iter().flat_map(|l| &l.modes))
            .cloned()
            .collect();
        let mut names: Vec<String> = theirs.iter().map(|m| m.name.clone()).collect();
        for t in self
            .tokens
            .iter()
            .filter(|t| t.system.as_deref() == Some(id))
        {
            names.extend(self.modes_needed(t));
        }
        self.promote_modes(&names);
        for m in &theirs {
            self.stand_in(m);
        }
        let local: Vec<Token> = self
            .tokens
            .iter()
            .filter(|t| t.system.as_deref() == Some(id))
            .map(|t| self.local_copy(t))
            .collect();
        for t in &mut self.tokens {
            if let Some(l) = local.iter().find(|l| l.id == t.id) {
                *t = l.clone();
            }
        }
        self.detach_styles(id);
        self.detach_effect_styles(id);
        // Removing the copies makes their instances act as plain frames.
        self.replace_components(id, Vec::new());
        self.systems.retain(|s| s.id != id);
    }
}

/// Replace linked items without reordering survivors. Append new items and return removed ones.
pub(super) fn replace_linked<T: Keyed>(
    list: &mut Vec<T>,
    ours: impl Fn(&T) -> bool,
    new: Vec<T>,
) -> Vec<T> {
    let has = |list: &[T], id: &str| list.iter().any(|t| t.key() == id);
    let (gone, kept): (Vec<T>, Vec<T>) = std::mem::take(list)
        .into_iter()
        .partition(|t| ours(t) && !has(&new, t.key()));
    *list = kept;
    for t in list.iter_mut() {
        if let Some(n) = new.iter().find(|n| n.key() == t.key()) {
            *t = n.clone();
        }
    }
    let fresh: Vec<T> = (new.into_iter()).filter(|n| !has(list, n.key())).collect();
    list.extend(fresh);
    gone
}

/// Fold `newer` into `older`, keeping the oldest before-state of each item.
pub fn merge_edits(older: &mut Vec<TokenEdit>, newer: Vec<TokenEdit>) {
    for e in newer {
        if !older.iter_mut().any(|x| x.absorb(&e)) {
            older.push(e);
        }
    }
    older.retain(TokenEdit::changes);
}

pub(super) fn check_description(d: Option<&str>) -> Result<(), String> {
    match d {
        Some(d) if d.chars().count() > MAX_DESCRIPTION => Err(format!(
            "Token descriptions can be at most {MAX_DESCRIPTION} characters."
        )),
        _ => Ok(()),
    }
}

/// A node property a token can own. Saved as its [`key`](TokenProp::key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TokenProp {
    /// The top fill layer.
    Fill,
    Stroke,
    StrokeWidth,
    Radius,
    Opacity,
    FontFamily,
    FontSize,
    FontWeight,
    LineHeight,
    LetterSpacing,
    Gap,
    /// All four sides.
    Padding,
    /// The color of the shadow at this index of `style.effects`. Change the list with
    /// [`edit_effects`](super::effects::edit_effects) so the index stays right.
    EffectColor(usize),
}

const EFFECT_COLOR_KEY: &str = "effectColor.";

impl Serialize for TokenProp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for TokenProp {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        TokenProp::from_key(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown token property {s:?}")))
    }
}

impl TokenProp {
    pub fn key(self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed(match self {
            TokenProp::Fill => "fill",
            TokenProp::Stroke => "stroke",
            TokenProp::StrokeWidth => "strokeWidth",
            TokenProp::Radius => "radius",
            TokenProp::Opacity => "opacity",
            TokenProp::FontFamily => "fontFamily",
            TokenProp::FontSize => "fontSize",
            TokenProp::FontWeight => "fontWeight",
            TokenProp::LineHeight => "lineHeight",
            TokenProp::LetterSpacing => "letterSpacing",
            TokenProp::Gap => "gap",
            TokenProp::Padding => "padding",
            TokenProp::EffectColor(i) => return format!("{EFFECT_COLOR_KEY}{i}").into(),
        })
    }

    pub fn from_key(key: &str) -> Option<TokenProp> {
        if let Some(i) = key.strip_prefix(EFFECT_COLOR_KEY) {
            return i.parse().ok().map(TokenProp::EffectColor);
        }
        TokenProp::ALL.into_iter().find(|p| p.key() == key)
    }

    pub const ALL: [TokenProp; 12] = [
        TokenProp::Fill,
        TokenProp::Stroke,
        TokenProp::StrokeWidth,
        TokenProp::Radius,
        TokenProp::Opacity,
        TokenProp::FontFamily,
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
            TokenProp::Fill | TokenProp::Stroke | TokenProp::EffectColor(_) => TokenKind::Color,
            TokenProp::StrokeWidth => TokenKind::BorderWidth,
            TokenProp::Radius => TokenKind::Radius,
            TokenProp::Opacity => TokenKind::Opacity,
            TokenProp::FontFamily => TokenKind::FontFamily,
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
            TokenProp::StrokeWidth => "stroke width",
            TokenProp::Radius => "radius",
            TokenProp::Opacity => "opacity",
            TokenProp::FontFamily => "font",
            TokenProp::FontSize => "font size",
            TokenProp::FontWeight => "bold",
            TokenProp::LineHeight => "line height",
            TokenProp::LetterSpacing => "letter spacing",
            TokenProp::Gap | TokenProp::Padding => "layout",
            TokenProp::EffectColor(_) => "effects",
        }
    }

    /// The CSS property that carries this on `n`.
    pub fn css_key(self, n: &Node) -> &'static str {
        match self {
            TokenProp::Fill if matches!(n.kind, NodeKind::Text { .. }) => "color",
            TokenProp::Fill => "backgroundColor",
            TokenProp::Stroke => "borderColor",
            TokenProp::StrokeWidth => "borderWidth",
            TokenProp::Radius => "borderRadius",
            TokenProp::Opacity => "opacity",
            TokenProp::FontFamily => "fontFamily",
            TokenProp::FontSize => "fontSize",
            TokenProp::FontWeight => "fontWeight",
            TokenProp::LineHeight => "lineHeight",
            TokenProp::LetterSpacing => "letterSpacing",
            TokenProp::Gap => "gap",
            TokenProp::Padding => "padding",
            TokenProp::EffectColor(_) => css::shadow_key(n),
        }
    }

    /// The property a CSS key binds on `n`, if it binds one. `boxShadow:1` is the color of
    /// the second shadow in the list.
    pub fn from_css_key(key: &str, n: &Node) -> Option<TokenProp> {
        if let Some((k, i)) = key.split_once(':') {
            let nth = css::shadow_slots(n)
                .get(i.parse::<usize>().ok()?)
                .copied()?;
            return (k == css::shadow_key(n)).then_some(TokenProp::EffectColor(nth));
        }
        let text = n.kind.text().is_some();
        // A sticky's fill is its paper; `color` is the writing on it.
        let sticky = matches!(n.kind, NodeKind::Sticky { .. });
        Some(match key {
            "color" | "fill" if text && !sticky => TokenProp::Fill,
            "backgroundColor" | "background" if sticky => TokenProp::Fill,
            "backgroundColor" | "background" | "fill" if !text => TokenProp::Fill,
            "borderColor" | "stroke" => TokenProp::Stroke,
            "borderWidth" | "strokeWidth" => TokenProp::StrokeWidth,
            "borderRadius" => TokenProp::Radius,
            "opacity" => TokenProp::Opacity,
            "fontFamily" if text => TokenProp::FontFamily,
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
            TokenProp::Fill
            | TokenProp::Stroke
            | TokenProp::StrokeWidth
            | TokenProp::Radius
            | TokenProp::Opacity => true,
            TokenProp::FontSize => n.kind.font_size().is_some(),
            TokenProp::FontFamily
            | TokenProp::FontWeight
            | TokenProp::LineHeight
            | TokenProp::LetterSpacing => is_text,
            TokenProp::Gap | TokenProp::Padding => n.layout.is_some(),
            TokenProp::EffectColor(i) => {
                (n.style.effects.get(i)).is_some_and(|e| e.kind.is_shadow())
            }
        }
    }

    /// Does `n` already show `v`?
    pub fn holds(self, n: &Node, v: &TokenValue) -> bool {
        match (self, v) {
            (TokenProp::Fill, TokenValue::Color(c)) => n.style.fill() == Some(*c),
            (TokenProp::Stroke, TokenValue::Color(c)) => n.style.stroke == Some(*c),
            (TokenProp::StrokeWidth, TokenValue::Number(w)) => n.style.stroke_width == *w,
            (TokenProp::Radius, TokenValue::Number(r)) => n.style.corner_radii() == [*r; 4],
            (TokenProp::Opacity, TokenValue::Number(o)) => n.style.opacity == *o,
            (TokenProp::FontFamily, TokenValue::Text(f)) => {
                n.text_style.family.as_deref().unwrap_or("Inter") == f
            }
            (TokenProp::FontSize, TokenValue::Number(s)) => n.kind.font_size() == Some(*s),
            (TokenProp::FontWeight, TokenValue::Number(w)) => {
                n.text_style.base_weight() == *w as u16
            }
            (TokenProp::LineHeight, TokenValue::Number(l)) => n.text_style.line_height == Some(*l),
            (TokenProp::LetterSpacing, TokenValue::Number(s)) => {
                n.text_style.letter_spacing == LetterSpacing::Px(*s)
            }
            (TokenProp::LetterSpacing, TokenValue::Em(em)) => {
                n.text_style.letter_spacing == LetterSpacing::Em { em: *em }
            }
            (TokenProp::EffectColor(i), TokenValue::Color(c)) => {
                (n.style.effects.get(i)).is_some_and(|e| e.kind.is_shadow() && e.color == *c)
            }
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
            (TokenProp::StrokeWidth, TokenValue::Number(w)) => n.style.stroke_width = *w,
            (TokenProp::Radius, TokenValue::Number(r)) => {
                n.style.corner_radius = *r;
                n.style.radii = None;
            }
            (TokenProp::Opacity, TokenValue::Number(o)) => n.style.opacity = *o,
            (TokenProp::FontFamily, TokenValue::Text(f)) => n.text_style.family = Some(f.clone()),
            (TokenProp::FontSize, TokenValue::Number(s)) => {
                if let NodeKind::Text { font_size, .. } | NodeKind::Sticky { font_size, .. } =
                    &mut n.kind
                {
                    *font_size = *s;
                }
            }
            (TokenProp::FontWeight, TokenValue::Number(w)) => n.text_style.set_weight(*w as u16),
            (TokenProp::LineHeight, TokenValue::Number(l)) => n.text_style.line_height = Some(*l),
            (TokenProp::LetterSpacing, TokenValue::Number(s)) => {
                n.text_style.letter_spacing = LetterSpacing::Px(*s)
            }
            (TokenProp::LetterSpacing, TokenValue::Em(em)) => {
                n.text_style.letter_spacing = LetterSpacing::Em { em: *em }
            }
            (TokenProp::EffectColor(i), TokenValue::Color(c)) => {
                if let Some(e) = n.style.effects.get_mut(i) {
                    e.color = *c;
                }
            }
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
    fn font_family_tokens_set_the_family() {
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
        assert_eq!(
            TokenProp::from_css_key("fontFamily", &n),
            Some(TokenProp::FontFamily)
        );
        let inter = TokenValue::Text("Inter".into());
        let lora = TokenValue::Text("Lora".into());
        assert!(TokenProp::FontFamily.holds(&n, &inter));
        TokenProp::FontFamily.write(&mut n, &lora);
        assert_eq!(n.text_style.family.as_deref(), Some("Lora"));
        assert!(TokenProp::FontFamily.holds(&n, &lora));
    }

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
            s.parse_value(TokenKind::LetterSpacing, "-0.025em"),
            Ok(TokenValue::Em(-0.025))
        );
        assert_eq!(
            s.parse_value(TokenKind::FontFamily, "'Inter Tight'"),
            Ok(TokenValue::Text("Inter Tight".into()))
        );
        for (k, v) in [
            (TokenKind::Color, "nope"),
            (TokenKind::Opacity, "1.5"),
            (TokenKind::Radius, "-4px"),
            (TokenKind::BorderWidth, "-1px"),
            (TokenKind::FontSize, "0"),
            (TokenKind::LetterSpacing, "1e308em"),
            (TokenKind::LetterSpacing, "em"),
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

    /// A set using a system with `prefix` and one color token, `name` (#ffffff).
    fn using(prefix: &str, name: &str) -> TokenSet {
        let mut src = TokenSet::default();
        src.create(TokenKind::Color, name, "#ffffff", None).unwrap();
        let link = SystemLink {
            id: prefix.into(),
            name: prefix.into(),
            prefix: prefix.into(),
            release: 1,
            description: None,
            notes: None,
            modes: Vec::new(),
        };
        let mut s = TokenSet::default();
        s.attach(link, &src.publishable(), &[], &[]).unwrap();
        s
    }

    #[test]
    fn new_tokens_skip_design_system_names() {
        let mut s = using("color", "2");
        s.add_default(TokenKind::Color);
        let second = s.add_default(TokenKind::Color);
        assert_eq!(s.css_name(s.get(&second).unwrap()), "--color-3");
    }

    #[test]
    fn copied_aliases_to_a_design_system_token_match_by_css_name() {
        let mut s = using("acme", "primary");
        let cta = s
            .create(TokenKind::Color, "cta", "var(--acme-primary)", None)
            .unwrap();
        let clip = s.copy(&[cta]);

        let mut unrelated = TokenSet::default();
        unrelated
            .create(TokenKind::Color, "primary", "#000000", None)
            .unwrap();
        unrelated.paste(&clip);
        assert_eq!(
            unrelated.by_css_name("--cta").unwrap().value,
            TokenValue::Color(Color::rgb(255, 255, 255))
        );

        let mut same = using("acme", "primary");
        same.paste(&clip);
        let target = same.by_css_name("--acme-primary").unwrap().id.clone();
        assert_eq!(
            same.by_css_name("--cta").unwrap().value,
            TokenValue::Alias(target)
        );
    }

    #[test]
    fn paste_renames_on_collision_with_a_design_system_token() {
        let mut s = using("acme", "primary");
        let mut other = TokenSet::default();
        let id = other
            .create(TokenKind::Color, "acme-primary", "#000000", None)
            .unwrap();
        let spacing = other
            .create(TokenKind::Spacing, "acme-primary", "8px", None)
            .unwrap();
        let pasted = s.paste(&other.copy(&[id, spacing]));
        for p in &pasted {
            assert_eq!(s.css_name(s.get(p).unwrap()), "--acme-primary-copy");
        }
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
        assert!(matches!(&edits[0], TokenEdit::Token(e) if e.id == a));

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

    const WHITE: TokenValue = TokenValue::Color(Color::WHITE);
    const BLACK: TokenValue = TokenValue::Color(Color::BLACK);

    fn themed() -> (TokenSet, String, String, String) {
        let mut s = TokenSet::default();
        let dark = vec![("dark".to_string(), "#000000".to_string())];
        let bg = (s.create_in_modes(TokenKind::Color, "bg", "#ffffff", &dark, None)).unwrap();
        let surface = s
            .create(TokenKind::Color, "surface", "var(--bg)", None)
            .unwrap();
        let line = s.create(TokenKind::Color, "line", "#888888", None).unwrap();
        (s, bg, surface, line)
    }

    #[test]
    fn aliases_resolve_in_the_same_mode() {
        let (s, _, surface, line) = themed();
        assert_eq!(s.resolve_in(&surface, Some("Dark")), Some(BLACK));
        assert_eq!(s.resolve_in(&surface, None), Some(WHITE));
        let grey = s.resolve(&line);
        assert_eq!(
            s.resolve_in(&line, Some("Dark")),
            grey,
            "one value serves every mode"
        );
    }

    #[test]
    fn cycles_are_checked_per_mode() {
        let (mut s, bg, _, line) = themed();
        s.update_in_mode(&line, "dark", Some("var(--bg)")).unwrap();
        // In Dark, --bg -> --line -> --bg.
        let err = s
            .update_in_mode(&bg, "dark", Some("var(--line)"))
            .unwrap_err();
        assert!(err.contains("Circular"), "{err}");
        // Light --bg aliasing --line is fine: Dark keeps its own value.
        s.update(&bg, None, Some("var(--line)"), None).unwrap();
    }

    #[test]
    fn clearing_a_mode_value_cannot_make_a_cycle() {
        let (mut s, bg, _, line) = themed();
        s.update(&line, None, Some("var(--bg)"), None).unwrap();
        s.update_in_mode(&line, "dark", Some("#000000")).unwrap();
        s.update_in_mode(&bg, "dark", Some("var(--line)")).unwrap();
        // Dark --line would follow its first value, --bg, which aliases --line in Dark.
        assert!(s.update_in_mode(&line, "dark", None).is_err());
        assert_eq!(s.resolve_in(&line, Some("Dark")), Some(BLACK));
    }

    #[test]
    fn stylesheets_add_a_block_for_each_other_mode() {
        let (s, ..) = themed();
        let css = s.stylesheet(&s.sorted(), false);
        assert!(
            css.starts_with(":root,\n[data-theme=\"light\"] {\n"),
            "{css}"
        );
        let (_, dark) = css
            .split_once("[data-theme=\"dark\"] {\n")
            .expect("a dark block");
        assert!(dark.contains("--bg: #000000;"), "{css}");
        assert!(
            dark.contains("--surface: var(--bg);"),
            "an alias whose target changes is set again where the mode applies: {css}"
        );
        assert!(!dark.contains("--line"), "{css}");
    }

    #[test]
    fn every_mode_block_resets_tokens_that_vary() {
        let (mut s, ..) = themed();
        s.add_mode();
        let css = s.stylesheet(&s.sorted(), false);
        let (_, third) = css
            .split_once("[data-theme=\"mode-3\"] {\n")
            .expect("{css}");
        assert!(
            third.contains("--bg: #ffffff;"),
            "a Mode 3 frame inside Dark: {css}"
        );
    }

    /// A Dark-first set with `--bg` black in Dark and white in Light.
    fn dark_first() -> (TokenSet, String) {
        let mut s =
            TokenSet::default().with_modes(vec![Mode::new("d", "Dark"), Mode::new("l", "Light")]);
        let light = vec![("l".to_string(), "#ffffff".to_string())];
        let bg = s.create_in_modes(TokenKind::Color, "bg", "#000000", &light, None);
        (s, bg.unwrap())
    }

    #[test]
    fn mode_order_doesnt_change_pasted_or_detached_values() {
        let (src, bg) = dark_first();
        let mut pasted = TokenSet::default();
        let new = pasted.paste(&src.copy(std::slice::from_ref(&bg)));
        assert_eq!(pasted.resolve_in(&new[0], Some("Light")), Some(WHITE));
        assert_eq!(pasted.resolve_in(&new[0], Some("Dark")), Some(BLACK));

        let link = SystemLink {
            id: "s".into(),
            name: "S".into(),
            prefix: "s".into(),
            release: 1,
            description: None,
            notes: None,
            modes: vec![Mode::new("s/d", "Dark"), Mode::new("s/l", "Light")],
        };
        let mut used = TokenSet::default();
        used.attach(link, &src.publishable(), &[], &[]).unwrap();
        used.detach_system("s");
        let id = linked_id("s", &bg);
        assert_eq!(used.resolve_in(&id, Some("Light")), Some(WHITE));
        assert_eq!(used.resolve_in(&id, Some("Dark")), Some(BLACK));
    }

    /// A set using system `s`, whose modes are `names`, with `--s-bg` white and red in the
    /// last mode.
    fn using_modes(names: &[&str]) -> TokenSet {
        let mut src = TokenSet::default().with_modes(
            (names.iter().enumerate())
                .map(|(i, n)| Mode::new(&i.to_string(), n))
                .collect(),
        );
        let last = (names.len() - 1).to_string();
        let red = vec![(last, "#ff0000".to_string())];
        (src.create_in_modes(TokenKind::Color, "bg", "#ffffff", &red, None)).unwrap();
        let link = SystemLink {
            id: "s".into(),
            name: "S".into(),
            prefix: "s".into(),
            release: 1,
            description: None,
            notes: None,
            modes: (src.modes().iter())
                .map(|m| Mode::new(&linked_id("s", &m.id), &m.name))
                .collect(),
        };
        let mut s = TokenSet::default();
        s.attach(link, &src.publishable(), &[], &[]).unwrap();
        s
    }

    #[test]
    fn copies_keep_modes_only_a_design_system_has() {
        let s = using_modes(&["Light", "Dark", "Contrast"]);
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        let mut other = TokenSet::default()
            .with_modes(vec![Mode::new("l", "Light"), Mode::new("c", "Contrast")]);
        let new = other.paste(&s.copy(&[bg]));
        let red = TokenValue::Color(Color::rgb(255, 0, 0));
        assert_eq!(other.resolve_in(&new[0], Some("Contrast")), Some(red));
    }

    /// A system with one mode, `name`, and no tokens.
    fn bare_system(id: &str, name: &str) -> SystemLink {
        SystemLink {
            id: id.into(),
            name: id.into(),
            prefix: id.into(),
            release: 1,
            description: None,
            notes: None,
            modes: vec![Mode::new(&format!("{id}/m"), name)],
        }
    }

    #[test]
    fn a_live_system_mode_wins_over_a_detached_one() {
        let mut s = TokenSet::default();
        s.attach(bare_system("x", "Contrast"), &[], &[], &[])
            .unwrap();
        s.rename_mode("dark", "Contrast").unwrap();
        s.detach_system("x");
        s.rename_mode("dark", "Dim").unwrap();
        s.attach(bare_system("x", "Contrast"), &[], &[], &[])
            .unwrap();
        assert_eq!(s.mode_name("x/m"), Some("Contrast"));
    }

    #[test]
    fn copies_keep_modes_another_system_brings() {
        let mut s = using_modes(&["Contrast", "Light"]);
        s.attach(bare_system("t", "Sepia"), &[], &[], &[]).unwrap();
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        let copy = s.duplicate(std::slice::from_ref(&bg));
        assert_eq!(s.resolve_in(&copy[0], Some("Sepia")), Some(WHITE));
        s.detach_system("s");
        assert_eq!(s.resolve_in(&bg, Some("Sepia")), Some(WHITE));
    }

    #[test]
    fn a_promoted_mode_stays_apart_from_the_systems() {
        let mut s = using_modes(&["Contrast", "Light"]);
        s.attach(bare_system("t", "Sepia"), &[], &[], &[]).unwrap();
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        s.duplicate(std::slice::from_ref(&bg));
        let sepia = s
            .modes()
            .iter()
            .find(|m| m.name == "Sepia")
            .unwrap()
            .id
            .clone();
        s.rename_mode(&sepia, "Dim").unwrap();
        assert_eq!(s.mode_name("t/m"), Some("Sepia"));
        s.detach_system("t");
        assert_eq!(
            s.mode_name("t/m"),
            Some("Sepia"),
            "the detached copy stands in"
        );
    }

    #[test]
    fn one_local_mode_stands_in_for_a_system_mode() {
        let mut s = using_modes(&["Contrast", "Light"]);
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        s.duplicate(std::slice::from_ref(&bg));
        let promoted = s
            .modes()
            .iter()
            .find(|m| m.name == "Contrast")
            .unwrap()
            .id
            .clone();
        s.rename_mode(&promoted, "Dim").unwrap();
        let contrast = s.add_mode();
        s.rename_mode(&contrast, "Contrast").unwrap();
        s.detach_system("s");
        assert_eq!(s.mode_name("s/0"), Some("Contrast"));
        let owners = s
            .modes()
            .iter()
            .filter(|m| m.aka.iter().any(|a| a == "s/0"));
        assert_eq!(owners.count(), 1);
    }

    #[test]
    fn duplicating_keeps_a_system_only_first_mode() {
        let mut s = using_modes(&["Contrast", "Light"]);
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        let copy = s.duplicate(std::slice::from_ref(&bg));
        assert_eq!(s.resolve_in(&copy[0], Some("Contrast")), Some(WHITE));
        assert_eq!(s.resolve_in(&bg, Some("Contrast")), Some(WHITE));
    }

    #[test]
    fn copies_in_the_same_file_keep_a_system_only_mode() {
        let mut s = using_modes(&["Light", "Dark", "Contrast"]);
        let bg = s.by_css_name("--s-bg").unwrap().id.clone();
        let red = Some(TokenValue::Color(Color::rgb(255, 0, 0)));
        let pasted = s.paste(&s.copy(std::slice::from_ref(&bg)));
        let duplicated = s.duplicate(std::slice::from_ref(&bg));
        assert_eq!(s.resolve_in(&pasted[0], Some("Contrast")), red);
        assert_eq!(s.resolve_in(&duplicated[0], Some("Contrast")), red);
        assert_eq!(s.modes().len(), 3, "Contrast became the file's own");
    }

    #[test]
    fn mode_names_match_like_data_theme_values() {
        let mut s = using_modes(&["light", "dark"]);
        let dark = vec![("dark".to_string(), "#000000".to_string())];
        (s.create_in_modes(TokenKind::Color, "fg", "#ffffff", &dark, None)).unwrap();
        assert_eq!(s.choices().len(), 2, "one Dark, not two");
        let css = s.stylesheet(&s.sorted(), false);
        assert_eq!(css.matches("[data-theme=\"dark\"]").count(), 1, "{css}");
        let sys = s.by_css_name("--s-bg").unwrap().id.clone();
        let red = TokenValue::Color(Color::rgb(255, 0, 0));
        assert_eq!(s.resolve_in(&sys, Some("Dark")), Some(red));
    }

    #[test]
    fn removing_the_first_mode_keeps_what_the_others_show() {
        let (mut s, bg, ..) = themed();
        let dim = s.add_mode();
        s.delete_mode("light").unwrap();
        assert_eq!(s.resolve(&bg), Some(BLACK), "Dark is the default now");
        assert_eq!(s.resolve_in(&bg, Some("Mode 3")), Some(WHITE));
        s.delete_mode(&dim).unwrap();
        assert!(s.delete_mode("dark").is_err(), "a file keeps one mode");
    }

    #[test]
    fn deleting_an_alias_target_keeps_each_mode() {
        let (mut s, bg, surface, _) = themed();
        s.delete(std::slice::from_ref(&bg));
        assert_eq!(s.resolve_in(&surface, Some("Dark")), Some(BLACK));
        assert_eq!(s.resolve(&surface), Some(WHITE));
    }

    #[test]
    fn a_pasted_alias_without_its_target_keeps_each_mode() {
        let (s, _, surface, _) = themed();
        let clip = s.copy(std::slice::from_ref(&surface));
        let mut other = TokenSet::default();
        let new = other.paste(&clip);
        assert_eq!(other.resolve(&new[0]), Some(WHITE));
        assert_eq!(other.resolve_in(&new[0], Some("Dark")), Some(BLACK));
    }

    #[test]
    fn pasted_tokens_match_modes_by_name() {
        let (s, bg, ..) = themed();
        let clip = s.copy(std::slice::from_ref(&bg));
        let mut other =
            TokenSet::default().with_modes(vec![Mode::new("l2", "Light"), Mode::new("d2", "Dark")]);
        let new = other.paste(&clip);
        assert_eq!(other.resolve_in(&new[0], Some("Dark")), Some(BLACK));
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
