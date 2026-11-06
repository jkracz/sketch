//! Text shaping: applying a font's own rules for kerning, ligatures and contextual letterforms.

use super::{FaceId, Loaded, loaded};
use harfrust::{
    Direction, Feature, Script, ShapeOptions, ShapePlan, Shaper, ShaperData, Tag, UnicodeBuffer,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What shaping needs from a font. Plans are slow to build and depend only on these settings.
pub(super) struct ShapeData {
    pub(super) font: harfrust::FontRef<'static>,
    pub(super) data: ShaperData,
    plans: Mutex<HashMap<(Script, bool), Arc<ShapePlan>>>,
}

impl ShapeData {
    pub(super) fn new(bytes: &'static [u8]) -> Result<ShapeData, String> {
        let font = harfrust::FontRef::new(bytes).map_err(|e| e.to_string())?;
        let data = ShaperData::new(&font);
        Ok(ShapeData {
            font,
            data,
            plans: Mutex::default(),
        })
    }

    fn plan(&self, shaper: &Shaper, script: Script, ligatures: bool) -> Arc<ShapePlan> {
        let mut plans = self.plans.lock().unwrap();
        let build = || {
            let features = features(ligatures);
            let direction = Direction::LeftToRight;
            Arc::new(ShapePlan::new(
                shaper,
                direction,
                Some(script),
                None,
                &features,
            ))
        };
        plans
            .entry((script, ligatures))
            .or_insert_with(build)
            .clone()
    }
}

/// Spaced-out letters can't join, so as in CSS, letter spacing turns ligatures off.
fn features(ligatures: bool) -> Vec<Feature> {
    if ligatures {
        return Vec::new();
    }
    [b"liga", b"clig"]
        .map(|tag| Feature::new(Tag::new(tag), 0, ..))
        .to_vec()
}

/// One positioned glyph, in font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub gid: u16,
    /// Index of the first character this glyph came from.
    pub cluster: usize,
    pub advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Shape text in one face, in reading order. Right-to-left scripts stay left to right here,
/// because laying them out properly needs bidi support that text layout doesn't have.
pub fn shape(face: FaceId, text: &[char], ligatures: bool) -> Vec<Glyph> {
    let Loaded { shaping, .. } = loaded(face);
    let mut buffer = UnicodeBuffer::new();
    buffer.reserve(text.len());
    for (i, c) in text.iter().enumerate() {
        buffer.add(*c, i as u32);
    }
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    let shaper = shaping.data.shaper(&shaping.font).build();
    let plan = shaping.plan(&shaper, buffer.script(), ligatures);
    let features = features(ligatures);
    let options = ShapeOptions::new().plan(Some(&plan)).features(&features);
    let shaped = shaper.shape(buffer, options);
    let infos = shaped.glyph_infos().iter();
    infos
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| Glyph {
            gid: info.glyph_id as u16,
            cluster: info.cluster as usize,
            advance: pos.x_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::face_of;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn width(glyphs: &[Glyph]) -> i32 {
        glyphs.iter().map(|g| g.advance).sum()
    }

    #[test]
    fn kerning_pulls_pairs_together() {
        let glyphs = shape(0, &chars("AV"), true);
        let face = face_of(0);
        let plain: i32 = ['A', 'V']
            .iter()
            .map(|c| {
                face.glyph_hor_advance(face.glyph_index(*c).unwrap())
                    .unwrap() as i32
            })
            .sum();
        assert_eq!(glyphs.len(), 2);
        assert!(width(&glyphs) < plain, "{} < {plain}", width(&glyphs));
    }

    #[test]
    fn glyphs_keep_their_source_characters() {
        let glyphs = shape(0, &chars("abc"), true);
        let clusters: Vec<usize> = glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, [0, 1, 2]);
    }

    #[test]
    fn empty_text_shapes_to_nothing() {
        assert!(shape(0, &[], true).is_empty());
    }
}
