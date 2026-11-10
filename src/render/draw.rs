//! Nodes → backend-neutral vector draw operations in world coordinates.
//!
//! The GPU tessellator, the SVG writer and the PDF writer all consume these ops, so every
//! backend draws the same geometry: shape outlines, gradients, images, strokes and glyphs.

use crate::assets;
use crate::geom::{Rect, V2, v2};
use crate::ink;
use crate::model::effects::blur_sigma;
use crate::model::{
    BlendMode, Cap, Color, Document, EffectKind, FillRule, ImageMode, Join, Node, NodeId, NodeKind,
    Paint, Stop, StrokeAlign,
};
use crate::text::{self, FaceId};
use crate::vector::{self, VPath};

#[derive(Debug, Clone, PartialEq)]
pub enum PaintSpec {
    /// Straight-alpha RGBA, opacity already applied.
    Solid([f32; 4]),
    Linear {
        p0: V2,
        p1: V2,
        stops: Vec<Stop>,
        opacity: f32,
    },
    /// Ellipse with center and the two radius vectors.
    Radial {
        center: V2,
        rx: V2,
        ry: V2,
        stops: Vec<Stop>,
        opacity: f32,
    },
    Image {
        asset: String,
        /// World rect of one copy of the image.
        place: Rect,
        tile: bool,
        opacity: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawOp {
    Fill {
        path: VPath,
        paint: PaintSpec,
        even_odd: bool,
    },
    Stroke {
        path: VPath,
        color: [f32; 4],
        width: f64,
        cap: Cap,
        join: Join,
        dash: Vec<f64>,
    },
    /// Glyphs sharing one face, size and color; positions are world baselines.
    Glyphs {
        face: FaceId,
        size: f64,
        color: [f32; 4],
        glyphs: Vec<(u16, V2)>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeDraw {
    pub ops: Vec<DrawOp>,
    pub blend: BlendMode,
}

pub fn rgba(c: Color, opacity: f64) -> [f32; 4] {
    c.to_f32(opacity)
}

/// The node's outline in world space (None for text and groups).
pub fn shape_path(n: &Node, wr: Rect) -> Option<VPath> {
    Some(match &n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => vector::rounded_rect(wr, n.style.corner_radii()),
        NodeKind::Ellipse => vector::ellipse(wr),
        NodeKind::Polygon { sides } => {
            vector::rounded_polygon(&vector::polygon_points(wr, *sides), n.style.corner_radius)
        }
        NodeKind::Star { points, inner } => vector::rounded_polygon(
            &vector::star_points(wr, *points, *inner),
            n.style.corner_radius,
        ),
        NodeKind::Path { subpaths, .. } => vector::from_subpaths(wr.min(), subpaths),
        NodeKind::Line { a, b } => VPath::polyline(&[wr.min() + *a, wr.min() + *b], false),
        NodeKind::Stroke {
            points,
            width,
            thinning,
        } => {
            let o = ink::outline(points, *width, *thinning);
            let pts: Vec<V2> = o.iter().map(|q| wr.min() + *q).collect();
            VPath::polyline(&pts, true)
        }
        NodeKind::Connector { points, route, .. } => {
            let pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
            crate::connector::line_path(&pts, *route)
        }
        NodeKind::Sticky { .. } => vector::rounded_rect(wr, [n.style.corner_radius.max(2.0); 4]),
        NodeKind::Text { .. } | NodeKind::Group => return None,
    })
}

/// Whether the outline is a closed area (fills apply).
pub fn is_area(n: &Node) -> bool {
    !matches!(
        n.kind,
        NodeKind::Line { .. }
            | NodeKind::Text { .. }
            | NodeKind::Group
            | NodeKind::Connector { .. }
    ) && !matches!(&n.kind, NodeKind::Path { subpaths, .. } if subpaths.iter().all(|s| !s.closed))
}

/// Stroke outline honoring inside/outside alignment for box shapes.
fn stroke_path(n: &Node, wr: Rect) -> Option<VPath> {
    let d = match n.style.stroke_align {
        StrokeAlign::Center => 0.0,
        StrokeAlign::Inside => -n.style.stroke_width * 0.5,
        StrokeAlign::Outside => n.style.stroke_width * 0.5,
    };
    if d == 0.0 {
        return shape_path(n, wr);
    }
    let r = wr.inflate(d);
    if r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    match &n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => Some(vector::rounded_rect(
            r,
            n.style
                .corner_radii()
                .map(|x| if x > 0.0 { (x + d).max(0.0) } else { 0.0 }),
        )),
        NodeKind::Ellipse => Some(vector::ellipse(r)),
        NodeKind::Polygon { sides } => Some(vector::rounded_polygon(
            &vector::polygon_points(r, *sides),
            n.style.corner_radius,
        )),
        NodeKind::Star { points, inner } => Some(vector::rounded_polygon(
            &vector::star_points(r, *points, *inner),
            n.style.corner_radius,
        )),
        _ => shape_path(n, wr),
    }
}

/// Where one copy of an image goes for a given mode.
pub fn image_place(mode: ImageMode, wr: Rect, iw: f64, ih: f64) -> Rect {
    let (iw, ih) = (iw.max(1.0), ih.max(1.0));
    match mode {
        ImageMode::Stretch => wr,
        ImageMode::Tile => Rect::new(wr.x, wr.y, iw, ih),
        ImageMode::Fill | ImageMode::Fit => {
            let s = if mode == ImageMode::Fill {
                (wr.w / iw).max(wr.h / ih)
            } else {
                (wr.w / iw).min(wr.h / ih)
            };
            let size = v2(iw * s, ih * s);
            let c = wr.center();
            Rect::new(c.x - size.x * 0.5, c.y - size.y * 0.5, size.x, size.y)
        }
    }
}

pub fn paint_spec(p: &Paint, wr: Rect, opacity: f64) -> Option<PaintSpec> {
    let at = |u: V2| v2(wr.x + u.x * wr.w, wr.y + u.y * wr.h);
    Some(match p {
        Paint::Solid { color } => PaintSpec::Solid(rgba(*color, opacity)),
        Paint::Linear { stops, from, to } => PaintSpec::Linear {
            p0: at(*from),
            p1: at(*to),
            stops: sorted(stops),
            opacity: opacity as f32,
        },
        Paint::Radial {
            stops,
            center,
            radius,
        } => PaintSpec::Radial {
            center: at(*center),
            rx: v2(radius.x * wr.w, 0.0),
            ry: v2(0.0, radius.y * wr.h),
            stops: sorted(stops),
            opacity: opacity as f32,
        },
        Paint::Image {
            asset,
            mode,
            opacity: o,
        } => {
            let a = assets::get(asset);
            let (iw, ih) = a
                .as_ref()
                .map_or((wr.w, wr.h), |a| (a.width as f64, a.height as f64));
            if a.is_none() {
                // Missing asset: a neutral placeholder.
                return Some(PaintSpec::Solid(rgba(
                    Color::rgb(0xcc, 0xcc, 0xd2),
                    opacity * o,
                )));
            }
            PaintSpec::Image {
                asset: asset.clone(),
                place: image_place(*mode, wr, iw, ih),
                tile: *mode == ImageMode::Tile,
                opacity: (opacity * o) as f32,
            }
        }
    })
}

fn sorted(stops: &[Stop]) -> Vec<Stop> {
    let mut s = stops.to_vec();
    s.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    s
}

/// Draw ops for one node (not its children), in world space.
pub fn node_draw(n: &Node, wr: Rect, opacity: f64) -> NodeDraw {
    let mut ops = Vec::new();
    if let Some(r) = n.style.rough {
        ops = super::rough::node_ops(n, wr, opacity, &r);
        return NodeDraw {
            ops,
            blend: n.style.blend,
        };
    }
    match &n.kind {
        NodeKind::Group => {}
        NodeKind::Text { .. } => text_ops(n, wr, opacity, &mut ops),
        NodeKind::Connector { .. } => connector_ops(n, wr, opacity, &mut ops, false),
        NodeKind::Sticky { .. } => {
            let c = n
                .style
                .fills
                .last()
                .map_or(Color::rgb(0xff, 0xdf, 0x6b), |p| p.preview_color());
            let rad = [n.style.corner_radius.max(2.0); 4];
            // Soft drop shadow, then the note and its text.
            for (i, a) in [(3.0, 0.05), (1.5, 0.07)] {
                ops.push(DrawOp::Fill {
                    path: vector::rounded_rect(wr.translate(v2(0.0, i)).inflate(i * 0.5), rad),
                    paint: PaintSpec::Solid(rgba(Color::rgba(0, 0, 0, (a * 255.0) as u8), opacity)),
                    even_odd: false,
                });
            }
            ops.push(DrawOp::Fill {
                path: vector::rounded_rect(wr, rad),
                paint: PaintSpec::Solid(rgba(c, opacity)),
                even_odd: false,
            });
            if let Some(sc) = n.style.stroke
                && n.style.stroke_width > 0.0
            {
                ops.push(DrawOp::Stroke {
                    path: vector::rounded_rect(wr, rad),
                    color: rgba(sc, opacity),
                    width: n.style.stroke_width,
                    cap: n.style.cap,
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
            text_ops(n, wr, opacity, &mut ops);
        }
        NodeKind::Line { .. } => {
            if let (Some(c), Some(path)) = (n.style.stroke, shape_path(n, wr)) {
                ops.push(DrawOp::Stroke {
                    path,
                    color: rgba(c, opacity),
                    width: n.style.stroke_width.max(0.0),
                    cap: if n.style.cap == Cap::Butt {
                        Cap::Round
                    } else {
                        n.style.cap
                    },
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
        }
        NodeKind::Stroke { .. } => {
            let c = n
                .style
                .fills
                .last()
                .map_or(Color::INK, |p| p.preview_color());
            if let Some(path) = shape_path(n, wr) {
                ops.push(DrawOp::Fill {
                    path,
                    paint: PaintSpec::Solid(rgba(c, opacity)),
                    even_odd: false,
                });
            }
        }
        _ => {
            let Some(path) = shape_path(n, wr) else {
                return NodeDraw::default();
            };
            let even_odd = matches!(
                n.kind,
                NodeKind::Path {
                    fill_rule: FillRule::EvenOdd,
                    ..
                }
            );
            if is_area(n) {
                for p in &n.style.fills {
                    if let Some(paint) = paint_spec(p, wr, opacity) {
                        ops.push(DrawOp::Fill {
                            path: path.clone(),
                            paint,
                            even_odd,
                        });
                    }
                }
            }
            if let Some(c) = n.style.stroke
                && n.style.stroke_width > 0.0
                && let Some(sp) = stroke_path(n, wr)
            {
                ops.push(DrawOp::Stroke {
                    path: sp,
                    color: rgba(c, opacity),
                    width: n.style.stroke_width,
                    cap: n.style.cap,
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
        }
    }
    NodeDraw {
        ops,
        blend: n.style.blend,
    }
}

/// A connector's line and heads. With `rough`, only the heads are produced (the line comes
/// from the rough renderer).
pub fn connector_ops(n: &Node, wr: Rect, opacity: f64, ops: &mut Vec<DrawOp>, heads_only: bool) {
    let NodeKind::Connector {
        points,
        route,
        start_head,
        end_head,
        ..
    } = &n.kind
    else {
        return;
    };
    let Some(c) = n.style.stroke else { return };
    let color = rgba(c, opacity);
    let sw = n.style.stroke_width.max(0.5);
    let pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
    if pts.len() < 2 {
        return;
    }
    if !heads_only {
        ops.push(DrawOp::Stroke {
            path: crate::connector::line_path(&pts, *route),
            color,
            width: sw,
            cap: Cap::Round,
            join: Join::Round,
            dash: n.style.dash.clone(),
        });
    }
    let last = pts.len() - 1;
    for (head, tip, from) in [
        (*start_head, pts[0], pts[1]),
        (*end_head, pts[last], pts[last - 1]),
    ] {
        let (lines, polys) = crate::connector::head_shapes(head, tip, from, sw, tip.dist(from));
        for l in lines {
            ops.push(DrawOp::Stroke {
                path: VPath::polyline(&l, false),
                color,
                width: sw,
                cap: Cap::Round,
                join: Join::Round,
                dash: Vec::new(),
            });
        }
        for p in polys {
            let path = VPath::polyline(&p, true);
            ops.push(DrawOp::Fill {
                path: path.clone(),
                paint: PaintSpec::Solid(color),
                even_odd: false,
            });
            ops.push(DrawOp::Stroke {
                path,
                color,
                width: sw,
                cap: Cap::Round,
                join: Join::Round,
                dash: Vec::new(),
            });
        }
    }
}

fn text_ops(n: &Node, wr: Rect, opacity: f64, ops: &mut Vec<DrawOp>) {
    let Some(l) = text::layout_node(n) else {
        return;
    };
    let o = wr.min() + text::text_offset(n);
    let mut i = 0;
    while i < l.glyphs.len() {
        let g0 = l.glyphs[i];
        let mut glyphs = Vec::new();
        while i < l.glyphs.len() {
            let g = &l.glyphs[i];
            if g.face != g0.face || g.size != g0.size || g.color != g0.color {
                break;
            }
            glyphs.push((g.gid, o + v2(g.x, g.baseline)));
            i += 1;
        }
        ops.push(DrawOp::Glyphs {
            face: g0.face,
            size: g0.size,
            color: rgba(g0.color, opacity),
            glyphs,
        });
    }
    // Underline / strikethrough as thin rectangles.
    for g in l.glyphs.iter().filter(|g| g.underline || g.strike) {
        let th = (g.size / 14.0).max(0.5);
        for (on, dy) in [(g.underline, g.size * 0.12), (g.strike, -g.size * 0.28)] {
            if on {
                let r = Rect::new(o.x + g.x, o.y + g.baseline + dy, g.advance, th);
                ops.push(DrawOp::Fill {
                    path: vector::rounded_rect(r, [0.0; 4]),
                    paint: PaintSpec::Solid(rgba(g.color, opacity)),
                    even_odd: false,
                });
            }
        }
    }
}

/// Product of ancestor opacities.
pub fn inherited_opacity(doc: &Document, id: NodeId) -> f64 {
    doc.ancestors(id)
        .iter()
        .filter_map(|a| doc.get(*a))
        .map(|n| n.style.opacity)
        .product()
}

/// World bounds a node draws into (rect plus stroke overhang).
pub fn draw_bounds(n: &Node, wr: Rect) -> Rect {
    let mut pad = match n.style.stroke_align {
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Center => n.style.stroke_width * 0.5,
        StrokeAlign::Outside => n.style.stroke_width,
    };
    if n.style.stroke.is_none() && !matches!(n.kind, NodeKind::Line { .. }) {
        pad = 0.0;
    }
    if n.style.rough.is_some() {
        pad += n.style.stroke_width * 2.0 + 4.0;
    }
    if matches!(n.kind, NodeKind::Line { .. }) {
        pad = pad.max(n.style.stroke_width * 0.5);
    }
    if matches!(n.kind, NodeKind::Connector { .. }) {
        // Arrowheads reach past the end points.
        pad = pad.max(n.style.stroke_width + 20.0);
    }
    if matches!(n.kind, NodeKind::Sticky { .. }) {
        pad = pad.max(4.0);
    }
    wr.inflate(pad)
}

/// How far a Gaussian of `sigma` visibly reaches.
pub fn blur_reach(sigma: f64) -> f64 {
    sigma * 3.0
}

/// World bounds a node and its drop shadows paint into, before any layer blur. Children are
/// not included.
pub fn shadow_bounds(n: &Node, wr: Rect) -> Rect {
    let own = draw_bounds(n, wr);
    let mut b = own;
    for e in &n.style.effects {
        if e.visible && e.kind == EffectKind::DropShadow {
            let reach = e.spread.max(0.0) + blur_reach(e.sigma());
            b = b.union(&own.translate(v2(e.x, e.y)).inflate(reach));
        }
    }
    b
}

/// How far a layer blur on `n` spreads what it and its children paint.
pub fn layer_blur_reach(n: &Node) -> f64 {
    blur_reach(blur_sigma(&n.style.effects, EffectKind::LayerBlur))
}

/// A rectangle with corner radii, in the order tl, tr, br, bl.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundRect {
    pub rect: Rect,
    pub radii: [f64; 4],
}

impl RoundRect {
    /// Grow by `d` on every side, or shrink when `d` is negative, as CSS grows a shadow by its
    /// spread. Round corners follow and square ones stay square. None once nothing is left.
    pub fn inflate(&self, d: f64) -> Option<RoundRect> {
        let rect = self.rect.inflate(d);
        (rect.w > 0.0 && rect.h > 0.0).then(|| RoundRect {
            rect,
            radii: (self.radii).map(|r| if r > 0.0 { (r + d).max(0.0) } else { 0.0 }),
        })
    }

    pub fn translate(&self, d: V2) -> RoundRect {
        RoundRect {
            rect: self.rect.translate(d),
            radii: self.radii,
        }
    }
}

/// The node's outline when it is a rounded box, which casts shadows the way a CSS box does:
/// from the box, whatever is painted in it. A circle counts.
pub fn box_shape(n: &Node, wr: Rect) -> Option<RoundRect> {
    let max = (wr.w.min(wr.h) * 0.5).max(0.0);
    let radii = match n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => n.style.corner_radii(),
        NodeKind::Sticky { .. } => [n.style.corner_radius.max(2.0); 4],
        NodeKind::Ellipse if (wr.w - wr.h).abs() < 1e-6 => [max; 4],
        _ => return None,
    };
    Some(RoundRect {
        rect: wr,
        radii: radii.map(|r| r.clamp(0.0, max)),
    })
}

/// How far the stroke of a box reaches outside its rect and inside it.
pub fn stroke_reach(n: &Node) -> (f64, f64) {
    if n.style.stroke.is_none() || n.style.stroke_width <= 0.0 {
        return (0.0, 0.0);
    }
    let w = n.style.stroke_width;
    match n.style.stroke_align {
        StrokeAlign::Inside => (0.0, w),
        StrokeAlign::Center => (w * 0.5, w * 0.5),
        StrokeAlign::Outside => (w, 0.0),
    }
}

/// The node's filled shape as one opaque op, for effects that work inside it. None for
/// layers that have no area of their own, such as text, lines and groups.
pub fn mask_op(n: &Node, wr: Rect) -> Option<DrawOp> {
    if !is_area(n) {
        return None;
    }
    Some(DrawOp::Fill {
        path: shape_path(n, wr)?,
        paint: PaintSpec::Solid([1.0; 4]),
        even_odd: matches!(
            n.kind,
            NodeKind::Path {
                fill_rule: FillRule::EvenOdd,
                ..
            }
        ),
    })
}

/// The node's filled shape grown by `d` on every side, or shrunk when `d` is negative, as one
/// opaque op. An ellipse stays an ellipse, as in CSS, and other outlines grow with round
/// corners. None for layers without an area.
pub fn spread_op(n: &Node, wr: Rect, d: f64, tol: f64) -> Option<DrawOp> {
    let DrawOp::Fill { path, even_odd, .. } = mask_op(n, wr)? else {
        return None;
    };
    let grown = wr.inflate(d);
    let path = match n.kind {
        NodeKind::Ellipse if grown.w > 0.0 && grown.h > 0.0 => vector::ellipse(grown),
        NodeKind::Ellipse => VPath::new(),
        _ => vector::offset(&path, even_odd, d, tol),
    };
    Some(solid(path))
}

fn solid(path: VPath) -> DrawOp {
    DrawOp::Fill {
        path,
        paint: PaintSpec::Solid([1.0; 4]),
        even_odd: false,
    }
}

/// What a drop shadow with spread is cast from: the area the node paints, grown by `d` or
/// shrunk when `d` is negative, as opaque ops, and how opaque the node paints it. A filled
/// shape grows from its outline, a hollow one from its stroke, and text from its glyphs.
/// None for groups, which paint nothing themselves.
pub fn cast_ops(n: &Node, wr: Rect, d: f64, tol: f64) -> Option<(Vec<DrawOp>, f64)> {
    let alpha = |c: Color| c.a as f64 / 255.0;
    let fill_alpha = (n.style.fills.iter())
        .map(|p| match p {
            Paint::Solid { color } => alpha(*color),
            Paint::Image { opacity, .. } => *opacity,
            _ => 1.0,
        })
        .fold(0.0, f64::max);
    if matches!(n.kind, NodeKind::Text { .. }) {
        let l = text::layout_node(n)?;
        let o = wr.min() + text::text_offset(n);
        let mut glyphs = VPath::new();
        for g in &l.glyphs {
            glyphs.append(&text::glyph_path(
                g.face,
                g.gid,
                o + v2(g.x, g.baseline),
                g.size,
            ));
        }
        let grown = vector::offset(&glyphs, false, d, tol);
        return Some((vec![solid(grown)], fill_alpha));
    }
    let ink = matches!(n.kind, NodeKind::Stroke { .. });
    if is_area(n) && (ink || !n.style.fills.is_empty()) {
        // A stroke outside the outline is part of what the layer paints.
        let op = spread_op(n, wr, stroke_reach(n).0 + d, tol)?;
        return Some((vec![op], if ink { 1.0 } else { fill_alpha }));
    }
    let color = n.style.stroke?;
    let width = n.style.stroke_width + 2.0 * d;
    let line = matches!(n.kind, NodeKind::Line { .. } | NodeKind::Connector { .. });
    let ops = (width > 0.0).then(|| DrawOp::Stroke {
        path: stroke_path(n, wr).unwrap_or_default(),
        color: [1.0; 4],
        width,
        cap: if line { Cap::Round } else { n.style.cap },
        join: n.style.join,
        dash: n.style.dash.clone(),
    });
    Some((ops.into_iter().collect(), alpha(color)))
}
