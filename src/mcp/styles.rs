//! Tools for effect styles, a layer's effect list and token modes: what the Tokens panel and
//! the inspector's Effects section do, over MCP.
//!
//! Effects travel as JSON objects (`{"type": "drop-shadow", "y": 4, "blur": 8, ...}`)
//! instead of CSS, because CSS has no way to say that an effect is hidden, and it loses the
//! order between shadows and blurs.

use super::exec::{Args, mark_override, page_arg, resolve_all, shadow_color, text};
use super::tools::tool;
use super::{ToolResult, css};
use crate::editor::Editor;
use crate::model::effects::edit_effects;
use crate::model::tokens::css_slug;
use crate::model::{
    Effect, EffectKind, EffectStyleDef, Node, NodeId, StyleEffect, TokenKind, TokenSet, TokenValue,
};
use serde_json::{Value, json};

/// Tools this module runs.
pub const TOOLS: [&str; 5] = [
    "create_effect_styles",
    "set_effect_styles",
    "set_effects",
    "create_modes",
    "set_modes",
];

/// Run one of [`TOOLS`].
pub(super) fn call(ed: &mut Editor, name: &str, a: &Args) -> ToolResult {
    match name {
        "create_effect_styles" => create_effect_styles(ed, a),
        "set_effect_styles" => set_effect_styles(ed, a),
        "set_effects" => set_effects(ed, a),
        "create_modes" => create_modes(ed, a),
        "set_modes" => set_modes(ed, a),
        _ => Err(format!("unknown tool `{name}`")),
    }
}

// ------------------------------------------------------------------------- effects

const SHADOW_KEYS: [&str; 7] = ["type", "x", "y", "blur", "spread", "color", "visible"];

fn kind(s: &str) -> Option<EffectKind> {
    let key: String = (s.trim().chars())
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match key.as_str() {
        "dropshadow" => Some(EffectKind::DropShadow),
        "innershadow" | "inset" => Some(EffectKind::InnerShadow),
        "layerblur" | "blur" => Some(EffectKind::LayerBlur),
        "backgroundblur" | "backdropblur" => Some(EffectKind::BackgroundBlur),
        _ => None,
    }
}

fn kind_key(k: EffectKind) -> &'static str {
    match k {
        EffectKind::DropShadow => "drop-shadow",
        EffectKind::InnerShadow => "inner-shadow",
        EffectKind::LayerBlur => "layer-blur",
        EffectKind::BackgroundBlur => "background-blur",
    }
}

/// One effect from JSON, and the id of the color token its color names, if any.
fn effect_input(tokens: &TokenSet, v: &Value) -> Result<(Effect, Option<String>), String> {
    let m = v.as_object().ok_or("each effect must be an object")?;
    let e = Args(m);
    let ty = e.req_str("type")?;
    let kind = kind(ty).ok_or_else(|| {
        format!(
            "Unknown effect type \"{ty}\". Use drop-shadow, inner-shadow, layer-blur or \
             background-blur."
        )
    })?;
    let mut out = Effect::new(kind);
    let allowed: &[&str] = match kind.is_shadow() {
        true => &SHADOW_KEYS,
        false => &["type", "blur", "visible"],
    };
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(match SHADOW_KEYS.contains(&k.as_str()) {
            true => format!("`{k}` applies to shadows only; a {ty} takes blur and visible."),
            false => format!(
                "Unknown effect field `{k}`. Fields: {}.",
                allowed.join(", ")
            ),
        });
    }
    let num = |k: &str| -> Result<Option<f64>, String> {
        match e.get(k) {
            None => Ok(None),
            Some(_) => (e.num(k).and_then(Effect::bounded))
                .map(Some)
                .ok_or_else(|| format!("`{k}` must be a number of px")),
        }
    };
    if let Some(b) = num("blur")? {
        if b < 0.0 {
            return Err("`blur` can't be negative".into());
        }
        out.blur = b;
    }
    out.x = num("x")?.unwrap_or(out.x);
    out.y = num("y")?.unwrap_or(out.y);
    out.spread = num("spread")?.unwrap_or(out.spread);
    if let Some(v) = e.get("visible") {
        out.visible = v.as_bool().ok_or("`visible` must be true or false")?;
    }
    let mut token = None;
    if let Some(c) = e.str("color") {
        match crate::model::tokens::var_name(c) {
            Some(name) => {
                let t = (tokens.by_css_name(name))
                    .filter(|t| t.kind == TokenKind::Color)
                    .ok_or_else(|| {
                        format!("{name} is not a color token. get_tokens lists them.")
                    })?;
                if let Some(TokenValue::Color(c)) = tokens.resolve(&t.id) {
                    out.color = c;
                }
                token = Some(t.id.clone());
            }
            None => {
                out.color = css::color(c).ok_or_else(|| format!("`{c}` is not a CSS color"))?;
            }
        }
    }
    Ok((out, token))
}

