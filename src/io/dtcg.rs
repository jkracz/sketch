//! W3C Design Tokens Community Group (2025.10) import and export.
//! `$extensions["sketch-rs"]` preserves exact names, Sketch types, and mode overrides.

use crate::model::tokens::css_slug;
use crate::model::{Color, Token, TokenKind, TokenSet, TokenValue};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

/// The `$extensions` key for Sketch's own data.
pub const EXT: &str = "sketch-rs";

fn dtcg_type(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Color => "color",
        TokenKind::FontFamily => "fontFamily",
        TokenKind::FontWeight => "fontWeight",
        TokenKind::LineHeight | TokenKind::Opacity => "number",
        _ => "dimension",
    }
}

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
    if v.fract() == 0.0 && v.abs() < 1e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn color_json(c: Color) -> Value {
    // Six decimal places round-trip 8-bit channels.
    let ch = |v: u8| number((v as f64 / 255.0 * 1e6).round() / 1e6);
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
        TokenValue::Em(e) => json!({"value": number(*e), "unit": "em"}),
        TokenValue::Number(n) if dtcg_type(kind) == "dimension" => {
            json!({"value": number(*n), "unit": "px"})
        }
        TokenValue::Number(n) => number(*n),
        TokenValue::Alias(_) => Value::Null,
    }
}

