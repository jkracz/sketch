//! Google Fonts: the catalog of families, and their files downloaded when text first uses them.
//!
//! The catalog is `assets/fonts/google-fonts.json`, written by `scripts/update-google-fonts.py`.
//! Font files come from the CSS API at fonts.googleapis.com, which needs no key. Without a
//! browser user agent it answers with plain TTF files, the format `ttf_parser` reads, with one
//! static file per weight. Files stay in a cache folder, so a font downloads once per machine.
//!
//! Nothing here runs until `start` is called, so tests and tools never touch the network.

use super::{has, register};
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CSS_API: &str = "https://fonts.googleapis.com/css2";
/// Where the CSS API serves files from. Anything else in a response is ignored.
const FILE_HOST: &str = "https://fonts.gstatic.com/";
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const WORKERS: usize = 4;
/// How long a failed download waits before text can ask for it again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Picker rows scroll past faster than they download, so old requests get dropped.
const MAX_QUEUED_PREVIEWS: usize = 64;
/// Height of a preview bitmap. The picker shows it at half size for sharp text on Retina.
const PREVIEW_PX: f32 = 36.0;
const MAX_PREVIEW_WIDTH: usize = 720;

#[derive(Debug, Deserialize)]
pub struct Family {
    pub family: String,
    pub category: String,
    pub weights: Vec<u16>,
    pub italics: Vec<u16>,
}

impl Family {
    /// The catalog style closest to a request: italic if the family has it, then nearest weight.
    pub fn nearest(&self, weight: u16, italic: bool) -> (u16, bool) {
        let italic = if self.weights.is_empty() {
            true
        } else {
            italic && !self.italics.is_empty()
        };
        let list = if italic { &self.italics } else { &self.weights };
        let best = list
            .iter()
            .copied()
            .min_by_key(|w| (w.abs_diff(weight), std::cmp::Reverse(*w)))
            .unwrap_or(400);
        (best, italic)
    }
}

/// Every family, most popular first.
pub fn catalog() -> &'static [Family] {
    static CATALOG: OnceLock<Vec<Family>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/fonts/google-fonts.json"))
            .expect("the bundled catalog parses")
    })
}

/// A family by name, ignoring case.
pub fn find(name: &str) -> Option<&'static Family> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let families = catalog().iter().enumerate();
        families
            .map(|(i, f)| (f.family.to_ascii_lowercase(), i))
            .collect()
    });
    index
        .get(&name.trim().to_ascii_lowercase())
        .map(|i| &catalog()[*i])
}

fn encoded(family: &str) -> String {
    family.replace(' ', "+")
}

fn font_css_url(family: &str, weight: u16, italic: bool) -> String {
    let family = encoded(family);
    if italic {
        format!("{CSS_API}?family={family}:ital,wght@1,{weight}")
    } else {
        format!("{CSS_API}?family={family}:wght@{weight}")
    }
}

/// A subset holding just the letters of the family's name.
fn preview_css_url(family: &str) -> String {
    let (family, text) = (encoded(family), encoded(family));
    format!("{CSS_API}?family={family}&text={text}")
}

/// The first font file a CSS response points at.
fn font_file_url(css: &str) -> Option<&str> {
    let start = css.find("url(")? + 4;
    let rest = &css[start..];
    let url = rest[..rest.find(')')?].trim_matches(|c| c == '"' || c == '\'');
    url.starts_with(FILE_HOST).then_some(url)
}

fn slug(family: &str) -> String {
    let lower = family.to_ascii_lowercase();
    lower
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", "Sketch")
        .call()
        .map_err(|e| e.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_FILE_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

type Fetch = fn(&str) -> Result<Vec<u8>, String>;

/// The font at `path`, downloading it through the CSS API when the cache doesn't have it.
fn cached_font(path: &Path, css_url: &str, fetch: Fetch) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = std::fs::read(path) {
        return Ok(bytes);
    }
    let css = fetch(css_url)?;
    let css = String::from_utf8_lossy(&css);
    let url = font_file_url(&css).ok_or("no font file in the response")?;
    let bytes = fetch(url)?;
    ttf_parser::Face::parse(&bytes, 0).map_err(|e| e.to_string())?;
    // The cache only speeds up the next launch, so a failed write doesn't fail the font.
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        if std::fs::write(&temp, &bytes).is_ok() && std::fs::rename(&temp, path).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
    Ok(bytes)
}

/// White text on transparent, for the picker to tint.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
}

