//! Text layout and glyph outlines.
//!
//! Each line is shaped, so fonts apply their kerning and ligatures, and characters map onto
//! glyphs through clusters instead of one to one.
//!
//! Text is rendered as vector glyph outlines, so it stays crisp at every zoom level and
//! exports to SVG without font dependencies. Faces come from `crate::fonts`: bundled Inter
//! (SIL OFL) plus whatever is registered at runtime, never anything taken from Paper.

use crate::fonts::{self, google, resolve};
pub use crate::fonts::{FaceId, face_bytes, face_of};
use crate::geom::{V2, v2};
use crate::model::{CharStyle, Color, TextAlign, TextStyle};
use crate::vector::VPath;
use lyon::path::Path;
use lyon::tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use unicode_script::{Script, UnicodeScript};

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

/// Characters a font needn't draw. Shaping uses them and then hides them.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}')
}

/// The face and character to shape for `c`: from `face`, then `fallback`, then `face`'s '?'.
fn pick_glyph(face: FaceId, fallback: FaceId, c: char) -> (FaceId, char) {
    if is_invisible(c) || face_of(face).glyph_index(c).is_some() {
        (face, c)
    } else if face_of(fallback).glyph_index(c).is_some() {
        (fallback, c)
    } else {
        (face, '?')
    }
}

/// A glyph placed along its line, before alignment.
#[derive(Debug, Clone, Copy)]
struct LineGlyph {
    gid: u16,
    face: FaceId,
    char_index: usize,
    x: f64,
    /// Up from the baseline.
    y: f64,
    advance: f64,
}

/// Shaped text: its glyphs, and each character's advance. Characters in one glyph share it.
struct Shaped {
    glyphs: Vec<LineGlyph>,
    advances: Vec<f64>,
}

/// One line's or paragraph's text, ready to shape.
struct ShapeInput<'a> {
    /// The face and character to use for each source character.
    picks: &'a [(FaceId, char)],
    styles: &'a [CharStyle],
    letter_spacing: f64,
}