fn effects_input(tokens: &TokenSet, e: &Args) -> Result<Vec<(Effect, Option<String>)>, String> {
    (e.arr("effects")?.iter())
        .enumerate()
        .map(|(i, v)| effect_input(tokens, v).map_err(|m| format!("effects[{i}]: {m}")))
        .collect()
}

/// An effect in the form [`effect_input`] reads, with `color` already written as CSS.
fn effect_json(e: &Effect, color: impl FnOnce() -> String) -> Value {
    let mut v = json!({"type": kind_key(e.kind), "blur": e.blur});
    if e.kind.is_shadow() {
        v["x"] = json!(e.x);
        v["y"] = json!(e.y);
        v["spread"] = json!(e.spread);
        v["color"] = json!(color());
    }
    v["visible"] = json!(e.visible);
    v
}

/// A style's effects, colors as `var(--token)` where a token gives them.
pub fn style_effects_json(tokens: &TokenSet, s: &EffectStyleDef) -> Value {
    let list: Vec<Value> = (s.effects.iter().enumerate())
        .map(|(i, e)| {
            effect_json(&e.effect, || match tokens.effect_style_color_token(s, i) {
                Some(t) => format!("var({})", tokens.css_name(t)),
                None => css::color_css(e.effect.color),
            })
        })
        .collect();
    json!(list)
}

/// A layer's effects, hidden ones included. None without any.
pub fn layer_effects_json(tokens: &TokenSet, n: &Node) -> Option<Value> {
    let list: Vec<Value> = (n.style.effects.iter().enumerate())
        .map(|(i, e)| effect_json(e, || shadow_color(tokens, n, i, e.color)))
        .collect();
    (!list.is_empty()).then(|| json!(list))
}

fn set_effects(ed: &mut Editor, a: &Args) -> ToolResult {
    let def = page_arg(ed, a)?;
    type Job = (usize, Vec<NodeId>, Vec<(Effect, Option<String>)>);
    let mut jobs: Vec<Job> = Vec::new();
    for u in a.arr("updates")? {
        let u = Args(u.as_object().ok_or("each update must be an object")?);
        let (page, ids) = resolve_all(ed, &u.strs("nodeIds")?, def)?;
        jobs.push((page, ids, effects_input(&ed.tokens, &u)?));
    }
    let tokens = ed.tokens.clone();
    let mut count = 0;
    for (page, ids, effects) in jobs {
        ed.with_page(page, |ed| {
            ed.edit("Edit effects", |tx| {
                for id in &ids {
                    let mode = tokens.mode_at(tx.doc(), *id);
                    let Some(n) = tx.node_mut(*id) else { continue };
                    let list = (effects.iter()).map(|(e, t)| {
                        let mut e = *e;
                        let color = t.as_deref().and_then(|t| tokens.resolve_in(t, mode));
                        if let Some(TokenValue::Color(c)) = color {
                            e.color = c;
                        }
                        (e, t.clone())
                    });
                    n.effect_style = None;
                    edit_effects(n, |l| *l = list.collect());
                    mark_override(n, "effects");
                    count += 1;
                }
            })
        });
    }
    text(json!({"updated": count}))
}

