//! Image assets, registered process-wide by content hash.
//!
//! Image fills reference an asset id (a hash of the encoded bytes). The registry keeps the
//! decoded pixels for rendering and the original bytes for saving and export. Projects store
//! their assets as `assets/<id>.<ext>`; copy/paste between projects carries only the id, and
//! the destination project writes the file on its next save.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub struct ImageAsset {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub rgba: image::RgbaImage,
    /// The original encoded file.
    pub bytes: Vec<u8>,
    /// "png", "jpg", ...
    pub ext: &'static str,
    /// Mip levels below `rgba`, built once on first use and shared by every renderer.
    mips: OnceLock<Vec<image::RgbaImage>>,
}

impl ImageAsset {
    /// `rgba`, then each mip level down to 1×1, so minified images stay smooth.
    pub fn mip_levels(&self) -> impl Iterator<Item = &image::RgbaImage> {
        let below = self.mips.get_or_init(|| mip_chain(&self.rgba));
        std::iter::once(&self.rgba).chain(below)
    }

    pub fn mime(&self) -> &'static str {
        match self.ext {
            "jpg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => "image/png",
        }
    }
}

/// Each level halves the one above, down to 1×1 (at most 14 levels below it). It goes through
/// `DynamicImage` because `imageops::resize` is generic, so calling it here would compile it
/// unoptimized into dev builds, where a phone screenshot took about a second.
fn mip_chain(base: &image::RgbaImage) -> Vec<image::RgbaImage> {
    let mut out: Vec<image::RgbaImage> = Vec::new();
    let mut cur = image::DynamicImage::ImageRgba8(base.clone());
    while (cur.width() > 1 || cur.height() > 1) && out.len() < 14 {
        let (w, h) = ((cur.width() / 2).max(1), (cur.height() / 2).max(1));
        cur = cur.resize_exact(w, h, image::imageops::FilterType::Triangle);
        out.push(cur.to_rgba8());
    }
    out
}

/// Build the mip chains of `assets` on background threads, so the first frame that shows an
/// image doesn't stall on it.
fn warm(assets: Vec<Arc<ImageAsset>>) {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
    let per = assets.len().div_ceil(threads).max(1);
    for chunk in assets.chunks(per) {
        let chunk = chunk.to_vec();
        let spawned = std::thread::Builder::new()
            .name("image mips".into())
            .spawn(move || {
                for a in chunk {
                    a.mip_levels().for_each(drop);
                }
            });
        if let Err(e) = spawned {
            log::warn!("image mips: {e}");
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<ImageAsset>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<ImageAsset>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// FNV-1a 64-bit content hash as 16 hex digits.
pub fn hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn ext_of(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::Gif) => "gif",
        Ok(image::ImageFormat::WebP) => "webp",
        _ => "png",
    }
}

/// Decode and register encoded image bytes; returns the asset id.
pub fn register(bytes: &[u8]) -> Result<String, String> {
    let (id, new) = decode(bytes)?;
    warm(new.into_iter().collect());
    Ok(id)
}

/// Register `bytes`; also returns the asset when it wasn't registered before.
fn decode(bytes: &[u8]) -> Result<(String, Option<Arc<ImageAsset>>), String> {
    let id = hash(bytes);
    if registry().lock().unwrap().contains_key(&id) {
        return Ok((id, None));
    }
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("unsupported image: {e}"))?
        .to_rgba8();
    let a = ImageAsset {
        id: id.clone(),
        width: img.width(),
        height: img.height(),
        rgba: img,
        bytes: bytes.to_vec(),
        ext: ext_of(bytes),
        mips: OnceLock::new(),
    };
    let a = Arc::new(a);
    registry().lock().unwrap().insert(id.clone(), a.clone());
    Ok((id, Some(a)))
}

/// Register raw pixels (generated images); encodes them as PNG.
pub fn register_rgba(img: &image::RgbaImage) -> String {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png encodes");
    register(&out.into_inner()).expect("own png decodes")
}

pub fn get(id: &str) -> Option<Arc<ImageAsset>> {
    registry().lock().unwrap().get(id).cloned()
}

/// Register every image in a project's `assets/` folder.
pub fn load_dir(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut new = Vec::new();
    for e in rd.flatten() {
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        match decode(&bytes) {
            Ok((_, a)) => new.extend(a),
            Err(err) => log::warn!("{}: {err}", e.path().display()),
        }
    }
    warm(new);
}

/// Write the given assets into `dir` (skipping ones already there).
pub fn save_to_dir(dir: &Path, ids: impl IntoIterator<Item = String>) -> std::io::Result<()> {
    for id in ids {
        let Some(a) = get(&id) else { continue };
        let path = dir.join(format!("{id}.{}", a.ext));
        if !path.exists() {
            std::fs::create_dir_all(dir)?;
            crate::io::atomic::write(&path, &a.bytes)?;
        }
    }
    Ok(())
}

/// Asset ids referenced by a document's image fills.
pub fn used_by(doc: &crate::model::Document) -> Vec<String> {
    used_by_nodes(doc.nodes.values())
}

/// Asset ids referenced by these layers' image fills.
pub fn used_by_nodes<'a>(nodes: impl IntoIterator<Item = &'a crate::model::Node>) -> Vec<String> {
    let mut out: Vec<String> = nodes
        .into_iter()
        .flat_map(|n| n.style.fills.iter())
        .filter_map(|p| match p {
            crate::model::Paint::Image { asset, .. } => Some(asset.clone()),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Base64 (standard alphabet, padded), for data URLs in SVG export.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_levels_halve_to_one_pixel_and_are_built_once() {
        let img =
            image::RgbaImage::from_fn(10, 3, |x, y| image::Rgba([x as u8 * 20, y as u8, 0, 255]));
        let id = register_rgba(&img);
        let a = get(&id).unwrap();
        let sizes: Vec<_> = a.mip_levels().map(|l| l.dimensions()).collect();
        assert_eq!(sizes, [(10, 3), (5, 1), (2, 1), (1, 1)]);
        let first: Vec<*const image::RgbaImage> = a.mip_levels().map(|l| l as *const _).collect();
        let again: Vec<*const image::RgbaImage> = a.mip_levels().map(|l| l as *const _).collect();
        assert_eq!(first, again, "levels are cached on the asset");
    }

    #[test]
    fn base64_roundtrip() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(base64_decode(&base64(s)).unwrap(), s);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn register_dedups_by_content() {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]));
        let a = register_rgba(&img);
        let b = register_rgba(&img);
        assert_eq!(a, b);
        let asset = get(&a).unwrap();
        assert_eq!((asset.width, asset.height), (3, 2));
        assert!(register(b"not an image").is_err());
    }
}