/// Export DTCG with one mode as `$value`. By default, use the first mode and
/// keep overrides in `$extensions`. Resolve aliases to tokens outside the export.
pub fn export(set: &TokenSet, tokens: &[&Token], mode: Option<&str>) -> Value {
    let name_of = |t: &Token| match t.system {
        Some(_) => set.slug(t),
        None => t.name.clone(),
    };
    // Visit descendants first to detect tokens that belong at a group's `$root`.
    let mut order: Vec<(&Token, Vec<String>)> =
        tokens.iter().map(|t| (*t, segments(&name_of(t)))).collect();
    order.sort_by_key(|(_, p)| std::cmp::Reverse(p.len()));
    let mut leaves: HashSet<Vec<String>> = HashSet::new();
    let mut groups: HashSet<Vec<String>> = HashSet::new();
    let mut paths: HashMap<&str, (Vec<String>, bool)> = HashMap::new();
    for (t, mut path) in order {
        let last = path.len() - 1;
        let base = path[last].clone();
        let mut n = 2;
        while leaves.contains(&path) {
            path[last] = format!("{base} {n}");
            n += 1;
        }
        for i in 1..path.len() {
            groups.insert(path[..i].to_vec());
        }
        leaves.insert(path.clone());
        let root = groups.contains(&path);
        paths.insert(&t.id, (path, root));
    }
    let reference = |id: &str| {
        let (path, root) = &paths[id];
        let mut r = path.join(".");
        if *root {
            r.push_str(".$root");
        }
        format!("{{{r}}}")
    };
    let value_at = |t: &Token, m: Option<&str>| match set.value_in(t, m) {
        TokenValue::Alias(a) if paths.contains_key(a.as_str()) => json!(reference(a)),
        TokenValue::Alias(_) => literal_json(
            t.kind,
            &set.resolve_in(&t.id, m).unwrap_or(t.kind.default_value()),
        ),
        v => literal_json(t.kind, v),
    };
    let others: Vec<&str> = match mode {
        Some(_) => Vec::new(),
        None => set
            .choices()
            .into_iter()
            .skip(1)
            .map(|m| m.name.as_str())
            .collect(),
    };
    let mut root = Map::new();
    for t in tokens {
        let (path, is_root) = &paths[t.id.as_str()];
        let mut tok = Map::new();
        let mut ext = Map::new();
        tok.insert("$type".into(), json!(dtcg_type(t.kind)));
        let value = value_at(t, mode);
        let mut spec_value = value.clone();
        // DTCG has no em unit. Use rem in `$value` and flag the original unit in the extension.
        if value.get("unit").and_then(Value::as_str) == Some("em") {
            spec_value["unit"] = json!("rem");
            ext.insert("em".into(), json!(true));
        }
        tok.insert("$value".into(), spec_value);
        if let Some(d) = &t.description {
            tok.insert("$description".into(), json!(d));
        }
        if dtcg_type(t.kind) != t.kind.key() {
            ext.insert("type".into(), json!(t.kind.key()));
        }
        // Other tools get the resolved value; Sketch restores the alias if its target exists.
        if let TokenValue::Alias(a) = set.value_in(t, mode)
            && !paths.contains_key(a.as_str())
            && others
                .iter()
                .all(|m| set.value_in(t, Some(m)) == set.value_in(t, mode))
            && let Some(target) = set.get(a)
        {
            ext.insert("alias".into(), json!(set.slug(target)));
        }
        let name = name_of(t);
        if path.join("-") != name {
            ext.insert("name".into(), json!(name));
        }
        let modes: Map<String, Value> = others
            .iter()
            .map(|m| (m.to_string(), value_at(t, Some(m))))
            .filter(|(_, v)| *v != value)
            .collect();
        if !modes.is_empty() {
            ext.insert("modes".into(), Value::Object(modes));
        }
        if !ext.is_empty() {
            tok.insert("$extensions".into(), json!({ EXT: ext }));
        }
        let (groups, key) = match is_root {
            true => (&path[..], "$root".to_string()),
            false => (&path[..path.len() - 1], path[path.len() - 1].clone()),
        };
        let mut group = &mut root;
        for seg in groups {
            group = group
                .entry(seg.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("deepest-first paths never put a token where a group is");
        }
        group.insert(key, Value::Object(tok));
    }
    if !others.is_empty() {
        let names: Vec<&str> = set.choices().iter().map(|m| m.name.as_str()).collect();
        root.insert("$extensions".into(), json!({ EXT: {"modes": names} }));
    }
    Value::Object(root)
}

/// Changes and skips from a DTCG import.
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
    /// Format import counts for the status bar.
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

struct Entry {
    /// Dotted alias path, including `.$root` for a group's own token.
    path: String,
    name: String,
    kind: Result<TokenKind, String>,
    /// Fallback type for an alias whose target is missing.
    group_kind: Option<Result<TokenKind, String>>,
    value: Value,
    modes: Vec<(String, Value)>,
    /// The CSS name of a token outside the file that Sketch's export resolved an alias to.
    alias: Option<String>,
    description: Option<String>,
}

fn alias_path(v: &Value) -> Option<&str> {
    let s = v.as_str()?.trim();
    s.strip_prefix('{')?.strip_suffix('}')
}

fn path_slug(p: &str) -> String {
    css_slug(&p.trim_end_matches(".$root").replace('.', "-"))
}

fn collect(group: &Map<String, Value>, path: &[String], ty: Option<&str>, out: &mut Vec<Entry>) {
    let ty = group.get("$type").and_then(Value::as_str).or(ty);
    for (key, v) in group {
        let Some(obj) = v.as_object() else { continue };
        let (path, root) = match key.as_str() {
            "$root" => (path.to_vec(), true),
            k if k.starts_with('$') => continue,
            k => (path.iter().cloned().chain([k.to_string()]).collect(), false),
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
        let mut value = obj["$value"].clone();
        if ext.and_then(|e| e.get("em")) == Some(&json!(true)) && value.get("unit").is_some() {
            value["unit"] = json!("em");
        }
        let own = obj.get("$type").and_then(Value::as_str);
        // Untyped aliases inherit from their target before their groups.
        let (kind, group_kind) = match (ext_str("type"), own, ty) {
            (Some(k), _, _) => (
                TokenKind::from_key(k).ok_or(format!("Unknown Sketch type \"{k}\".")),
                None,
            ),
            (None, Some(t), _) => (kind_for(t, &path), None),
            (None, None, g) if alias_path(&value).is_some() => {
                (Err(String::new()), g.map(|g| kind_for(g, &path)))
            }
            (None, None, Some(g)) => (kind_for(g, &path), None),
            (None, None, None) => (Err("No `$type` on the token or its groups.".into()), None),
        };
        let modes = ext
            .and_then(|e| e.get("modes"))
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let mut alias = path.join(".");
        if root {
            alias.push_str(".$root");
        }
        out.push(Entry {
            path: alias,
            name: ext_str("name").map_or_else(|| path.join("-"), String::from),
            kind,
            group_kind,
            value,
            modes,
            alias: ext_str("alias").map(String::from),
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

fn kind_for(ty: &str, path: &[String]) -> Result<TokenKind, String> {
    use TokenKind::*;
    if let Some(k) = TokenKind::from_key(ty) {
        return Ok(k);
    }
    // A DTCG dimension or number can map to several Sketch types. Use the path to choose.
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
    let comps: Option<Vec<f64>> = match o.get("components") {
        None => None,
        Some(c) => Some(
            (c.as_array()
                .ok_or("Color `components` must be a list.")?
                .iter())
            .map(|v| match v {
                Value::String(s) if s == "none" => Ok(0.0),
                v => v.as_f64().ok_or(format!("Bad color component {v}.")),
            })
            .collect::<Result<_, String>>()?,
        ),
    };
    let alpha = match o.get("alpha") {
        None => 1.0,
        Some(a) => a.as_f64().ok_or(format!("Bad color alpha {a}."))?,
    };
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
    let hex = hex.map(|h| match (h.len(), alpha < 1.0) {
        (7, true) => format!("{h}{:02x}", (alpha.clamp(0.0, 1.0) * 255.0).round() as u8),
        _ => h,
    });
    css.or(hex).ok_or(format!(
        "Color space \"{space}\" isn't supported without a `hex`."
    ))
}

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

/// Import DTCG, updating matches by type and CSS name. Apply `$value` to the requested
/// mode, the first mode listed in the extension, or the project's first mode, in that
/// order. Create missing modes. A skip leaves that token intact. An error rolls back
/// the whole import.
pub fn import(set: &mut TokenSet, doc: &Value, mode: Option<&str>) -> Result<Report, String> {
    let original = set.clone();
    let r = import_into(set, &original, doc, mode);
    if r.is_err() {
        *set = original;
    }
    r
}

fn import_into(
    set: &mut TokenSet,
    original: &TokenSet,
    doc: &Value,
    mode: Option<&str>,
) -> Result<Report, String> {
    let root = doc
        .as_object()
        .ok_or("A DTCG file is a JSON object of groups and tokens.")?;
    let mut entries = Vec::new();
    collect(root, &[], None, &mut entries);
    if entries.is_empty() {
        return Err("No tokens found. DTCG tokens are objects with a `$value`.".into());
    }
    let mut report = Report::default();
    // The mode list includes modes whose values match `$value` and have no override.
    let listed: Vec<&str> = match mode {
        Some(_) => Vec::new(),
        None => root
            .get("$extensions")
            .and_then(|e| e.get(EXT))
            .and_then(|e| e.get("modes"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default(),
    };
    let target = match mode.or(listed.first().copied()) {
        Some(m) => mode_id(set, m, &mut report)?,
        None => set.modes()[0].id.clone(),
    };
    let mut covered = Vec::new();
    for name in listed.iter().skip(1) {
        covered.push(mode_id(set, name, &mut report)?);
    }
    let by_path: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.path.clone(), i))
        .collect();
    for i in 0..entries.len() {
        let mut seen = HashSet::new();
        let mut cur = i;
        while matches!(&entries[cur].kind, Err(e) if e.is_empty()) && seen.insert(cur) {
            match alias_path(&entries[cur].value).and_then(|p| by_path.get(p)) {
                Some(&next) => cur = next,
                None => break,
            }
        }
        entries[i].kind = match &entries[cur].kind {
            Err(e) if e.is_empty() => {
                let p = alias_path(&entries[cur].value).unwrap_or_default();
                match (set.by_css_name(&path_slug(p)), &entries[i].group_kind) {
                    (Some(t), _) => Ok(t.kind),
                    (None, Some(k)) => k.clone(),
                    (None, None) => Err(format!("The alias {{{p}}} points to no token.")),
                }
            }
            k => k.clone(),
        };
    }
    for e in &mut entries {
        let target = (e.alias.as_deref()).and_then(|a| set.by_css_name(a));
        if let (Some(t), Ok(kind)) = (target, &e.kind)
            && t.kind == *kind
        {
            e.value = json!(format!("{{{}}}", set.slug(t)));
            e.modes.clear();
        }
    }
    // Match existing tokens in file order, before alias order sets when each is written. An
    // unedited Sketch export lists same-named tokens in the order it found them.
    let mut used = HashSet::new();
    let matched: Vec<Option<String>> = (entries.iter())
        .map(|e| {
            let kind = e.kind.clone().ok()?;
            let slug = css_slug(&e.name);
            let t = (set.sorted().into_iter())
                .filter(|t| !used.contains(&t.id))
                .find(|t| set.slug(t) == slug && (t.kind == kind || t.system.is_some()))?;
            used.insert(t.id.clone());
            Some(t.id.clone())
        })
        .collect();
    // Place alias targets first so aliases can use their token IDs.
    let mut ids: Vec<Option<String>> = vec![None; entries.len()];
    let mut bases: Vec<Option<TokenValue>> = vec![None; entries.len()];
    let mut done = vec![false; entries.len()];
    loop {
        let ready: Vec<usize> = (0..entries.len())
            .filter(|&i| !done[i])
            .filter(|&i| {
                alias_path(&entries[i].value)
                    .and_then(|p| by_path.get(p))
                    .is_none_or(|&t| done[t])
            })
            .collect();
        if ready.is_empty() {
            break;
        }
        for i in ready {
            done[i] = true;
            let ctx = Ctx {
                by_path: &by_path,
                ids: &ids,
            };
            match place(set, &ctx, &entries[i], matched[i].as_ref()) {
                Ok((id, base)) => {
                    ids[i] = Some(id);
                    bases[i] = Some(base);
                }
                Err(why) => report.skipped.push((entries[i].path.clone(), why)),
            }
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if !done[i] {
            report
                .skipped
                .push((e.path.clone(), "Circular alias.".into()));
        }
    }
    // Validate aliases after all writes. Old values can form temporary cycles.
    let ctx = Ctx {
        by_path: &by_path,
        ids: &ids,
    };
    let mut failed = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let (Some(id), Some(base)) = (id, &bases[i]) else {
            continue;
        };
        let e = &entries[i];
        if let Err(why) = write_modes(set, &ctx, e, id, base, &target, &covered, &mut report) {
            failed.push((i, why));
        }
    }
    // Rollback can remove alias targets. Check again after each pass.
    let mut undone = HashSet::new();
    loop {
        for (i, _) in &failed {
            let id = ids[*i].as_ref().unwrap();
            if undone.insert(id.clone()) {
                set.restore_token(id, original);
            }
        }
        let modes: Vec<String> = set.choices().iter().map(|m| m.name.clone()).collect();
        let broken: Vec<(usize, String, bool)> = ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let id = id.as_ref().filter(|id| !undone.contains(*id))?;
                let bad: Vec<&String> = modes
                    .iter()
                    .filter(|m| set.resolve_in(id, Some(m)).is_none())
                    .collect();
                let why = format!(
                    "{}: Circular alias, or an alias to a skipped token.",
                    bad.first()?
                );
                Some((i, why, bad.iter().any(|m| to_blame(set, id, m))))
            })
            .collect();
        if broken.is_empty() {
            break;
        }
        // Restore broken targets first. Their dependents may then resolve.
        let blamed = broken.iter().any(|b| b.2);
        failed.extend(
            broken
                .into_iter()
                .filter(|b| b.2 || !blamed)
                .map(|(i, why, _)| (i, why)),
        );
    }
    // Drop modes only skipped tokens asked for. Modes the file lists stay.
    let mut keep: HashSet<String> = covered.iter().cloned().chain([target.clone()]).collect();
    for (e, id) in entries.iter().zip(&ids) {
        if id.as_ref().is_some_and(|id| !undone.contains(id)) {
            for (name, _) in &e.modes {
                let slug = css_slug(name);
                let m = set.modes().iter().find(|m| css_slug(&m.name) == slug);
                keep.extend(m.map(|m| m.id.clone()));
            }
        }
    }
    for m in set.modes().to_vec() {
        if original.modes().iter().all(|o| o.id != m.id)
            && !keep.contains(&m.id)
            && set.delete_mode(&m.id).is_ok()
        {
            report.modes_added.retain(|n| *n != m.name);
        }
    }
    for (i, why) in failed {
        report.skipped.push((entries[i].path.clone(), why));
    }
    for id in ids.iter().flatten().filter(|id| !undone.contains(*id)) {
        let t = set.get(id).unwrap();
        match original.get(id) {
            None => report.created.push(set.css_name(t)),
            Some(b) if b != t => report.updated.push(set.css_name(t)),
            Some(_) => report.unchanged += 1,
        }
    }
    Ok(report)
}

struct Ctx<'a> {
    by_path: &'a HashMap<String, usize>,
    /// The token placed for each entry so far.
    ids: &'a [Option<String>],
}

impl Ctx<'_> {
    fn parse(&self, set: &TokenSet, kind: TokenKind, v: &Value) -> Result<TokenValue, String> {
        // A path in the file names the token placed for it, never another with its CSS name.
        if let Some(p) = alias_path(v)
            && let Some(&i) = self.by_path.get(p)
        {
            let t = (self.ids[i].as_deref())
                .and_then(|id| set.get(id))
                .ok_or(format!("The alias {{{p}}} points to a skipped token."))?;
            if t.kind != kind {
                return Err(format!("{{{p}}} isn't a {} token.", kind.label()));
            }
            return Ok(TokenValue::Alias(t.id.clone()));
        }
        let alias = |p: &str| set.by_css_name(&path_slug(p)).map(|t| set.slug(t));
        set.parse_value(kind, &value_css(kind, v, &alias)?)
    }
}

/// True for a cycle member or a direct alias to a missing token.
fn to_blame(set: &TokenSet, id: &str, mode: &str) -> bool {
    let Some(TokenValue::Alias(next)) = set.get(id).map(|t| set.value_in(t, Some(mode))) else {
        return false;
    };
    let mut cur = next.as_str();
    let mut seen = HashSet::new();
    while cur != id {
        if !seen.insert(cur) {
            return false;
        }
        match set.get(cur).map(|t| set.value_in(t, Some(mode))) {
            Some(TokenValue::Alias(n)) => cur = n,
            Some(_) => return false,
            None => return seen.len() == 1,
        }
    }
    true
}

fn place(
    set: &mut TokenSet,
    ctx: &Ctx,
    e: &Entry,
    matched: Option<&String>,
) -> Result<(String, TokenValue), String> {
    let kind = e.kind.clone()?;
    let value = ctx.parse(set, kind, &e.value)?;
    if let Some(id) = matched.cloned() {
        if set.locked(&id).is_none()
            && let Some(d) = &e.description
        {
            set.update(&id, None, None, Some(d))?;
        }
        return Ok((id, value));
    }
    // `create` would merge tokens that differ only in their mode overrides.
    let id = set.add_default(kind);
    set.set_value(&id, value.clone());
    if let Err(why) = set.update(
        &id,
        Some(&e.name),
        None,
        Some(e.description.as_deref().unwrap_or_default()),
    ) {
        set.delete(std::slice::from_ref(&id));
        return Err(why);
    }
    Ok((id, value))
}

#[allow(clippy::too_many_arguments)]
fn write_modes(
    set: &mut TokenSet,
    ctx: &Ctx,
    e: &Entry,
    id: &str,
    base: &TokenValue,
    target: &str,
    covered: &[String],
    report: &mut Report,
) -> Result<(), String> {
    let kind = e.kind.clone()?;
    let mut values = vec![(target.to_string(), base.clone())];
    for (name, v) in &e.modes {
        let v = ctx
            .parse(set, kind, v)
            .map_err(|why| format!("{name}: {why}"))?;
        let m = mode_id(set, name, report)?;
        if !values.iter().any(|(x, _)| *x == m) {
            values.push((m, v));
        }
    }
    for m in covered {
        if !values.iter().any(|(x, _)| x == m) {
            values.push((m.clone(), base.clone()));
        }
    }
    if let Some(why) = set.locked(id) {
        let t = set.get(id).unwrap();
        let same =
            t.kind == kind && (values.iter()).all(|(m, v)| set.value_in(t, set.mode_name(m)) == v);
        return if same { Ok(()) } else { Err(why) };
    }
    // Write the first mode before comparing overrides against it.
    let first = set.modes()[0].id.clone();
    values.sort_by_key(|(m, _)| *m != first);
    for (m, v) in values {
        set.set_value_in(id, &m, v);
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
        s.create(TokenKind::LetterSpacing, "tight", "-0.02em", None)
            .unwrap();
        s
    }

    #[test]
    fn round_trip_keeps_sketch_metadata_and_mode_overrides() {
        let s = seeded();
        let doc = export(&s, &s.sorted(), None);
        assert_eq!(doc["color"]["surface"]["$value"]["colorSpace"], "srgb");
        assert_eq!(doc["radius"]["lg"]["$type"], "dimension");
        assert_eq!(
            doc["tight"]["$value"],
            json!({"value": -0.02, "unit": "rem"})
        );
        assert_eq!(doc["$extensions"][EXT]["modes"], json!(["Light", "Dark"]));

        let mut back = TokenSet::default();
        let r = import(&mut back, &doc, None).unwrap();
        assert!(r.skipped.is_empty(), "{r:?}");
        let brand = back.by_css_name("colorbrand").unwrap();
        assert_eq!(brand.name, "color/brand");
        assert_eq!(brand.description.as_deref(), Some("Brand"));
        assert_eq!(
            back.by_css_name("radius-lg").unwrap().kind,
            TokenKind::Radius
        );
        assert_eq!(
            back.css_value(back.by_css_name("tight").unwrap()),
            "-0.02em"
        );
        let surface = back.by_css_name("color-surface").unwrap();
        assert_eq!(back.css_value_in(surface, Some("Dark")), "#111111");
    }

    #[test]
    fn export_of_one_theme_uses_its_values_without_mode_metadata() {
        let s = seeded();
        let dark = export(&s, &s.sorted(), Some("Dark"));
        assert_eq!(dark["color"]["surface"]["$value"]["hex"], "#111111");
        assert!(dark["$extensions"][EXT].get("modes").is_none());
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

        // Sketch puts the alias back where its target exists.
        let mut back = seeded();
        import(&mut back, &v, None).unwrap();
        let t = back.by_css_name("color-action").unwrap();
        assert_eq!(back.css_value(t), "var(--colorbrand)");
        let mut empty = TokenSet::default();
        import(&mut empty, &v, None).unwrap();
        let t = empty.by_css_name("color-action").unwrap();
        assert_eq!(empty.css_value(t), "#2f6bff");
    }

    #[test]
    fn same_named_tokens_keep_their_ids_and_aliases_their_exact_target() {
        let mut s = TokenSet::default();
        s.create(TokenKind::Color, "z", "#00ff00", None).unwrap();
        s.create(TokenKind::Color, "x", "var(--z)", None).unwrap();
        s.create(TokenKind::Color, "x", "#0000ff", None).unwrap();
        let before = s.clone();
        let doc = export(&s, &s.sorted(), None);
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.unchanged, 3, "{r:?}");
        assert_eq!(s, before);

        let doc = json!({"$type": "color",
            "a": {"$value": "#111111", "$extensions": {EXT: {"name": "x"}}},
            "b": {"$value": "#222222", "$extensions": {EXT: {"name": "x"}}},
            "c": {"$value": "{b}"}});
        let mut s = TokenSet::default();
        import(&mut s, &doc, None).unwrap();
        let c = s.by_css_name("c").unwrap();
        let hex = |v: Option<TokenValue>| match v {
            Some(TokenValue::Color(c)) => c.to_hex(),
            v => panic!("{v:?}"),
        };
        assert_eq!(hex(s.resolve(&c.id)), "#222222");
    }

    #[test]
    fn malformed_color_fields_skip_the_token() {
        let mut s = seeded();
        let before = s.clone();
        let bad = [
            json!({"hex": "#ff0000", "alpha": "opaque"}),
            json!({"colorSpace": "srgb", "components": [1, "x", 0]}),
        ];
        for dark in bad {
            let doc = json!({"color": {"surface": {"$type": "color", "$value": "#ffffff",
                "$extensions": {EXT: {"modes": {"Dark": dark}}}}}});
            let r = import(&mut s, &doc, None).unwrap();
            assert_eq!(r.skipped.len(), 1, "{r:?}");
        }
        assert_eq!(s, before);
    }

    #[test]
    fn import_reads_other_tools_files() {
        let doc = json!({
            "color": {
                "$type": "color",
                "blue": {"500": {"$value": "#2f6bff"}},
                "primary": {"$value": "{color.blue.500}"},
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
        let skipped: Vec<&str> = r.skipped.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(skipped, ["motion", "z"]);
        let get = |n: &str| s.by_css_name(n).unwrap();
        assert_eq!(s.css_value(get("color-primary")), "var(--color-blue-500)");
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
        let t = s.by_css_name("color-surface").unwrap();
        assert_eq!(s.css_value_in(t, Some("Dim")), "#222222");
    }

    #[test]
    fn circular_aliases_are_skipped() {
        let doc = json!({"$type": "color", "a": {"$value": "{b}"}, "b": {"$value": "{a}"}});
        let mut s = TokenSet::default();
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.skipped.len(), 2);
        assert!(s.is_empty());
    }

    #[test]
    fn a_token_that_is_also_a_group_goes_under_root() {
        let mut s = TokenSet::default();
        s.create(TokenKind::Color, "color.brand", "#2f6bff", None)
            .unwrap();
        s.create(TokenKind::Color, "color.brand.500", "#1f4bcc", None)
            .unwrap();
        s.create(TokenKind::Color, "link", "var(--color-brand)", None)
            .unwrap();
        let v = export(&s, &s.sorted(), None);
        assert_eq!(v["color"]["brand"]["$root"]["$value"]["hex"], "#2f6bff");
        assert_eq!(v["color"]["brand"]["500"]["$value"]["hex"], "#1f4bcc");
        assert_eq!(v["link"]["$value"], "{color.brand.$root}");
        let mut back = TokenSet::default();
        let r = import(&mut back, &v, None).unwrap();
        assert_eq!(r.created.len(), 3, "{r:?}");
        let t = back.by_css_name("link").unwrap();
        assert_eq!(back.css_value(t), "var(--color-brand)");
    }

    #[test]
    fn mode_list_sets_the_value_mode_and_clears_old_overrides() {
        let doc = json!({
            "$extensions": {EXT: {"modes": ["Slate", "Light", "Dark"]}},
            "$type": "color",
            "color": {"surface": {"$value": "#ffffff"}},
            "banner": {"$value": "#222222", "$extensions": {EXT: {"modes": {"Light": "#ffffff"}}}},
        });
        let mut s = seeded();
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.modes_added, ["Slate"]);
        assert_eq!(r.updated, ["--color-surface"]);
        let surface = s.by_css_name("color-surface").unwrap();
        assert!(s.own_mode_values(surface).is_empty());
        let banner = s.by_css_name("banner").unwrap();
        assert_eq!(s.css_value_in(banner, Some("Slate")), "#222222");
        assert_eq!(s.css_value_in(banner, Some("Light")), "#ffffff");
    }

    #[test]
    fn an_untyped_alias_takes_its_targets_type() {
        let doc = json!({
            "color": {"a": {"$type": "color", "$value": "#ffffff"}},
            "space": {"$type": "dimension", "x": {"$value": "{color.a}"}},
        });
        let mut s = TokenSet::default();
        import(&mut s, &doc, None).unwrap();
        assert_eq!(s.by_css_name("space-x").unwrap().kind, TokenKind::Color);
    }

    #[test]
    fn a_hex_fallback_keeps_alpha() {
        let doc = json!({"c": {"$type": "color", "$value":
            {"colorSpace": "display-p3", "components": [1, 0, 0], "alpha": 0.5, "hex": "#ff0000"}}});
        let mut s = TokenSet::default();
        import(&mut s, &doc, None).unwrap();
        assert_eq!(
            s.css_value(s.by_css_name("c").unwrap()),
            "rgba(255, 0, 0, 0.502)"
        );
    }

    #[test]
    fn mode_aliases_that_loop_only_midway_import() {
        let modes = |x: &str, y: &str| json!({EXT: {"modes": {"X": x, "Y": y}}});
        let doc = json!({
            "$type": "color",
            "$extensions": {EXT: {"modes": ["Light", "X", "Y"]}},
            "a": {"$value": "#aaaaaa", "$extensions": modes("{c}", "#aaaaaa")},
            "b": {"$value": "#bbbbbb", "$extensions": modes("#bbbbbb", "{d}")},
            "c": {"$value": "{b}", "$extensions": modes("{d}", "#cccccc")},
            "d": {"$value": "{a}", "$extensions": modes("#dddddd", "{c}")},
        });
        let mut s = TokenSet::default();
        let r = import(&mut s, &doc, None).unwrap();
        assert!(r.skipped.is_empty(), "{r:?}");
        let c = s.by_css_name("c").unwrap();
        let d = s.by_css_name("d").unwrap();
        assert_eq!(s.css_value_in(c, Some("X")), "var(--d)");
        assert_eq!(s.css_value_in(d, Some("Y")), "var(--c)");
        for t in s.sorted() {
            for m in ["Light", "X", "Y"] {
                assert!(s.resolve_in(&t.id, Some(m)).is_some(), "{} in {m}", t.name);
            }
        }
    }

    #[test]
    fn tokens_with_the_same_name_and_value_stay_apart() {
        let mut s = TokenSet::default();
        let dark = s.modes()[1].id.clone();
        for v in ["#000000", "#111111"] {
            s.create_in_modes(
                TokenKind::Color,
                "x",
                "#ffffff",
                &[(dark.clone(), v.into())],
                None,
            )
            .unwrap();
        }
        let doc = export(&s, &s.sorted(), None);
        let mut back = TokenSet::default();
        let r = import(&mut back, &doc, None).unwrap();
        assert!(r.skipped.is_empty(), "{r:?}");
        let mut values: Vec<_> = back
            .sorted()
            .into_iter()
            .map(|t| back.css_value_in(t, Some("Dark")))
            .collect();
        values.sort();
        assert_eq!(values, ["#000000", "#111111"]);
    }

    #[test]
    fn bad_mode_values_roll_back_tokens_aliases_and_unused_modes() {
        let doc = json!({
            "$type": "color",
            "color": {"surface": {"$value": "#222222", "$description": "New",
                "$extensions": {EXT: {"modes": {"Dark": "not a color"}}}}},
            "new": {"$value": "#123456",
                "$extensions": {EXT: {"modes": {"Dim": "#111111", "Night": "not a color"}}}},
            "alias": {"$value": "{new}"},
        });
        let mut s = seeded();
        let before = s.clone();
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.skipped.len(), 3, "{r:?}");
        assert!(r.modes_added.is_empty(), "{r:?}");
        assert_eq!(s, before);
    }

    #[test]
    fn a_bad_mode_list_changes_nothing() {
        let doc = json!({"$extensions": {EXT: {"modes": ["Light", "Dim", "!"]}},
            "c": {"$type": "color", "$value": "#ffffff"}});
        let mut s = TokenSet::default();
        let before = s.clone();
        assert!(import(&mut s, &doc, None).is_err());
        assert_eq!(s, before);
    }

    #[test]
    fn a_locked_token_with_other_mode_values_is_skipped() {
        let mut src = TokenSet::default();
        src.create(TokenKind::Color, "surface", "#ffffff", None)
            .unwrap();
        src.create(TokenKind::Spacing, "size", "16px", None)
            .unwrap();
        let link = crate::model::SystemLink {
            id: "ds".into(),
            name: "DS".into(),
            prefix: "ds".into(),
            release: 1,
            description: None,
            notes: None,
            modes: Vec::new(),
        };
        let mut s = TokenSet::default();
        s.attach(link, &src.publishable(), &[], &[]).unwrap();
        let before = s.clone();
        // A token of another type is skipped even with the same value.
        let size = s
            .sorted()
            .into_iter()
            .find(|t| t.kind == TokenKind::Spacing)
            .unwrap();
        let size = s.slug(size);
        let doc = json!({size.clone(): {"$type": "dimension", "$value": {"value": 16, "unit": "px"},
            "$extensions": {EXT: {"type": "radius", "name": size}}}});
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!((r.unchanged, r.skipped.len()), (0, 1), "{r:?}");
        let color = s
            .sorted()
            .into_iter()
            .find(|t| t.kind == TokenKind::Color)
            .unwrap();
        let name = s.css_name(color);
        let slug = name.trim_start_matches("--").to_string();
        let doc = json!({
            "$extensions": {EXT: {"modes": ["Light", "Dark"]}},
            slug.clone(): {"$type": "color", "$value": "#ffffff",
                "$extensions": {EXT: {"name": slug, "modes": {"Dark": "#000000"}}}},
        });
        let r = import(&mut s, &doc, None).unwrap();
        assert_eq!(r.skipped.len(), 1, "{r:?}");
        assert_eq!(s, before);

        let doc = json!({slug.clone(): {"$type": "color", "$value": "#000000",
            "$extensions": {EXT: {"name": slug, "modes": {"Dark": "#ffffff"}}}}});
        let r = import(&mut s, &doc, Some("Dark")).unwrap();
        assert_eq!(r.skipped.len(), 1, "{r:?}");
        assert_eq!(s, before);
    }

    #[test]
    fn undoing_a_loop_keeps_the_aliases_it_had_broken() {
        let mut s = TokenSet::default();
        s.create(TokenKind::Color, "a", "#ffffff", None).unwrap();
        s.create(TokenKind::Color, "b", "var(--a)", None).unwrap();
        let doc = json!({"$type": "color", "a": {"$value": "{b}"}, "c": {"$value": "{a}"}});
        let r = import(&mut s, &doc, None).unwrap();
        let skipped: Vec<&str> = r.skipped.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(skipped, ["a"]);
        assert_eq!(r.created, ["--c"]);
        assert_eq!(s.css_value(s.by_css_name("a").unwrap()), "#ffffff");
    }
}