// ------------------------------------------------------------------- effect styles

fn style_input(tokens: &TokenSet, e: &Args) -> Result<Vec<StyleEffect>, String> {
    Ok((effects_input(tokens, e)?.into_iter())
        .map(|(effect, color_token)| StyleEffect {
            effect,
            color_token,
        })
        .collect())
}

fn create_effect_styles(ed: &mut Editor, a: &Args) -> ToolResult {
    let entries = a.arr("styles")?;
    if entries.is_empty() {
        return Err("`styles` needs at least one entry".into());
    }
    let results: Vec<Value> = ed.edit_tokens("Create effect styles", None, |tokens| {
        (entries.iter())
            .map(|e| {
                let made = (|| {
                    let e = Args(e.as_object().ok_or("each style must be an object")?);
                    let effects = style_input(tokens, &e)?;
                    tokens.create_effect_style(e.req_str("name")?, effects, e.str("description"))
                })();
                match made {
                    Ok(id) => {
                        let s = tokens.effect_style(&id).unwrap();
                        json!({"var": tokens.effect_style_var(s), "result": "created"})
                    }
                    Err(message) => json!({"result": "error", "message": message}),
                }
            })
            .collect()
    });
    text(json!({"count": results.len(), "styles": results}))
}

fn set_effect_styles(ed: &mut Editor, a: &Args) -> ToolResult {
    let entries = a.arr("styles")?;
    if entries.is_empty() {
        return Err("`styles` needs at least one entry".into());
    }
    let results: Vec<Value> = ed.edit_tokens("Update effect styles", None, |tokens| {
        (entries.iter())
            .map(|e| {
                let done = (|| {
                    let e = Args(e.as_object().ok_or("each style must be an object")?);
                    let name = e.req_str("name")?;
                    let s = tokens.effect_style_by_name(name).ok_or_else(|| {
                        format!("Effect style not found: \"{name}\". get_tokens lists the styles.")
                    })?;
                    let (id, var) = (s.id.clone(), tokens.effect_style_var(s));
                    if let Some(why) = tokens.effect_style_locked(&id) {
                        return Err(why);
                    }
                    if e.get("delete").and_then(Value::as_bool) == Some(true) {
                        tokens.delete_effect_styles(std::slice::from_ref(&id));
                        return Ok::<_, String>((var, "deleted"));
                    }
                    let effects = match e.get("effects") {
                        Some(_) => Some(style_input(tokens, &e)?),
                        None => None,
                    };
                    tokens.update_effect_style(
                        &id,
                        e.str("newName"),
                        effects,
                        e.str("description"),
                    )?;
                    let s = tokens.effect_style(&id).unwrap();
                    Ok((tokens.effect_style_var(s), "updated"))
                })();
                match done {
                    Ok((var, result)) => json!({"var": var, "result": result}),
                    Err(message) => json!({"result": "error", "message": message}),
                }
            })
            .collect()
    });
    text(json!({"count": results.len(), "styles": results}))
}

// ---------------------------------------------------------------------------- modes

fn mode_names(tokens: &TokenSet) -> Value {
    json!(tokens.modes().iter().map(|m| &m.name).collect::<Vec<_>>())
}

/// The id of this file's mode called `name`, matched as `data-theme` values are.
fn own_mode(tokens: &TokenSet, name: &str) -> Result<String, String> {
    let slug = css_slug(name);
    if let Some(m) = tokens.modes().iter().find(|m| css_slug(&m.name) == slug) {
        return Ok(m.id.clone());
    }
    let names: Vec<&str> = tokens.modes().iter().map(|m| m.name.as_str()).collect();
    let system =
        (tokens.systems().iter()).find(|s| s.modes.iter().any(|m| css_slug(&m.name) == slug));
    Err(match system {
        Some(s) => format!(
            "\"{name}\" is a mode of the {} design system, so it can't be changed in this \
             file. This file's modes: {}.",
            s.name,
            names.join(", ")
        ),
        None => format!(
            "Mode not found: \"{name}\". This file's modes: {}.",
            names.join(", ")
        ),
    })
}

