//! Style keys for layer properties CSS has no name for: constraints, removing auto layout,
//! stroke alignment, caps and joins, and polygon and star geometry. `update_styles`,
//! `write_html` and `create_shape` take them, and `get_computed_styles` returns them.

use super::css::{Applied, Context};
use crate::model::{Cap, Constraint, Join, Node, NodeKind, Sizing, StrokeAlign};
use serde_json::{Map, Value};

const KEYS: [&str; 8] = [
    "horizontalConstraint",
    "verticalConstraint",
    "autoLayout",
    "strokeAlign",
    "strokeLinecap",
    "strokeLinejoin",
    "sides",
    "innerRadius",
];

pub fn owns(key: &str) -> bool {
    KEYS.contains(&key)
}

/// Apply this module's keys. Runs after the other styles, so `strokeAlign` wins over the
/// inside alignment that `border` sets.
pub fn apply(n: &mut Node, styles: &[(String, String)], ctx: &Context, a: &mut Applied) {
    for (k, v) in styles.iter().filter(|(k, _)| owns(k)) {
        let v = v.trim().to_ascii_lowercase();
        let ok = match k.as_str() {
            // Constraints place free children of frames, and absolute ones in auto layout.
            "horizontalConstraint" | "verticalConstraint"
                if ctx.parent_is_frame
                    && (ctx.parent_layout.is_none() || n.layout_child.absolute) =>
            {
                let h = k == "horizontalConstraint";
                match constraint(&v, h) {
                    Some(c) if h => {
                        n.layout_child.constraints.h = c;
                        true
                    }
                    Some(c) => {
                        n.layout_child.constraints.v = c;
                        true
                    }
                    None => false,
                }
            }
            "autoLayout" if n.is_frame() && matches!(v.as_str(), "false" | "none") => {
                // Children keep where layout put them. Hug needs a layout; Fill is about the
                // parent's, so it stays.
                n.layout = None;
                for s in &mut n.layout_child.sizing {
                    if *s == Sizing::Hug {
                        *s = Sizing::Fixed;
                    }
                }
                true
            }
            "strokeAlign" => match v.as_str() {
                "inside" => set(&mut n.style.stroke_align, StrokeAlign::Inside),
                "center" => set(&mut n.style.stroke_align, StrokeAlign::Center),
                "outside" => set(&mut n.style.stroke_align, StrokeAlign::Outside),
                _ => false,
            },
            "strokeLinecap" => match v.as_str() {
                "butt" => set(&mut n.style.cap, Cap::Butt),
                "round" => set(&mut n.style.cap, Cap::Round),
                "square" => set(&mut n.style.cap, Cap::Square),
                _ => false,
            },
            "strokeLinejoin" => match v.as_str() {
                "miter" => set(&mut n.style.join, Join::Miter),
                "round" => set(&mut n.style.join, Join::Round),
                "bevel" => set(&mut n.style.join, Join::Bevel),
                _ => false,
            },
            "sides" => match (&mut n.kind, number(&v)) {
                // The same range as create_shape.
                (NodeKind::Polygon { sides: s } | NodeKind::Star { points: s, .. }, Some(x)) => {
                    *s = x.round().clamp(3.0, 64.0) as u32;
                    true
                }
                _ => false,
            },
            "innerRadius" => match (&mut n.kind, ratio(&v)) {
                (NodeKind::Star { inner, .. }, Some(r)) => {
                    // The inspector's range: 1% to 100% of the outer radius.
                    *inner = r.clamp(0.01, 1.0);
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if !ok && !a.ignored.contains(k) {
            a.ignored.push(k.clone());
        }
    }
}

fn set<T>(field: &mut T, v: T) -> bool {
    *field = v;
    true
}

fn constraint(v: &str, horizontal: bool) -> Option<Constraint> {
    Some(match (v, horizontal) {
        ("left", true) | ("top", false) => Constraint::Min,
        ("right", true) | ("bottom", false) => Constraint::Max,
        ("left-right", true) | ("top-bottom", false) | ("stretch", _) => Constraint::Stretch,
        ("center", _) => Constraint::Center,
        ("scale", _) => Constraint::Scale,
        _ => return None,
    })
}

fn constraint_css(c: Constraint, horizontal: bool) -> &'static str {
    match (c, horizontal) {
        (Constraint::Min, true) => "left",
        (Constraint::Min, false) => "top",
        (Constraint::Max, true) => "right",
        (Constraint::Max, false) => "bottom",
        (Constraint::Stretch, true) => "left-right",
        (Constraint::Stretch, false) => "top-bottom",
        (Constraint::Center, _) => "center",
        (Constraint::Scale, _) => "scale",
    }
}

/// A plain number. JSON numbers arrive as px.
fn number(v: &str) -> Option<f64> {
    let v = v.strip_suffix("px").unwrap_or(v).trim();
    v.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// 0 to 1, or a percent.
fn ratio(v: &str) -> Option<f64> {
    match v.strip_suffix('%') {
        Some(p) => number(p).map(|p| p / 100.0),
        None => number(v),
    }
}

/// Add this module's keys to computed styles, where they differ from the defaults. `stroked`
/// says whether the node shows a border.
pub fn computed(n: &Node, is_root: bool, stroked: bool, m: &mut Map<String, Value>) {
    let mut put = |k: &str, v: String| {
        m.insert(k.into(), Value::String(v));
    };
    let c = n.layout_child.constraints;
    if !is_root {
        if c.h != Constraint::Min {
            put("horizontalConstraint", constraint_css(c.h, true).into());
        }
        if c.v != Constraint::Min {
            put("verticalConstraint", constraint_css(c.v, false).into());
        }
    }
    if stroked {
        // `border` is inside, like CSS.
        match n.style.stroke_align {
            StrokeAlign::Inside => {}
            StrokeAlign::Center => put("strokeAlign", "center".into()),
            StrokeAlign::Outside => put("strokeAlign", "outside".into()),
        }
        match n.style.cap {
            Cap::Butt => {}
            Cap::Round => put("strokeLinecap", "round".into()),
            Cap::Square => put("strokeLinecap", "square".into()),
        }
        match n.style.join {
            Join::Miter => {}
            Join::Round => put("strokeLinejoin", "round".into()),
            Join::Bevel => put("strokeLinejoin", "bevel".into()),
        }
    }
    match n.kind {
        NodeKind::Polygon { sides } => put("sides", sides.to_string()),
        NodeKind::Star { points, inner } => {
            put("sides", points.to_string());
            put(
                "innerRadius",
                format!("{}", (inner * 1000.0).round() / 1000.0),
            );
        }
        _ => {}
    }
}
