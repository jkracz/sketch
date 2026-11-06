//! Text layout and glyph outlines.
//!
//! Text is rendered as vector glyph outlines, so it stays crisp at every zoom level and
//! exports to SVG without font dependencies. Faces come from `crate::fonts`: bundled Inter
//! (SIL OFL) plus whatever is registered at runtime, never anything taken from Paper.

pub use crate::fonts::{FaceId, face_bytes, face_of};
use crate::fonts::{google, resolve};
use crate::geom::{V2, v2};
use crate::model::{CharStyle, Color, TextAlign, TextStyle};
use crate::vector::VPath;
use lyon::path::Path;
use lyon::tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

fn line_height_in(face: FaceId, font_size: f64) -> f64 {
    let f = face_of(face);
    let h = (f.ascender() as f64 - f.descender() as f64 + f.line_gap() as f64)
        / f.units_per_em() as f64;
    font_size * h.max(1.1)
}

fn ascent_in(face: FaceId, font_size: f64) -> f64 {
    let f = face_of(face);
    f.ascender() as f64 / f.units_per_em() as f64 * font_size
}

pub fn line_height(font_size: f64) -> f64 {
    line_height_in(0, font_size)
}

pub fn ascent(font_size: f64) -> f64 {
    ascent_in(0, font_size)
}

#[derive(Debug, Clone, Copy)]
pub struct PlacedGlyph {
    pub gid: u16,
    pub face: FaceId,
    pub size: f64,
    pub color: Color,
    /// Pen position of the glyph origin (baseline), relative to the text box.
    pub x: f64,
    pub baseline: f64,
    pub advance: f64,
    /// Char index into the source string this glyph comes from.
    pub char_index: usize,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub top: f64,
    pub height: f64,
    pub baseline: f64,
    pub width: f64,
    /// Char index range `[start, end)`; `end` excludes the line break.
    pub start: usize,
    pub end: usize,
    /// Left x of each char boundary in the line (len = end - start + 1).
    pub carets: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub lines: Vec<Line>,
    pub width: f64,
    pub height: f64,
    pub font_size: f64,
}

/// The glyph for `c` and its advance: from `face`, then `fallback`, then `face`'s '?'.
fn advance_in(face: FaceId, fallback: FaceId, c: char, font_size: f64) -> (FaceId, u16, f64) {
    let (face, gid) = [face, fallback]
        .into_iter()
        .find_map(|id| Some((id, face_of(id).glyph_index(c)?.0)))
        .or_else(|| Some((face, face_of(face).glyph_index('?')?.0)))
        .unwrap_or((face, 0));
    let f = face_of(face);
    let adv = f.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f64;
    (face, gid, adv / f.units_per_em() as f64 * font_size)
}

#[cfg(test)]
fn advance(c: char, font_size: f64) -> (u16, f64) {
    let (_, gid, adv) = advance_in(0, 0, c, font_size);
    (gid, adv)
}

/// Lay out plain text with optional wrapping at `max_width`.
pub fn layout(text: &str, font_size: f64, max_width: Option<f64>, align: TextAlign) -> TextLayout {
    layout_rich(
        text,
        font_size,
        Color::INK,
        &TextStyle::default(),
        max_width,
        align,
    )
}