fn create_modes(ed: &mut Editor, a: &Args) -> ToolResult {
    let names = a.strs("names")?;
    if names.is_empty() {
        return Err("`names` needs at least one mode name".into());
    }
    let results: Vec<Value> = ed.edit_tokens("Add modes", None, |tokens| {
        (names.iter())
            .map(|name| {
                let name = name.trim();
                if own_mode(tokens, name).is_ok() {
                    return json!({"name": name, "result": "exists"});
                }
                let id = tokens.add_mode();
                match tokens.rename_mode(&id, name) {
                    Ok(()) => json!({"name": name, "result": "created"}),
                    Err(message) => {
                        let _ = tokens.delete_mode(&id);
                        json!({"name": name, "result": "error", "message": message})
                    }
                }
            })
            .collect()
    });
    text(json!({"modes": mode_names(&ed.tokens), "results": results}))
}

fn set_modes(ed: &mut Editor, a: &Args) -> ToolResult {
    let entries = a.arr("modes")?;
    if entries.is_empty() {
        return Err("`modes` needs at least one entry".into());
    }
    let results: Vec<Value> = ed.edit_tokens("Edit modes", None, |tokens| {
        (entries.iter())
            .map(|e| {
                let done = (|| {
                    let e = Args(e.as_object().ok_or("each mode must be an object")?);
                    let name = e.req_str("name")?;
                    let id = own_mode(tokens, name)?;
                    if e.get("delete").and_then(Value::as_bool) == Some(true) {
                        tokens.delete_mode(&id)?;
                        return Ok::<_, String>((name.to_string(), "deleted"));
                    }
                    let new = e.req_str("newName").map_err(
                        |_| "Each entry needs `newName` to rename the mode or `delete: true`.",
                    )?;
                    tokens.rename_mode(&id, new)?;
                    Ok((new.to_string(), "renamed"))
                })();
                match done {
                    Ok((name, result)) => json!({"name": name, "result": result}),
                    Err(message) => json!({"result": "error", "message": message}),
                }
            })
            .collect()
    });
    text(json!({"modes": mode_names(&ed.tokens), "results": results}))
}

// ------------------------------------------------------------------- textStyle: none

/// Handle `textStyle: "none"` and `effectStyle: "none"` in update_styles: unlink the layer
/// from its style and keep what it shows, as the inspector's detach button does. Return the
/// keys that don't apply to `n`.
pub fn detach_styles(tokens: &TokenSet, n: &mut Node, styles: &mut css::Styles) -> Vec<String> {
    let mut ignored = Vec::new();
    styles.retain(|(k, v)| {
        if !v.trim().eq_ignore_ascii_case("none") {
            return true;
        }
        match k.as_str() {
            "textStyle" if matches!(n.kind, crate::model::NodeKind::Text { .. }) => {
                n.text_style_ref = None;
            }
            "textStyle" => ignored.push(k.clone()),
            "effectStyle" => tokens.detach_effect_style_from(n),
            _ => return true,
        }
        false
    });
    ignored
}

// --------------------------------------------------------------------- definitions

fn effect_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "enum": ["drop-shadow", "inner-shadow", "layer-blur", "background-blur"]},
            "x": {"type": "number", "description": "Shadows: horizontal offset in px. Default 0."},
            "y": {"type": "number", "description": "Shadows: vertical offset in px. Default 4."},
            "blur": {"type": "number", "minimum": 0, "description": "Blur radius in px, as CSS uses it. Default 4 for shadows, 8 for blurs."},
            "spread": {"type": "number", "description": "Shadows: spread in px. Default 0."},
            "color": {"type": "string", "description": "Shadows: a CSS color (\"#0000001a\") or var(--color-token), which follows the token in each mode. Default #00000040."},
            "visible": {"type": "boolean", "description": "false hides the effect but keeps it. Default true."},
        },
        "required": ["type"],
    })
}

fn effects_schema(desc: &str) -> Value {
    json!({"type": "array", "items": effect_schema(), "description": desc})
}

