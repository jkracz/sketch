//! Design tokens in the W3C Design Tokens Community Group format (2025.10), the
//! `.tokens.json` files that Tokens Studio, Style Dictionary and others read and write.
//!
//! What the format has no place for goes under `$extensions["sketch-rs"]`: the token type
//! when `$type` doesn't settle it, the exact name when the group path doesn't give it back,
//! and values in other modes. A file exported here imports back unchanged, and other tools
//! ignore the extra data. Text and effect styles aren't included.

use crate::model::tokens::css_slug;
use crate::model::{Color, Token, TokenKind, TokenSet, TokenValue};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

/// The `$extensions` key for Sketch's own data.
pub const EXT: &str = "sketch-rs";

/// The DTCG `$type` a token of `kind` is written as.
fn dtcg_type(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Color => "color",
        TokenKind::FontFamily => "fontFamily",
        TokenKind::FontWeight => "fontWeight",
        TokenKind::LineHeight | TokenKind::Opacity => "number",
        _ => "dimension",
    }
}

/// A group path from a token name. DTCG uses `.` between groups, most design tools `/`, and
/// CSS names `-`, which is also what import joins a path with.
fn segments(name: &str) -> Vec<String> {
    let s: Vec<String> = name
        .split(['/', '.', '-'])
        .map(|s| {
            s.replace(['{', '}'], "")
                .trim()
                .trim_start_matches('$')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect();
    if s.is_empty() {
        vec![css_slug(name)]
    } else {
        s
    }
}

fn number(v: f64) -> Value {
    let v = (v * 1e6).round() / 1e6;
    if v.fract() == 0.0 && v.abs() < 1e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn color_json(c: Color) -> Value {
    let ch = |v: u8| number(v as f64 / 255.0);
    let mut v = json!({
        "colorSpace": "srgb",
        "components": [ch(c.r), ch(c.g), ch(c.b)],
        "hex": format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
    });
    if c.a != 255 {
        v["alpha"] = ch(c.a);
    }
    v
}

fn literal_json(kind: TokenKind, v: &TokenValue) -> Value {
    match v {
        TokenValue::Color(c) => color_json(*c),
        TokenValue::Text(s) => json!(s),
        // DTCG dimensions are px or rem. em letter spacing has no spec form.
        TokenValue::Em(e) => json!({"value": number(*e), "unit": "em"}),
        TokenValue::Number(n) if dtcg_type(kind) == "dimension" => {
            json!({"value": number(*n), "unit": "px"})
        }
        TokenValue::Number(n) => number(*n),
        TokenValue::Alias(_) => Value::Null,
    }
}

/// `tokens` as a DTCG document. `mode` writes that mode's values as `$value` and leaves the
/// other modes out, for tools that take one file per theme. Without it, `$value` is the
/// first mode's and the others go in `$extensions`. An alias to a token outside `tokens`
/// becomes the value it resolves to.
pub fn export(set: &TokenSet, tokens: &[&Token], mode: Option<&str>) -> Value {
    let mut paths: HashMap<&str, Vec<String>> = HashMap::new();
    let mut leaves: HashSet<Vec<String>> = HashSet::new();
    let mut groups: HashSet<Vec<String>> = HashSet::new();
    for t in tokens {
        let name = match t.system {
            Some(_) => set.slug(t),
            None => t.name.clone(),
        };
        let mut path = segments(&name);
        let last = path.len() - 1;
        let base = path[last].clone();
        let clashes = |p: &Vec<String>| {
            leaves.contains(p)
                || groups.contains(p)
                || (1..p.len()).any(|i| leaves.contains(&p[..i]))
        };
        let mut n = 2;
        while clashes(&path) {
            path[last] = format!("{base} {n}");
            n += 1;
        }
        for i in 1..path.len() {
            groups.insert(path[..i].to_vec());
        }
        leaves.insert(path.clone());
        paths.insert(&t.id, path);
    }
    let value_at = |t: &Token, m: Option<&str>| match set.value_in(t, m) {
        TokenValue::Alias(a) if paths.contains_key(a.as_str()) => {
            json!(format!("{{{}}}", paths[a.as_str()].join(".")))
        }
        TokenValue::Alias(_) => literal_json(
            t.kind,
            &set.resolve_in(&t.id, m).unwrap_or(t.kind.default_value()),
        ),
        v => literal_json(t.kind, v),
    };
    let others: Vec<&str> = match mode {
        Some(_) => Vec::new(),
        None => (set.choices().into_iter().skip(1))
            .map(|m| m.name.as_str())
            .collect(),
    };
    let mut root = Map::new();
    for t in tokens {
        let path = &paths[t.id.as_str()];
        let mut tok = Map::new();
        tok.insert("$type".into(), json!(dtcg_type(t.kind)));
        let value = value_at(t, mode);
        tok.insert("$value".into(), value.clone());
        if let Some(d) = &t.description {
            tok.insert("$description".into(), json!(d));
        }
        let mut ext = Map::new();
        if dtcg_type(t.kind) != t.kind.key() {
            ext.insert("type".into(), json!(t.kind.key()));
        }
        let name = match t.system {
            Some(_) => set.slug(t),
            None => t.name.clone(),
        };
        if path.join("-") != name {
            ext.insert("name".into(), json!(name));
        }
        let modes: Map<String, Value> = (others.iter())
            .map(|m| (m.to_string(), value_at(t, Some(m))))
            .filter(|(_, v)| *v != value)
            .collect();
        if !modes.is_empty() {
            ext.insert("modes".into(), Value::Object(modes));
        }
        if !ext.is_empty() {
            tok.insert("$extensions".into(), json!({ EXT: ext }));
        }
        let mut group = &mut root;
        for seg in &path[..path.len() - 1] {
            group = group
                .entry(seg.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("paths never put a token where a group is");
        }
        group.insert(path[path.len() - 1].clone(), Value::Object(tok));
    }
    if !others.is_empty() {
        let names: Vec<&str> = set.choices().iter().map(|m| m.name.as_str()).collect();
        root.insert("$extensions".into(), json!({ EXT: {"modes": names} }));
    }
    Value::Object(root)
}

/// What [`import`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// CSS names of the tokens it added.
    pub created: Vec<String>,
    /// CSS names of existing tokens it changed.
    pub updated: Vec<String>,
    /// Tokens that already had the file's values.
    pub unchanged: usize,
    /// Tokens left out, as (DTCG path, reason).
    pub skipped: Vec<(String, String)>,
    /// Modes it added to the file.
    pub modes_added: Vec<String>,
}

impl Report {
    /// One line for the status bar.
    pub fn summary(&self) -> String {
        let mut parts = vec![
            format!("{} created", self.created.len()),
            format!("{} updated", self.updated.len()),
        ];
        if self.unchanged > 0 {
            parts.push(format!("{} unchanged", self.unchanged));
        }
        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }
        format!("Imported tokens: {}", parts.join(", "))
    }
}

/// A token read from the file.
struct Entry {
    /// The DTCG path, dot-separated, as aliases name it.
    path: String,
    name: String,
    kind: Result<TokenKind, String>,
    /// `$value`, then other modes' values by mode name.
    values: Vec<(Option<String>, Value)>,
    description: Option<String>,
}

fn alias_path(v: &Value) -> Option<&str> {
    let s = v.as_str()?.trim();
    s.strip_prefix('{')?.strip_suffix('}')
}

/// Collect the tokens under `group`. `$type` on a group applies to the tokens inside it.
fn collect(group: &Map<String, Value>, path: &[String], ty: Option<&str>, out: &mut Vec<Entry>) {
    let ty = group.get("$type").and_then(Value::as_str).or(ty);
    for (key, v) in group {
        let Some(obj) = v.as_object() else { continue };
        let path: Vec<String> = match key.as_str() {
            "$root" => path.to_vec(),
            k if k.starts_with('$') => continue,
            k => path.iter().cloned().chain([k.to_string()]).collect(),
        };
        if path.is_empty() {
            continue;
        }
        if !obj.contains_key("$value") {
            collect(obj, &path, ty, out);
            continue;
        }
        let ext = obj
            .get("$extensions")
            .and_then(|e| e.get(EXT))
            .and_then(Value::as_object);
        let ext_str = |k: &str| ext.and_then(|e| e.get(k)).and_then(Value::as_str);
        let ty = obj.get("$type").and_then(Value::as_str).or(ty);
        let kind = match (ext_str("type"), ty) {
            (Some(k), _) => TokenKind::from_key(k).ok_or(format!("Unknown Sketch type \"{k}\".")),
            (None, Some(t)) => kind_for(t, &path),
            // An alias without a type takes its target's, which is looked up later.
            (None, None) if alias_path(&obj["$value"]).is_some() => Err(String::new()),
            (None, None) => Err("No `$type` on the token or its groups.".into()),
        };
        let mut values = vec![(None, obj["$value"].clone())];
        if let Some(modes) = ext.and_then(|e| e.get("modes")).and_then(Value::as_object) {
            values.extend(modes.iter().map(|(m, v)| (Some(m.clone()), v.clone())));
        }
        out.push(Entry {
            path: path.join("."),
            name: ext_str("name").map_or_else(|| path.join("-"), String::from),
            kind,
            values,
            description: obj
                .get("$description")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .map(|d| {
                    d.chars()
                        .take(crate::model::tokens::MAX_DESCRIPTION)
                        .collect()
                }),
        });
    }
}

/// The Sketch type for a DTCG `$type`. Also takes Sketch's own type names and Tokens
/// Studio's. `dimension` and `number` don't say what they're for, so the path decides.
fn kind_for(ty: &str, path: &[String]) -> Result<TokenKind, String> {
    use TokenKind::*;
    if let Some(k) = TokenKind::from_key(ty) {
        return Ok(k);
    }
    let guess = || {
        let p = path.join("").to_lowercase().replace(['-', '_', ' '], "");
        [
            ("radius", Radius),
            ("radii", Radius),
            ("borderwidth", BorderWidth),
            ("strokewidth", BorderWidth),
            ("fontsize", FontSize),
            ("letterspacing", LetterSpacing),
            ("tracking", LetterSpacing),
            ("lineheight", LineHeight),
            ("leading", LineHeight),
            ("fontweight", FontWeight),
            ("opacity", Opacity),
            ("breakpoint", Breakpoint),
            ("container", Container),
        ]
        .into_iter()
        .find(|(word, _)| p.contains(word))
        .map(|(_, k)| k)
    };
    Ok(match ty {
        "fontFamilies" => FontFamily,
        "fontWeights" => FontWeight,
        "fontSizes" => FontSize,
        "lineHeights" => LineHeight,
        "borderRadius" => Radius,
        "sizing" => Spacing,
        "dimension" => guess()
            .filter(|k| dtcg_type(*k) == "dimension")
            .unwrap_or(Spacing),
        "number" => guess()
            .filter(|k| matches!(k, LineHeight | Opacity | FontWeight))
            .ok_or(
                "A `number` token needs a name saying it's a line height, opacity or font weight.",
            )?,
        t => return Err(format!("Type \"{t}\" isn't supported.")),
    })
}

/// DTCG's named font weights.
fn weight(name: &str) -> Option<u32> {
    let n = name.to_lowercase().replace(['-', '_', ' '], "");
    Some(match n.as_str() {
        "thin" | "hairline" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "normal" | "regular" | "book" => 400,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        "extrablack" | "ultrablack" => 950,
        _ => return None,
    })
}

fn color_css(o: &Map<String, Value>) -> Result<String, String> {
    let hex = o.get("hex").and_then(Value::as_str).map(String::from);
    let comps: Option<Vec<f64>> = o.get("components").and_then(Value::as_array).map(|c| {
        c.iter()
            .map(|v| v.as_f64().unwrap_or(0.0)) // "none" is a missing channel: 0.
            .collect()
    });
    let alpha = o.get("alpha").and_then(Value::as_f64).unwrap_or(1.0);
    let space = o
        .get("colorSpace")
        .and_then(Value::as_str)
        .unwrap_or("srgb");
    let css = match (space, comps.as_deref()) {
        ("srgb", Some([r, g, b, ..])) => Some(format!(
            "rgb({} {} {} / {alpha})",
            r * 255.0,
            g * 255.0,
            b * 255.0
        )),
        ("hsl", Some([h, s, l, ..])) => Some(format!("hsl({h} {s}% {l}% / {alpha})")),
        ("oklch", Some([l, c, h, ..])) => Some(format!("oklch({l} {c} {h} / {alpha})")),
        ("oklab", Some([l, a, b, ..])) => Some(format!("oklab({l} {a} {b} / {alpha})")),
        _ => None,
    };
    css.or(hex).ok_or(format!(
        "Color space \"{space}\" isn't supported without a `hex`."
    ))
}

/// A DTCG value as the CSS input [`TokenSet::parse_value`] takes. `alias` turns an alias
/// path into the target's CSS name.
fn value_css(
    kind: TokenKind,
    v: &Value,
    alias: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if let Some(p) = alias_path(v) {
        return alias(p)
            .map(|slug| format!("var(--{slug})"))
            .ok_or(format!("The alias {{{p}}} points to no token."));
    }
    let bad = || format!("Can't read {v} as a {} value.", kind.key());
    Ok(match v {
        Value::String(s) if kind == TokenKind::FontWeight => {
            weight(s).map_or(s.clone(), |w| w.to_string())
        }
        Value::String(s) => s.clone(),
        Value::Number(n) if dtcg_type(kind) == "dimension" => format!("{n}px"),
        Value::Number(n) => n.to_string(),
        Value::Array(a) if kind == TokenKind::FontFamily => a
            .first()
            .and_then(Value::as_str)
            .ok_or_else(bad)?
            .to_string(),
        Value::Object(o) if kind == TokenKind::Color => color_css(o)?,
        Value::Object(o) => {
            let n = o.get("value").and_then(Value::as_f64).ok_or_else(bad)?;
            let unit = o.get("unit").and_then(Value::as_str).unwrap_or("px");
            format!("{n}{unit}")
        }
        _ => return Err(bad()),
    })
}

/// The id of the mode named `name` in the file, added if missing.
fn mode_id(set: &mut TokenSet, name: &str, report: &mut Report) -> Result<String, String> {
    let slug = css_slug(name);
    if let Some(m) = set.modes().iter().find(|m| css_slug(&m.name) == slug) {
        return Ok(m.id.clone());
    }
    let id = set.add_mode();
    if let Err(e) = set.rename_mode(&id, name) {
        set.delete_mode(&id).ok();
        return Err(e);
    }
    report.modes_added.push(name.trim().to_string());
    Ok(id)
}

/// Add the tokens of DTCG document `doc` to `set`. A token matches an existing one of the
/// same type and CSS name, which takes the file's values, so a file can be imported again
/// after it changes. `$value` goes to the mode named `mode` (the first mode when None), and
/// modes Sketch wrote in `$extensions` go to theirs. Missing modes are added. Problems are
/// reported per token in [`Report::skipped`]; only a document with no tokens is an error.
pub fn import(set: &mut TokenSet, doc: &Value, mode: Option<&str>) -> Result<Report, String> {
    let root = doc
        .as_object()
        .ok_or("A DTCG file is a JSON object of groups and tokens.")?;
    let mut entries = Vec::new();
    collect(root, &[], None, &mut entries);
    if entries.is_empty() {
        return Err("No tokens found. DTCG tokens are objects with a `$value`.".into());
    }
    let mut report = Report::default();
    let target = match mode {
        Some(m) => mode_id(set, m, &mut report)?,
        None => set.modes()[0].id.clone(),
    };
    let by_path: HashMap<String, usize> = (entries.iter().enumerate())
        .map(|(i, e)| (e.path.clone(), i))
        .collect();
    // An alias without a type takes its target's.
    for i in 0..entries.len() {
        let mut seen = HashSet::new();
        let mut cur = i;
        while matches!(&entries[cur].kind, Err(e) if e.is_empty()) && seen.insert(cur) {
            match alias_path(&entries[cur].values[0].1).and_then(|p| by_path.get(p)) {
                Some(&next) => cur = next,
                None => break,
            }
        }
        entries[i].kind = match &entries[cur].kind {
            Err(e) if e.is_empty() => {
                let p = alias_path(&entries[i].values[0].1).unwrap_or_default();
                let found = set.by_css_name(&css_slug(&p.replace('.', "-")));
                found
                    .map(|t| t.kind)
                    .ok_or(format!("The alias {{{p}}} points to no token."))
            }
            k => k.clone(),
        };
    }
    // Targets first, so aliases to them resolve.
    let mut done = vec![false; entries.len()];
    loop {
        let ready: Vec<usize> = (0..entries.len())
            .filter(|&i| !done[i])
            .filter(|&i| {
                entries[i].values.iter().all(|(_, v)| {
                    alias_path(v)
                        .and_then(|p| by_path.get(p))
                        .is_none_or(|&t| done[t] || t == i)
                })
            })
            .collect();
        if ready.is_empty() {
            break;
        }
        for i in ready {
            done[i] = true;
            let e = &entries[i];
            if let Err(why) = import_one(set, e, &entries, &by_path, &target, &mut report) {
                report.skipped.push((e.path.clone(), why));
            }
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if !done[i] {
            report
                .skipped
                .push((e.path.clone(), "Circular alias.".to_string()));
        }
    }
    Ok(report)
}

fn import_one(
    set: &mut TokenSet,
    e: &Entry,
    entries: &[Entry],
    by_path: &HashMap<String, usize>,
    target: &str,
    report: &mut Report,
) -> Result<(), String> {
    let kind = e.kind.clone()?;
    let alias = |p: &str| match by_path.get(p) {
        Some(&i) => Some(css_slug(&entries[i].name)),
        None => {
            let slug = css_slug(&p.replace('.', "-"));
            set.by_css_name(&slug).map(|t| set.slug(t))
        }
    };
    let raws = (e.values.iter())
        .map(|(m, v)| Ok((m, value_css(kind, v, &alias)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut values: Vec<(String, String)> = Vec::new();
    for (m, raw) in raws {
        let id = match m {
            Some(name) => mode_id(set, name, report)?,
            None => target.to_string(),
        };
        values.retain(|(m, _)| *m != id);
        values.push((id, raw));
    }
    let slug = css_slug(&e.name);
    let existing = (set.sorted().into_iter())
        .find(|t| set.slug(t) == slug && (t.kind == kind || t.system.is_some()))
        .map(|t| t.id.clone());
    let Some(id) = existing else {
        let first = set.modes()[0].id.clone();
        let (base, rest): (Vec<_>, Vec<_>) = values.into_iter().partition(|(m, _)| *m == first);
        let (raw, rest) = match base.into_iter().next() {
            Some((_, raw)) => (raw, rest),
            // Only other modes in the file: their first value stands in for all modes.
            None => (rest[0].1.clone(), rest),
        };
        let id = set.create_in_modes(kind, &e.name, &raw, &rest, e.description.as_deref())?;
        report.created.push(set.css_name(set.get(&id).unwrap()));
        return Ok(());
    };
    if let Some(why) = set.locked(&id) {
        let t = set.get(&id).unwrap();
        let same = values.iter().all(|(m, raw)| {
            let name = set.mode_name(m).map(String::from);
            set.parse_value(kind, raw).ok().as_ref() == Some(set.value_in(t, name.as_deref()))
        });
        if same {
            report.unchanged += 1;
            return Ok(());
        }
        return Err(why);
    }
    let before = set.clone();
    let old = set.get(&id).cloned();
    let done = (values.iter())
        .try_for_each(|(m, raw)| set.update_in_mode(&id, m, Some(raw)))
        .and_then(|()| match &e.description {
            Some(d) => set.update(&id, None, None, Some(d)),
            None => Ok(()),
        });
    if let Err(why) = done {
        *set = before;
        return Err(why);
    }
    let t = set.get(&id).unwrap();
    if Some(t) == old.as_ref() {
        report.unchanged += 1;
    } else {
        report.updated.push(set.css_name(t));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> TokenSet {
        let mut s = TokenSet::default();
        let dark = s.modes()[1].id.clone();
        s.create(TokenKind::Color, "color/brand", "#2f6bff", Some("Brand"))
            .unwrap();
        s.create_in_modes(
            TokenKind::Color,
            "color.surface",
            "#ffffff",
            &[(dark, "#111111".into())],
            None,
        )
        .unwrap();
        s.create(TokenKind::Color, "color.action", "var(--colorbrand)", None)
            .unwrap();
        s.create(TokenKind::Radius, "radius.lg", "12px", None)
            .unwrap();
        s.create(TokenKind::LineHeight, "Body leading", "1.5", None)
            .unwrap();
        s.create(TokenKind::LetterSpacing, "tight", "-0.02em", None)
            .unwrap();
        s.create(TokenKind::FontWeight, "weight.bold", "700", None)
            .unwrap();
        s
    }

    #[test]
    fn export_writes_spec_values_and_keeps_the_rest_in_extensions() {
        let s = seeded();
        let v = export(&s, &s.sorted(), None);
        let surface = &v["color"]["surface"];
        assert_eq!(surface["$type"], "color");
        assert_eq!(surface["$value"]["colorSpace"], "srgb");
        assert_eq!(surface["$value"]["hex"], "#ffffff");
        assert_eq!(
            surface["$extensions"][EXT]["modes"]["Dark"]["hex"],
            "#111111"
        );
        assert_eq!(v["color"]["action"]["$value"], "{color.brand}");
        assert_eq!(
            v["color"]["brand"]["$extensions"][EXT]["name"],
            "color/brand"
        );
        assert_eq!(v["color"]["brand"]["$description"], "Brand");
        let lg = &v["radius"]["lg"];
        assert_eq!(lg["$type"], "dimension");
        assert_eq!(lg["$value"], json!({"value": 12, "unit": "px"}));
        assert_eq!(lg["$extensions"][EXT]["type"], "radius");
        assert_eq!(v["Body leading"]["$type"], "number");
        assert_eq!(v["weight"]["bold"]["$value"], 700);
        assert_eq!(v["$extensions"][EXT]["modes"], json!(["Light", "Dark"]));

        let dark = export(&s, &s.sorted(), Some("Dark"));
        assert_eq!(dark["color"]["surface"]["$value"]["hex"], "#111111");
        assert!(
            dark["color"]["surface"]["$extensions"][EXT]
                .get("modes")
                .is_none()
        );
    }

    #[test]
    fn export_resolves_aliases_to_tokens_left_out() {
        let s = seeded();
        let action: Vec<&Token> = s
            .sorted()
            .into_iter()
            .filter(|t| t.name == "color.action")
            .collect();
        let v = export(&s, &action, None);
        assert_eq!(v["color"]["action"]["$value"]["hex"], "#2f6bff");
    }

    #[test]
    fn a_round_trip_restores_every_token() {
        let s = seeded();
        let doc = export(&s, &s.sorted(), None);
        let mut back = TokenSet::default();
        let r = import(&mut back, &doc, None).unwrap();
        assert_eq!(r.created.len(), 7, "{r:?}");
        assert!(r.skipped.is_empty(), "{r:?}");
        let css = |s: &TokenSet| {
            let mut lines: Vec<String> = s
                .stylesheet(&s.sorted(), false)
                .lines()
                .map(String::from)
                .collect();
            lines.sort();
            lines
        };
        assert_eq!(css(&back), css(&s));
        let again = import(&mut back, &doc, None).unwrap();
        assert_eq!(again.unchanged, 7, "{again:?}");
        assert_eq!(back.len(), 7);
    }

    #[test]
    fn import_reads_other_tools_files() {
        let doc = json!({
            "color": {
                "$type": "color",
                "blue": {"500": {"$value": "#2f6bff"}},
                "primary": {"$value": "{color.blue.500}", "$description": "Main"},
                "ok": {"$value": {"colorSpace": "hsl", "components": [120, 100, 25]}},
            },
            "space": {"$type": "dimension", "sm": {"$value": {"value": 0.5, "unit": "rem"}}},
            "radii": {"md": {"$type": "dimension", "$value": "6px"}},
            "font": {
                "family": {"$type": "fontFamily", "$value": ["Inter", "sans-serif"]},
                "weight": {"$type": "fontWeight", "$value": "semi-bold"},
                "opacity": {"$type": "number", "$value": 0.5},
            },
            "motion": {"$type": "duration", "$value": {"value": 200, "unit": "ms"}},
            "z": {"$type": "number", "$value": 10},
        });
        let mut s = TokenSet::default();
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.created.len(), 8, "{r:?}");
        let skipped: Vec<&str> = r.skipped.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(skipped, ["motion", "z"]);
        let get = |n: &str| s.by_css_name(n).unwrap();
        assert_eq!(s.css_value(get("color-primary")), "var(--color-blue-500)");
        assert_eq!(get("color-primary").description.as_deref(), Some("Main"));
        assert_eq!(s.css_value(get("color-ok")), "#008000");
        assert_eq!(get("space-sm").kind, TokenKind::Spacing);
        assert_eq!(s.css_value(get("space-sm")), "8px");
        assert_eq!(get("radii-md").kind, TokenKind::Radius);
        assert_eq!(s.css_value(get("font-family")), "Inter");
        assert_eq!(s.css_value(get("font-weight")), "600");
        assert_eq!(get("font-opacity").kind, TokenKind::Opacity);
    }

    #[test]
    fn import_updates_matching_tokens_and_fills_a_named_mode() {
        let mut s = seeded();
        let doc = json!({"color": {"surface": {"$type": "color", "$value": "#222222"}}});
        let r = import(&mut s, &doc, Some("Dark")).unwrap();
        assert_eq!(r.updated, ["--color-surface"]);
        let t = s.by_css_name("color-surface").unwrap();
        assert_eq!(s.css_value(t), "#ffffff");
        assert_eq!(s.css_value_in(t, Some("Dark")), "#222222");

        let r = import(&mut s, &doc, Some("Dim")).unwrap();
        assert_eq!(r.modes_added, ["Dim"]);
        assert_eq!(s.modes().len(), 3);
    }

    #[test]
    fn circular_aliases_are_skipped() {
        let doc = json!({"$type": "color", "a": {"$value": "{b}"}, "b": {"$value": "{a}"}});
        let mut s = TokenSet::default();
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.skipped.len(), 2);
        assert!(s.is_empty());
        assert!(import(&mut s, &json!({"a": 1}), None).is_err());
    }
}