/// `text` in the font, or None if the font is missing any of its letters.
fn rasterize(font: &[u8], text: &str) -> Option<Preview> {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    let font = FontRef::try_from_slice(font).ok()?;
    let scaled = font.as_scaled(PxScale::from(PREVIEW_PX));
    let height = scaled.height().ceil() as usize + 2;
    let mut x = 0.0;
    let mut glyphs = Vec::new();
    for c in text.chars() {
        let id = font.glyph_id(c);
        if id.0 == 0 && !c.is_whitespace() {
            return None;
        }
        glyphs
            .push(id.with_scale_and_position(PxScale::from(PREVIEW_PX), point(x, scaled.ascent())));
        x += scaled.h_advance(id);
    }
    let width = (x.ceil() as usize + 2).clamp(1, MAX_PREVIEW_WIDTH);
    let mut alpha = vec![0u8; width * height];
    for glyph in glyphs {
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|px, py, coverage| {
            let (px, py) = (
                bounds.min.x as i64 + px as i64,
                bounds.min.y as i64 + py as i64,
            );
            if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                let a = &mut alpha[py as usize * width + px as usize];
                *a = (*a).max((coverage * 255.0) as u8);
            }
        });
    }
    Some(Preview {
        width,
        height,
        alpha,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Style {
    family: &'static str,
    weight: u16,
    italic: bool,
}

/// Where a requested font stands. A font that's ready has no state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Downloading,
    Failed(Instant),
}

enum PreviewState {
    Queued,
    Ready(Preview),
    Failed,
}

enum Job {
    Font(Style),
    Preview(String),
}

#[derive(Default)]
struct Shared {
    fonts: HashMap<Style, Status>,
    font_jobs: VecDeque<Style>,
    previews: HashMap<String, PreviewState>,
    /// Newest last. Workers take the newest, which is what the picker is showing now.
    preview_jobs: Vec<String>,
}

struct Service {
    dir: PathBuf,
    fetch: Fetch,
    notify: Box<dyn Fn() + Send + Sync>,
    shared: Mutex<Shared>,
    wake: Condvar,
}

impl Service {
    fn font_path(&self, s: Style) -> PathBuf {
        let italic = if s.italic { "i" } else { "" };
        let name = format!("{}-{}{italic}.ttf", slug(s.family), s.weight);
        self.dir.join(name)
    }

    fn preview_path(&self, family: &str) -> PathBuf {
        self.dir
            .join("previews")
            .join(format!("{}.ttf", slug(family)))
    }

    /// Load `s` from the cache now, or queue its download.
    fn request(&self, s: Style) {
        {
            let shared = self.shared.lock().unwrap();
            match shared.fonts.get(&s) {
                Some(Status::Downloading) => return,
                Some(Status::Failed(at)) if at.elapsed() < RETRY_AFTER => return,
                _ => {}
            }
        }
        // Reading a cached file here means text never flashes Inter for a font this machine has.
        if let Ok(bytes) = std::fs::read(self.font_path(s))
            && register(s.family, s.weight, s.italic, bytes).is_ok()
        {
            return;
        }
        let mut shared = self.shared.lock().unwrap();
        if shared.fonts.insert(s, Status::Downloading) != Some(Status::Downloading) {
            shared.font_jobs.push_back(s);
            self.wake.notify_one();
        }
    }

    fn take_preview(&self, family: &'static str) -> Option<Preview> {
        let mut shared = self.shared.lock().unwrap();
        match shared.previews.get(family) {
            Some(PreviewState::Ready(_)) => {}
            Some(_) => return None,
            None => {
                shared.previews.insert(family.into(), PreviewState::Queued);
                shared.preview_jobs.push(family.into());
                if shared.preview_jobs.len() > MAX_QUEUED_PREVIEWS {
                    let old = shared.preview_jobs.remove(0);
                    shared.previews.remove(&old);
                }
                self.wake.notify_one();
                return None;
            }
        }
        match shared.previews.remove(family) {
            Some(PreviewState::Ready(p)) => Some(p),
            _ => None,
        }
    }

    fn next_job(shared: &mut Shared) -> Option<Job> {
        let font = shared.font_jobs.pop_front().map(Job::Font);
        font.or_else(|| shared.preview_jobs.pop().map(Job::Preview))
    }

    fn run(&self, job: Job) {
        match job {
            Job::Font(s) => {
                let url = font_css_url(s.family, s.weight, s.italic);
                let loaded = cached_font(&self.font_path(s), &url, self.fetch)
                    .and_then(|b| register(s.family, s.weight, s.italic, b));
                let mut shared = self.shared.lock().unwrap();
                match loaded {
                    Ok(_) => {
                        shared.fonts.remove(&s);
                    }
                    Err(e) => {
                        log::warn!("could not download {}: {e}", s.family);
                        shared.fonts.insert(s, Status::Failed(Instant::now()));
                    }
                }
            }
            Job::Preview(family) => {
                let url = preview_css_url(&family);
                let preview = cached_font(&self.preview_path(&family), &url, self.fetch)
                    .ok()
                    .and_then(|bytes| rasterize(&bytes, &family));
                let state = preview.map_or(PreviewState::Failed, PreviewState::Ready);
                self.shared.lock().unwrap().previews.insert(family, state);
            }
        }
        (self.notify)();
    }

    fn work(&self) {
        loop {
            let job = {
                let mut shared = self.shared.lock().unwrap();
                loop {
                    if let Some(job) = Self::next_job(&mut shared) {
                        break job;
                    }
                    shared = self.wake.wait(shared).unwrap();
                }
            };
            self.run(job);
        }
    }
}

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();

/// Start downloading fonts into `dir`. `notify` runs on a worker thread after each one finishes.
pub fn start(dir: PathBuf, notify: impl Fn() + Send + Sync + 'static) {
    let service = Arc::new(Service {
        dir,
        fetch: http_get,
        notify: Box::new(notify),
        shared: Mutex::default(),
        wake: Condvar::new(),
    });
    if SERVICE.set(service.clone()).is_err() {
        return;
    }
    for _ in 0..WORKERS {
        let service = service.clone();
        std::thread::spawn(move || service.work());
    }
}

fn style_for(family: &str, weight: u16, italic: bool) -> Option<Style> {
    let f = find(family)?;
    let (weight, italic) = f.nearest(weight, italic);
    Some(Style {
        family: &f.family,
        weight,
        italic,
    })
}

/// Make sure the closest style of `family` loads, if Google has the family.
pub fn request(family: &str, weight: u16, italic: bool) {
    if let Some(service) = SERVICE.get()
        && let Some(s) = style_for(family, weight, italic)
        && !has(s.family, s.weight, s.italic)
    {
        service.request(s);
    }
}

/// Whether the closest style of `family` is still on its way, or failed to arrive.
pub fn status(family: &str, weight: u16, italic: bool) -> Option<Status> {
    let service = SERVICE.get()?;
    let s = style_for(family, weight, italic)?;
    service.shared.lock().unwrap().fonts.get(&s).copied()
}

/// The picker's preview of a family's name, once. Until it's ready this queues it and returns
/// None. Previews reload from the cache, so the picker can drop them.
pub fn take_preview(family: &'static str) -> Option<Preview> {
    SERVICE.get()?.take_preview(family)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/fonts/inter-ab.ttf");

    fn fixture_fetch(url: &str) -> Result<Vec<u8>, String> {
        Ok(if url.starts_with(CSS_API) {
            format!("@font-face {{ src: url({FILE_HOST}s/font.ttf) format('truetype'); }}")
                .into_bytes()
        } else {
            FIXTURE.to_vec()
        })
    }

    fn offline_fetch(_: &str) -> Result<Vec<u8>, String> {
        Err("offline".into())
    }

    fn service(fetch: Fetch) -> Service {
        let dir = std::env::temp_dir().join(format!("sketch-fonts-{:016x}", fastrand::u64(..)));
        Service {
            dir,
            fetch,
            notify: Box::new(|| {}),
            shared: Mutex::default(),
            wake: Condvar::new(),
        }
    }

    fn run_all(s: &Service) {
        loop {
            let job = Service::next_job(&mut s.shared.lock().unwrap());
            match job {
                Some(job) => s.run(job),
                None => return,
            }
        }
    }

    fn style(family: &'static str) -> Style {
        Style {
            family,
            weight: 400,
            italic: false,
        }
    }

    #[test]
    fn the_catalog_loads_and_finds_families_by_any_case() {
        assert!(catalog().len() > 1000);
        assert_eq!(catalog()[0].family, find("ROBOTO").unwrap().family);
        assert!(find("No Such Font").is_none());
        assert!(
            find("Noto Color Emoji").is_none(),
            "color fonts are left out"
        );
    }

    #[test]
    fn nearest_prefers_the_slant_then_the_weight() {
        let f = Family {
            family: "F".into(),
            category: "serif".into(),
            weights: vec![400, 700],
            italics: vec![400],
        };
        assert_eq!(f.nearest(600, false), (700, false));
        assert_eq!(f.nearest(700, true), (400, true));
        let roman = Family {
            italics: vec![],
            ..f
        };
        assert_eq!(roman.nearest(400, true), (400, false));
        let italic_only = Family {
            weights: vec![],
            italics: vec![400],
            ..roman
        };
        assert_eq!(italic_only.nearest(700, false), (400, true));
    }

    #[test]
    fn urls_follow_the_css_api() {
        assert_eq!(
            font_css_url("Playfair Display", 700, false),
            "https://fonts.googleapis.com/css2?family=Playfair+Display:wght@700"
        );
        assert_eq!(
            font_css_url("Lora", 500, true),
            "https://fonts.googleapis.com/css2?family=Lora:ital,wght@1,500"
        );
        assert_eq!(
            preview_css_url("Open Sans"),
            "https://fonts.googleapis.com/css2?family=Open+Sans&text=Open+Sans"
        );
    }

    #[test]
    fn only_google_file_urls_are_followed() {
        let css = "src: url(https://fonts.gstatic.com/s/a.ttf) format('truetype');";
        assert_eq!(
            font_file_url(css),
            Some("https://fonts.gstatic.com/s/a.ttf")
        );
        assert_eq!(font_file_url("src: url(https://example.com/a.ttf);"), None);
        assert_eq!(font_file_url("no fonts here"), None);
    }

    #[test]
    fn a_requested_font_downloads_registers_and_stays_cached() {
        let s = service(fixture_fetch);
        let wanted = style("Fixture Download");
        s.request(wanted);
        assert_eq!(s.shared.lock().unwrap().fonts[&wanted], Status::Downloading);
        s.request(wanted);
        assert_eq!(
            s.shared.lock().unwrap().font_jobs.len(),
            1,
            "one job per font"
        );
        run_all(&s);
        assert!(has("Fixture Download", 400, false));
        assert!(s.shared.lock().unwrap().fonts.is_empty());
        assert_eq!(std::fs::read(s.font_path(wanted)).unwrap(), FIXTURE);

        // A new run reads the cache without touching the network.
        let again = Service {
            dir: s.dir.clone(),
            ..service(offline_fetch)
        };
        let cached = Style {
            weight: 500,
            ..wanted
        };
        std::fs::copy(s.font_path(wanted), again.font_path(cached)).unwrap();
        again.request(cached);
        assert!(has("Fixture Download", 500, false));
        assert!(again.shared.lock().unwrap().font_jobs.is_empty());
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    #[test]
    fn a_failed_download_waits_before_trying_again() {
        let s = service(offline_fetch);
        let wanted = style("Fixture Offline");
        s.request(wanted);
        run_all(&s);
        assert!(matches!(
            s.shared.lock().unwrap().fonts[&wanted],
            Status::Failed(_)
        ));
        s.request(wanted);
        assert!(s.shared.lock().unwrap().font_jobs.is_empty());

        let long_ago = Instant::now() - RETRY_AFTER - Duration::from_secs(1);
        s.shared
            .lock()
            .unwrap()
            .fonts
            .insert(wanted, Status::Failed(long_ago));
        s.request(wanted);
        assert_eq!(s.shared.lock().unwrap().font_jobs.len(), 1);
    }

    #[test]
    fn previews_draw_the_name_and_skip_fonts_without_its_letters() {
        let p = rasterize(FIXTURE, "ABBA").unwrap();
        assert!(p.width > 40 && p.height >= PREVIEW_PX as usize);
        assert_eq!(p.alpha.len(), p.width * p.height);
        assert!(p.alpha.iter().any(|a| *a > 200));
        assert!(rasterize(FIXTURE, "Abc").is_none());
    }

    #[test]
    fn previews_queue_once_and_old_requests_drop() {
        let s = service(fixture_fetch);
        assert_eq!(s.take_preview("AB"), None);
        assert_eq!(s.take_preview("AB"), None);
        assert_eq!(s.shared.lock().unwrap().preview_jobs.len(), 1);
        run_all(&s);
        assert!(s.take_preview("AB").is_some());
        assert!(
            s.shared.lock().unwrap().previews.is_empty(),
            "taken, not kept"
        );

        for f in catalog().iter().take(MAX_QUEUED_PREVIEWS + 5) {
            s.take_preview(&f.family);
        }
        let shared = s.shared.lock().unwrap();
        assert_eq!(shared.preview_jobs.len(), MAX_QUEUED_PREVIEWS);
        assert_eq!(shared.previews.len(), MAX_QUEUED_PREVIEWS);
        drop(shared);
        std::fs::remove_dir_all(&s.dir).unwrap();
    }
}
