//! The font faces text can use: the bundled Inter faces plus any registered at runtime.
//!
//! Text stores a family name and weight. `resolve` turns those into a `FaceId`, falling back
//! to Inter when the family isn't loaded, so a document keeps its fonts on a machine that
//! doesn't have them yet. Faces live for the whole process, which keeps `face_of` cheap and
//! `'static`, and ids stay valid for the glyph caches.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

pub mod google;

/// Index of a loaded face. The bundled faces come first, in `BUNDLED` order.
pub type FaceId = u16;

const INTER: &str = "Inter";

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever a face is added, so anything measured or drawn with fallback faces can redo it.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Bytes, weight and italic of each bundled face.
static BUNDLED: [(&[u8], u16, bool); 6] = [
    (crate::theme::INTER_REGULAR, 400, false),
    (include_bytes!("../assets/fonts/Inter-Bold.ttf"), 700, false),
    (
        include_bytes!("../assets/fonts/Inter-Italic.ttf"),
        400,
        true,
    ),
    (
        include_bytes!("../assets/fonts/Inter-BoldItalic.ttf"),
        700,
        true,
    ),
    (crate::theme::INTER_MEDIUM, 500, false),
    (crate::theme::INTER_SEMIBOLD, 600, false),
];

struct Loaded {
    bytes: &'static [u8],
    face: ttf_parser::Face<'static>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Style {
    weight: u16,
    italic: bool,
    id: FaceId,
}

struct Registry {
    faces: Vec<&'static Loaded>,
    /// Styles of each family, keyed by `family_key`.
    families: HashMap<String, Vec<Style>>,
}

impl Registry {
    fn add(
        &mut self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &'static [u8],
    ) -> Result<FaceId, String> {
        let face = ttf_parser::Face::parse(bytes, 0).map_err(|e| format!("{family}: {e}"))?;
        let id = self.faces.len() as FaceId;
        self.faces.push(Box::leak(Box::new(Loaded { bytes, face })));
        let style = Style { weight, italic, id };
        self.families
            .entry(family_key(family))
            .or_default()
            .push(style);
        Ok(id)
    }
}

fn family_key(family: &str) -> String {
    family.trim().to_ascii_lowercase()
}

fn registry() -> &'static RwLock<Registry> {
    static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r = Registry {
            faces: Vec::new(),
            families: HashMap::new(),
        };
        for (bytes, weight, italic) in BUNDLED {
            r.add(INTER, weight, italic, bytes)
                .expect("bundled font parses");
        }
        RwLock::new(r)
    })
}

/// The closest loaded face: italic must match first, then the nearest weight, then the heavier
/// of two equally near weights. Families that aren't loaded use Inter.
pub fn resolve(family: Option<&str>, weight: u16, italic: bool) -> FaceId {
    let r = registry().read().unwrap();
    let styles = family
        .and_then(|f| r.families.get(&family_key(f)))
        .or_else(|| r.families.get(&family_key(INTER)))
        .expect("Inter is bundled");
    styles
        .iter()
        .min_by_key(|s| {
            (
                s.italic != italic,
                s.weight.abs_diff(weight),
                Reverse(s.weight),
            )
        })
        .expect("a family has at least one style")
        .id
}

/// Make a font file usable as `family` at `weight`. Registering a style again keeps the first.
pub fn register(family: &str, weight: u16, italic: bool, bytes: Vec<u8>) -> Result<FaceId, String> {
    ttf_parser::Face::parse(&bytes, 0).map_err(|e| format!("{family}: {e}"))?;
    let mut r = registry().write().unwrap();
    let known = r
        .families
        .get(&family_key(family))
        .and_then(|styles| {
            styles
                .iter()
                .find(|s| s.weight == weight && s.italic == italic)
        })
        .map(|s| s.id);
    match known {
        Some(id) => Ok(id),
        None => {
            let id = r.add(family, weight, italic, Box::leak(bytes.into_boxed_slice()))?;
            GENERATION.fetch_add(1, Ordering::Relaxed);
            Ok(id)
        }
    }
}

/// Whether exactly this style is loaded.
pub fn has(family: &str, weight: u16, italic: bool) -> bool {
    let r = registry().read().unwrap();
    r.families.get(&family_key(family)).is_some_and(|styles| {
        styles
            .iter()
            .any(|s| s.weight == weight && s.italic == italic)
    })
}

/// The weights and slants loaded for `family`, lightest first (empty when it isn't loaded).
pub fn styles(family: &str) -> Vec<(u16, bool)> {
    let r = registry().read().unwrap();
    let mut out: Vec<(u16, bool)> = r
        .families
        .get(&family_key(family))
        .map(|styles| styles.iter().map(|s| (s.weight, s.italic)).collect())
        .unwrap_or_default();
    out.sort_unstable();
    out
}

fn loaded(id: FaceId) -> &'static Loaded {
    let r = registry().read().unwrap();
    r.faces.get(id as usize).copied().unwrap_or(r.faces[0])
}

pub fn face_of(id: FaceId) -> &'static ttf_parser::Face<'static> {
    &loaded(id).face
}

pub fn face_bytes(id: FaceId) -> &'static [u8] {
    loaded(id).bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inter limited to the letters A and B, as served by Google Fonts' `text=` parameter.
    pub const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/fonts/inter-ab.ttf");

    #[test]
    fn inter_resolves_to_the_nearest_bundled_face() {
        for (weight, italic, id) in [
            (400, false, 0),
            (700, false, 1),
            (400, true, 2),
            (700, true, 3),
            (500, false, 4),
            (600, false, 5),
            (100, false, 0),
            (900, false, 1),
            (500, true, 2),
        ] {
            assert_eq!(resolve(None, weight, italic), id, "{weight} {italic}");
        }
    }

    #[test]
    fn missing_families_use_inter() {
        assert_eq!(resolve(Some("No Such Font"), 700, true), 3);
        assert!(styles("No Such Font").is_empty());
    }

    #[test]
    fn registered_faces_resolve_by_family_and_weight() {
        let light = register("Fixture Sans", 300, false, FIXTURE.to_vec()).unwrap();
        let before = generation();
        let black = register("Fixture Sans", 900, false, FIXTURE.to_vec()).unwrap();
        assert_ne!(light, black);
        assert!(generation() > before);
        assert!(has("fixture sans", 900, false) && !has("Fixture Sans", 500, false));
        assert_eq!(resolve(Some("fixture sans"), 200, false), light);
        assert_eq!(resolve(Some(" Fixture Sans "), 800, false), black);
        assert_eq!(resolve(Some("Fixture Sans"), 300, true), light);
        assert_eq!(styles("Fixture Sans"), vec![(300, false), (900, false)]);
        assert!(face_of(light).glyph_index('A').is_some());
        assert_eq!(face_bytes(light), FIXTURE);
    }

    #[test]
    fn registering_a_style_twice_keeps_one_face() {
        let a = register("Fixture Twice", 400, false, FIXTURE.to_vec()).unwrap();
        let b = register("Fixture Twice", 400, false, FIXTURE.to_vec()).unwrap();
        assert_eq!(a, b);
        assert_eq!(styles("Fixture Twice").len(), 1);
    }

    #[test]
    fn bad_font_data_is_rejected() {
        assert!(register("Fixture Bad", 400, false, b"not a font".to_vec()).is_err());
        assert!(styles("Fixture Bad").is_empty());
    }
}