/// MCP definitions of [`TOOLS`].
pub(super) fn definitions() -> Vec<Value> {
    let ids = json!({"type": "array", "items": {"type": "string"}, "description": "Layers that get the list."});
    vec![
        tool(
            "create_effect_styles",
            "Creates effect styles: named lists of shadows and blurs, like \"Elevation/2\". The \
             name becomes a CSS variable (\"Elevation/2\" is --elevation-2). Give a shadow \
             color var(--token) so the style follows that color token in every mode: one \
             style then works in Light and Dark. Apply a style with box-shadow: \
             var(--elevation-2) in write_html or {effectStyle: \"elevation-2\"} in \
             update_styles. Results are per entry.",
            json!({"styles": {"type": "array", "minItems": 1, "description": "Styles to create.", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Style name, e.g. \"Elevation/2\"."},
                    "effects": effects_schema("The effects, in the order the inspector lists them."),
                    "description": {"type": "string", "maxLength": 1024, "description": "What the style is for."},
                },
                "required": ["name", "effects"],
            }}}),
            &["styles"],
            false,
        ),
        tool(
            "set_effect_styles",
            "Updates or deletes effect styles by variable (\"--elevation-2\") or name, applied \
             in order. Set `newName` to rename, `effects` to replace the whole list (get_tokens \
             lists each style's `effects`; edit that list and pass it back), `delete: true` to \
             delete. Changing a style updates every layer that uses it, on all pages. Deleting \
             keeps those layers' effects. Results are per entry.",
            json!({"styles": {"type": "array", "minItems": 1, "description": "Changes to apply.", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The style's variable (\"--elevation-2\") or name."},
                    "newName": {"type": "string"},
                    "effects": effects_schema("The new effect list. It replaces the old one."),
                    "description": {"type": "string", "maxLength": 1024, "description": "New description. An empty string clears it."},
                    "delete": {"type": "boolean"},
                },
                "required": ["name"],
            }}}),
            &["styles"],
            false,
        ),
        tool(
            "set_effects",
            "Replaces layers' effect lists: shadows and blurs, each of which can be hidden \
             without removing it. get_node_info lists a layer's `effects` in this form, hidden \
             ones included: to hide one effect, pass that list back with its `visible` set to \
             false. A shadow color of var(--token) binds to that color token. Detaches an \
             effect style. An empty list removes every effect.",
            json!({
                "updates": {"type": "array", "minItems": 1, "items": {
                    "type": "object",
                    "properties": {
                        "nodeIds": ids,
                        "effects": effects_schema("The layer's new effects, first listed first."),
                    },
                    "required": ["nodeIds", "effects"],
                }},
                "pageId": {"type": "string", "description": "The page of nodeIds given as bare numbers. Defaults to the page the user is viewing."},
            }),
            &["updates"],
            false,
        ),
        tool(
            "create_modes",
            "Adds token modes, like \"Dark\" or \"High contrast\". A mode gives each token \
             another value (create_tokens and set_tokens take them under `modes`); until set, \
             a token shows its first mode's value there. A frame picks a mode with {mode: \
             \"Dark\"} in update_styles or data-theme=\"dark\" in write_html. A name the file \
             already has returns \"exists\". Returns the file's modes, the first being the \
             default.",
            json!({"names": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Mode names, e.g. [\"Dark\"]."}}),
            &["names"],
            false,
        ),
        tool(
            "set_modes",
            "Renames or removes the file's token modes, applied in order. Set `newName` to \
             rename (tokens and frames keep their values and picks), `delete: true` to remove \
             the mode and its values; frames that picked it follow the frame around them. \
             Removing the first mode makes the next one the default. A file keeps at least \
             one mode, and a design system's modes can't be changed here. Results are per \
             entry.",
            json!({"modes": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The mode's current name."},
                    "newName": {"type": "string"},
                    "delete": {"type": "boolean"},
                },
                "required": ["name"],
            }}}),
            &["modes"],
            false,
        ),
    ]
}