/// Lay out text whose characters may each have their own family, weight, slant, size and color.
pub fn layout_rich(
    text: &str,
    font_size: f64,
    color: Color,
    ts: &TextStyle,
    max_width: Option<f64>,
    align: TextAlign,
) -> TextLayout {
    let chars: Vec<char> = text.chars().collect();
    let styles: Vec<CharStyle> = (0..chars.len())
        .map(|i| ts.char_style(i, font_size, color))
        .collect();
    let ls = ts.letter_spacing;
    // The face each character asks for, and the Inter face for glyphs that face doesn't have.
    // Runs share a style, so resolve only when it changes.
    let mut last = None;
    let mut wanted = (0, 0);
    let faces: Vec<(FaceId, FaceId)> = styles
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let key = (ts.family_at(i), s.weight, s.italic);
            if last != Some(key) {
                // First, so a font already in the download cache is used from this layout on.
                google::request(key.0.unwrap_or("Inter"), s.weight, s.italic);
                wanted = (
                    resolve(key.0, s.weight, s.italic),
                    resolve(None, s.weight, s.italic),
                );
                last = Some(key);
            }
            wanted
        })
        .collect();
    let adv: Vec<(FaceId, u16, f64)> = (0..chars.len())
        .map(|i| {
            let (f, g, a) = advance_in(faces[i].0, faces[i].1, chars[i], styles[i].size);
            (f, g, a + ls)
        })
        .collect();
    let base_face = resolve(ts.family.as_deref(), ts.base_weight(), ts.italic);
    // Break into lines: hard breaks on '\n', soft breaks at spaces when wrapping.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start <= chars.len() {
        let hard_end = chars[start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |p| start + p);
        let mut line_start = start;
        if let Some(maxw) = max_width {
            let mut x = 0.0;
            let mut last_space: Option<usize> = None;
            let mut i = line_start;
            while i < hard_end {
                let a = adv[i].2;
                if chars[i] == ' ' {
                    last_space = Some(i);
                }
                if x + a > maxw && i > line_start {
                    let brk = match last_space {
                        Some(s) if s > line_start => s,
                        _ => i,
                    };
                    let skip_space = chars.get(brk) == Some(&' ');
                    ranges.push((line_start, brk));
                    line_start = if skip_space { brk + 1 } else { brk };
                    i = line_start;
                    x = 0.0;
                    last_space = None;
                    continue;
                }
                x += a;
                i += 1;
            }
        }
        ranges.push((line_start, hard_end));
        start = hard_end + 1;
    }

    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut max_line_w: f64 = 0.0;
    let mut top = 0.0;
    for (s, e) in ranges.iter().copied() {
        // Line metrics from the largest character on the line (or the one before an empty line).
        let largest = (s..e).fold(None, |m: Option<usize>, i| match m {
            Some(j) if styles[j].size >= styles[i].size => Some(j),
            _ => Some(i),
        });
        let (max_size, line_face) = match largest.or(s.checked_sub(1)) {
            Some(i) if i < styles.len() => (styles[i].size, faces[i].0),
            _ => (font_size, base_face),
        };
        let natural = line_height_in(line_face, max_size);
        let lh = ts.line_height.map_or(natural, |m| max_size * m);
        let baseline = top + ascent_in(line_face, max_size) + (lh - natural) * 0.5;
        let mut x = 0.0;
        let mut carets = vec![0.0];
        for i in s..e {
            let st = styles[i];
            let (face, gid, a) = adv[i];
            glyphs.push(PlacedGlyph {
                gid,
                face,
                size: st.size,
                color: st.color,
                x,
                baseline,
                advance: a,
                char_index: i,
                underline: st.underline,
                strike: st.strike,
            });
            x += a;
            carets.push(x);
        }
        max_line_w = max_line_w.max(x);
        lines.push(Line {
            top,
            height: lh,
            baseline,
            width: x,
            start: s,
            end: e,
            carets,
        });
        top += lh;
    }
    let box_w = max_width.unwrap_or(max_line_w);
    if align != TextAlign::Left {
        let k = if align == TextAlign::Center { 0.5 } else { 1.0 };
        for line in &mut lines {
            let dx = (box_w - line.width) * k;
            for c in &mut line.carets {
                *c += dx;
            }
            for g in glyphs
                .iter_mut()
                .filter(|g| g.char_index >= line.start && g.char_index < line.end)
            {
                g.x += dx;
            }
        }
    }
    TextLayout {
        glyphs,
        height: top,
        lines,
        width: max_line_w,
        font_size,
    }
}

impl TextLayout {
    /// Caret (x, top, height) for a char index into the source string.
    pub fn caret(&self, index: usize) -> (f64, f64, f64) {
        let i = self.line_of(index);
        let l = &self.lines[i];
        let k = index.clamp(l.start, l.end) - l.start;
        (l.carets[k], l.top, l.height)
    }

    /// Index of the line holding char boundary `index`.
    pub fn line_of(&self, index: usize) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            let next_start = self.lines.get(i + 1).map(|n| n.start);
            if index >= l.start && index <= l.end && next_start.is_none_or(|ns| index < ns) {
                return i;
            }
        }
        self.lines.len() - 1
    }

    /// Highlight rectangles (x, top, w, h) for the char range `[a, b)`.
    pub fn selection_rects(&self, a: usize, b: usize) -> Vec<(f64, f64, f64, f64)> {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = Vec::new();
        for l in &self.lines {
            let s = a.max(l.start);
            let e = b.min(l.end);
            if s > e || (s == e && !(a < l.start && b > l.end)) {
                continue;
            }
            let x0 = l.carets[s - l.start];
            let x1 = l.carets[e - l.start];
            let extra = if b > l.end && l.end < usize::MAX {
                4.0
            } else {
                0.0
            };
            out.push((x0, l.top, (x1 - x0 + extra).max(1.0), l.height));
        }
        out
    }

    /// Char index nearest to a local point.
    pub fn hit(&self, p: V2) -> usize {
        let li = self
            .lines
            .iter()
            .position(|l| p.y < l.top + l.height)
            .unwrap_or(self.lines.len() - 1);
        let l = &self.lines[li];
        let mut best = 0;
        for (i, x) in l.carets.iter().enumerate() {
            if (x - p.x).abs() < (l.carets[best] - p.x).abs() {
                best = i;
            }
        }
        l.start + best
    }
}