impl ShapeInput<'_> {
    /// The end of the run starting at `start`: one face, size and script, since shaping works on
    /// one of each. Spaces and punctuation belong to the script around them.
    fn run_end(&self, start: usize, end: usize) -> usize {
        let mut script = None;
        for i in start..end {
            let same = self.picks[i].0 == self.picks[start].0
                && self.styles[i].size == self.styles[start].size;
            let this = self.picks[i].1.script();
            let strong = !matches!(this, Script::Common | Script::Inherited | Script::Unknown);
            if !same || (strong && script.is_some_and(|s| s != this)) {
                return i;
            }
            if strong {
                script = Some(this);
            }
        }
        end
    }

    fn shape(&self, range: std::ops::Range<usize>) -> Shaped {
        let mut out = Shaped {
            glyphs: Vec::new(),
            advances: vec![0.0; range.len()],
        };
        let ligatures = self.letter_spacing == 0.0;
        let mut pen = 0.0;
        let mut start = range.start;
        while start < range.end {
            let end = self.run_end(start, range.end);
            let face = self.picks[start].0;
            let text: Vec<char> = self.picks[start..end].iter().map(|p| p.1).collect();
            let scale = self.styles[start].size / face_of(face).units_per_em() as f64;
            let glyphs = fonts::shape(face, &text, ligatures);
            let clusters = glyphs.chunk_by(|a, b| a.cluster == b.cluster);
            let mut rest = glyphs.as_slice();
            for cluster in clusters {
                rest = &rest[cluster.len()..];
                let first = cluster[0].cluster;
                let next = rest.first().map_or(text.len(), |g| g.cluster);
                let spacing = self.letter_spacing * (next - first) as f64;
                let last = cluster.len() - 1;
                let mut width = spacing;
                for (i, g) in cluster.iter().enumerate() {
                    let advance = g.advance as f64 * scale;
                    out.glyphs.push(LineGlyph {
                        gid: g.gid,
                        face,
                        char_index: start + first,
                        x: pen + width - spacing + g.x_offset as f64 * scale,
                        y: g.y_offset as f64 * scale,
                        advance: if i == last {
                            advance + spacing
                        } else {
                            advance
                        },
                    });
                    width += advance;
                }
                let each = width / (next - first) as f64;
                out.advances[start + first - range.start..start + next - range.start].fill(each);
                pen += width;
            }
            start = end;
        }
        out
    }
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
    let picks: Vec<(FaceId, char)> = (0..chars.len())
        .map(|i| pick_glyph(faces[i].0, faces[i].1, chars[i]))
        .collect();
    let input = ShapeInput {
        picks: &picks,
        styles: &styles,
        letter_spacing: ts.letter_spacing,
    };
    let base_face = resolve(ts.family.as_deref(), ts.base_weight(), ts.italic);
    // Break into lines: hard breaks on '\n', soft breaks at spaces when wrapping.
    let mut ranges: Vec<(usize, usize, Shaped)> = Vec::new();
    let mut start = 0;
    while start <= chars.len() {
        let hard_end = chars[start..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |p| start + p);
        let paragraph = input.shape(start..hard_end);
        let mut soft: Vec<(usize, usize)> = Vec::new();
        let mut line_start = start;
        if let Some(maxw) = max_width {
            let mut x = 0.0;
            let mut last_space: Option<usize> = None;
            let mut i = line_start;
            while i < hard_end {
                let a = paragraph.advances[i - start];
                if chars[i] == ' ' {
                    last_space = Some(i);
                }
                if x + a > maxw && i > line_start {
                    let brk = match last_space {
                        Some(s) if s > line_start => s,
                        _ => i,
                    };
                    let skip_space = chars.get(brk) == Some(&' ');
                    soft.push((line_start, brk));
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
        if soft.is_empty() {
            ranges.push((start, hard_end, paragraph));
        } else {
            // Letters only join within a line, so each line shapes again on its own.
            soft.push((line_start, hard_end));
            ranges.extend(soft.into_iter().map(|(s, e)| (s, e, input.shape(s..e))));
        }
        start = hard_end + 1;
    }

    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut max_line_w: f64 = 0.0;
    let mut top = 0.0;
    for (s, e, shaped) in ranges {
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
        for a in &shaped.advances {
            x += a;
            carets.push(x);
        }
        for g in shaped.glyphs {
            let st = styles[g.char_index];
            glyphs.push(PlacedGlyph {
                gid: g.gid,
                face: g.face,
                size: st.size,
                color: st.color,
                x: g.x,
                baseline: baseline - g.y,
                advance: g.advance,
                char_index: g.char_index,
                underline: st.underline,
                strike: st.strike,
            });
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

    fn ligature_style() -> TextStyle {
        crate::fonts::register(
            "Fixture Ligature",
            400,
            false,
            include_bytes!("../tests/fixtures/fonts/playfair-fi.ttf").to_vec(),
        )
        .unwrap();
        TextStyle {
            family: Some("Fixture Ligature".into()),
            ..TextStyle::default()
        }
    }

    fn laid_out(text: &str, ts: &TextStyle) -> TextLayout {
        layout_rich(text, 40.0, Color::INK, ts, None, TextAlign::Left)
    }

    #[test]
    fn ligatures_join_letters_and_keep_a_caret_for_each() {
        let l = laid_out("fi", &ligature_style());
        assert_eq!(l.glyphs.len(), 1, "one glyph for two letters");
        assert_eq!(l.glyphs[0].char_index, 0);
        let carets = &l.lines[0].carets;
        assert_eq!(carets.len(), 3);
        assert!((carets[1] - carets[2] / 2.0).abs() < 1e-9, "{carets:?}");
        assert_eq!(l.hit(v2(carets[1], 10.0)), 1);
    }

    #[test]
    fn letter_spacing_turns_ligatures_off() {
        let ts = TextStyle {
            letter_spacing: 2.0,
            ..ligature_style()
        };
        let spaced = laid_out("fi", &ts);
        assert_eq!(spaced.glyphs.len(), 2);
        let tight = laid_out("fi", &ligature_style());
        assert!(spaced.width > tight.width);
    }

    #[test]
    fn a_size_change_splits_a_ligature() {
        let mut ts = ligature_style();
        ts.apply_range(1, 2, |s| s.size = Some(60.0));
        assert_eq!(laid_out("fi", &ts).glyphs.len(), 2);
    }

    #[test]
    fn a_color_change_keeps_a_ligature() {
        let mut ts = ligature_style();
        ts.apply_range(1, 2, |s| s.color = Some(Color::BLACK));
        assert_eq!(laid_out("fi", &ts).glyphs.len(), 1);
    }

    #[test]
    fn a_soft_break_splits_a_ligature() {
        let ts = ligature_style();
        let whole = laid_out("fi", &ts).width;
        let l = layout_rich(
            "fi",
            40.0,
            Color::INK,
            &ts,
            Some(whole * 0.6),
            TextAlign::Left,
        );
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.glyphs.len(), 2);
    }

    #[test]
    fn kerning_narrows_text_and_moves_later_glyphs() {
        let plain = TextStyle::default();
        let (a, v) = (laid_out("A", &plain).width, laid_out("V", &plain).width);
        let l = laid_out("AV", &plain);
        assert!(l.width < a + v, "{} < {}", l.width, a + v);
        assert_eq!(l.lines[0].carets, [0.0, l.glyphs[1].x, l.width]);
    }

    #[test]
    fn letter_spacing_adds_to_every_character() {
        let ts = TextStyle {
            letter_spacing: 3.0,
            ..TextStyle::default()
        };
        let plain = laid_out("ab", &TextStyle::default());
        let spaced = laid_out("ab", &ts);
        assert!((spaced.width - plain.width - 6.0).abs() < 1e-9);
        assert_eq!(spaced.lines[0].carets.len(), 3);
    }

    #[test]
    fn runs_split_where_the_size_or_script_changes() {
        let mut ts = TextStyle::default();
        ts.apply_range(1, 2, |s| s.size = Some(80.0));
        let l = laid_out("abc", &ts);
        assert_eq!(l.glyphs.len(), 3);
        let sizes: Vec<f64> = l.glyphs.iter().map(|g| g.size).collect();
        assert_eq!(sizes, [40.0, 80.0, 40.0]);

        let chars: Vec<(FaceId, char)> = "a. б".chars().map(|c| (0, c)).collect();
        let styles = laid_out_styles(chars.len());
        let input = ShapeInput {
            picks: &chars,
            styles: &styles,
            letter_spacing: 0.0,
        };
        assert_eq!(
            input.run_end(0, chars.len()),
            3,
            "punctuation stays with Latin"
        );
    }

    fn laid_out_styles(n: usize) -> Vec<CharStyle> {
        let ts = TextStyle::default();
        (0..n).map(|i| ts.char_style(i, 16.0, Color::INK)).collect()
    }

    #[test]
    fn characters_no_font_has_show_a_question_mark_and_keep_a_caret() {
        let l = laid_out("a\u{AC00}b", &TextStyle::default());
        assert_eq!(l.glyphs.len(), 3);
        assert_eq!(l.lines[0].carets.len(), 4);
        let alone = laid_out("\u{AC00}", &TextStyle::default());
        let mark = laid_out("?", &TextStyle::default());
        assert_eq!(alone.glyphs[0].gid, mark.glyphs[0].gid);
    }

    #[test]
    fn invisible_joiners_take_no_space() {
        let plain = laid_out("ab", &TextStyle::default());
        let joined = laid_out("a\u{200D}b", &TextStyle::default());
        assert!((joined.width - plain.width).abs() < 1e-9);
        assert_eq!(joined.lines[0].carets.len(), 4);
    }

    #[test]
    fn glyphs_tessellate() {
        let gid = face_of(0).glyph_index('A').unwrap().0;
        let m = glyph_mesh(0, gid);
        assert!(!m.indices.is_empty());
        assert!(glyph_svg_path(gid, V2::ZERO, 10.0).starts_with('M'));
    }
}