/// A glyph tessellated once in font units (y up).
pub struct GlyphMesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub fn glyph_mesh(face: FaceId, gid: u16) -> Arc<GlyphMesh> {
    type Cache = Mutex<HashMap<(FaceId, u16), Arc<GlyphMesh>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = cache.lock().unwrap().get(&(face, gid)) {
        return m.clone();
    }
    let mesh = Arc::new(tessellate_glyph(face, gid));
    cache.lock().unwrap().insert((face, gid), mesh.clone());
    mesh
}

fn tessellate_glyph(face: FaceId, gid: u16) -> GlyphMesh {
    let mut b = LyonOutline {
        b: Path::builder(),
        open: false,
    };
    face_of(face).outline_glyph(ttf_parser::GlyphId(gid), &mut b);
    if b.open {
        b.b.end(true);
    }
    let path = b.b.build();
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let tol = (face_of(face).units_per_em() as f64 / 2000.0) as f32;
    let _ = FillTessellator::new().tessellate_path(
        &path,
        &FillOptions::tolerance(tol).with_fill_rule(lyon::tessellation::FillRule::NonZero),
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
    );
    GlyphMesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

struct LyonOutline {
    b: lyon::path::path::Builder,
    open: bool,
}

impl ttf_parser::OutlineBuilder for LyonOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.b.end(true);
        }
        self.b.begin(lyon::math::point(x, y));
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.b.line_to(lyon::math::point(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.b
            .quadratic_bezier_to(lyon::math::point(x1, y1), lyon::math::point(x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.b.cubic_bezier_to(
            lyon::math::point(x1, y1),
            lyon::math::point(x2, y2),
            lyon::math::point(x, y),
        );
    }
    fn close(&mut self) {
        self.b.end(true);
        self.open = false;
    }
}

/// Outline of a glyph placed at `origin` (baseline) with `font_size`, y down.
pub fn glyph_path(face: FaceId, gid: u16, origin: V2, font_size: f64) -> VPath {
    struct B {
        p: VPath,
        o: V2,
        s: f64,
    }
    impl B {
        fn t(&self, x: f32, y: f32) -> V2 {
            self.o + v2(x as f64 * self.s, -(y as f64) * self.s)
        }
    }
    impl ttf_parser::OutlineBuilder for B {
        fn move_to(&mut self, x: f32, y: f32) {
            let q = self.t(x, y);
            self.p.move_to(q);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            let q = self.t(x, y);
            self.p.line_to(q);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            let (a, b) = (self.t(x1, y1), self.t(x, y));
            self.p.quad_to(a, b);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            let (a, b, c) = (self.t(x1, y1), self.t(x2, y2), self.t(x, y));
            self.p.cubic_to(a, b, c);
        }
        fn close(&mut self) {
            self.p.close();
        }
    }
    let f = face_of(face);
    let mut b = B {
        p: VPath::new(),
        o: origin,
        s: font_size / f.units_per_em() as f64,
    };
    f.outline_glyph(ttf_parser::GlyphId(gid), &mut b);
    b.p
}

/// SVG path data for a glyph (regular face).
pub fn glyph_svg_path(gid: u16, origin: V2, font_size: f64) -> String {
    glyph_path(0, gid, origin, font_size).to_svg()
}

/// Inner padding of sticky notes.
pub const STICKY_PAD: f64 = 16.0;

/// Where a node's text box starts inside the node.
pub fn text_offset(n: &crate::model::Node) -> V2 {
    match n.kind {
        crate::model::NodeKind::Sticky { .. } => v2(STICKY_PAD, STICKY_PAD),
        _ => V2::ZERO,
    }
}

/// Layout of a text node's (or sticky note's) content in its box (None for other kinds).
pub fn layout_node(n: &crate::model::Node) -> Option<TextLayout> {
    if let crate::model::NodeKind::Sticky { text, font_size } = &n.kind {
        // Shrink the font (down to 12) until the text fits, like Excalidraw's sticky notes.
        let w = (n.rect.w - 2.0 * STICKY_PAD).max(10.0);
        let h = (n.rect.h - 2.0 * STICKY_PAD).max(10.0);
        let mut size = *font_size;
        loop {
            let ratio = size / font_size;
            let mut ts = n.text_style.clone();
            for sp in &mut ts.spans {
                sp.size = sp.size.map(|z| z * ratio);
            }
            let l = layout_rich(text, size, Color::INK, &ts, Some(w), TextAlign::Left);
            if l.height <= h || size <= 12.0 {
                return Some(l);
            }
            size = (size - 2.0).max(12.0);
        }
    }
    let crate::model::NodeKind::Text {
        text,
        font_size,
        auto_width,
        align,
    } = &n.kind
    else {
        return None;
    };
    let color = n
        .style
        .fills
        .last()
        .map_or(Color::INK, |p| p.preview_color());
    Some(layout_rich(
        text,
        *font_size,
        color,
        &n.text_style,
        (!auto_width).then_some(n.rect.w),
        *align,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_basic_and_newlines() {
        let l = layout("Hello\nWorld!", 20.0, None, TextAlign::Left);
        assert_eq!(l.lines.len(), 2);
        assert!(l.width > 40.0 && l.width < 120.0, "{}", l.width);
        assert!((l.height - 2.0 * line_height(20.0)).abs() < 1e-9);
        assert_eq!(l.glyphs.len(), 11);
        assert_eq!(l.caret(0).0, 0.0);
        assert_eq!(l.caret(6).1, line_height(20.0));
        let empty = layout("", 20.0, None, TextAlign::Left);
        assert_eq!(empty.lines.len(), 1);
        assert_eq!(empty.caret(0), (0.0, 0.0, line_height(20.0)));
    }

    #[test]
    fn wrapping_respects_width() {
        let l = layout(
            "the quick brown fox jumps over the lazy dog",
            16.0,
            Some(80.0),
            TextAlign::Left,
        );
        assert!(l.lines.len() >= 3);
        for line in &l.lines {
            assert!(
                line.width <= 80.0 + 1e-6 || line.end - line.start == 1,
                "{line:?}"
            );
        }
    }

    #[test]
    fn hit_and_caret_agree() {
        let l = layout("abc def", 30.0, None, TextAlign::Left);
        for i in 0..=7 {
            let (x, top, h) = l.caret(i);
            assert_eq!(l.hit(v2(x, top + h * 0.5)), i);
        }
    }

    #[test]
    fn spans_pick_their_family_and_missing_glyphs_fall_back_to_inter() {
        let face = crate::fonts::register(
            "Fixture Layout",
            400,
            false,
            include_bytes!("../tests/fixtures/fonts/inter-ab.ttf").to_vec(),
        )
        .unwrap();
        let mut ts = TextStyle::default();
        ts.apply_range(1, 3, |s| s.family = Some("Fixture Layout".into()));
        let l = layout_rich("AAz", 20.0, Color::INK, &ts, None, TextAlign::Left);
        let got: Vec<FaceId> = l.glyphs.iter().map(|g| g.face).collect();
        assert_eq!(got, [0, face, 0]);
    }

    #[test]
    fn weight_picks_the_face_and_unloaded_families_keep_inter() {
        let mut ts = TextStyle {
            family: Some("Not Loaded".into()),
            ..TextStyle::default()
        };
        ts.set_weight(600);
        let l = layout_rich("A", 20.0, Color::INK, &ts, None, TextAlign::Left);
        assert_eq!(l.glyphs[0].face, 5);
    }

    #[test]
    fn line_metrics_follow_the_largest_characters_face() {
        // An empty layout in a family uses that family's metrics.
        let face = crate::fonts::register(
            "Fixture Metrics",
            400,
            false,
            include_bytes!("../tests/fixtures/fonts/inter-ab.ttf").to_vec(),
        )
        .unwrap();
        let ts = TextStyle {
            family: Some("Fixture Metrics".into()),
            ..TextStyle::default()
        };
        let empty = layout_rich("", 20.0, Color::INK, &ts, None, TextAlign::Left);
        assert_eq!(empty.lines[0].height, line_height_in(face, 20.0));
        let one = layout_rich("A", 20.0, Color::INK, &ts, None, TextAlign::Left);
        assert_eq!(one.lines[0].baseline, ascent_in(face, 20.0));
    }

    #[test]
    fn glyphs_tessellate() {
        let (gid, _) = advance('A', 10.0);
        let m = glyph_mesh(0, gid);
        assert!(!m.indices.is_empty());
        assert!(glyph_svg_path(gid, V2::ZERO, 10.0).starts_with('M'));
    }
}
