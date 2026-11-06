//! App-wide preferences, kept as JSON in the projects root.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILE: &str = "settings.json";

/// Which palette the app uses. `System` follows the OS and is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
}

impl Settings {
    pub fn path_in(root: &Path) -> PathBuf {
        root.join(FILE)
    }

    /// A missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::io::atomic::write(
            path,
            serde_json::to_string_pretty(self)
                .expect("settings serialize")
                .as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("sketch-rs-settings-{:016x}", fastrand::u64(..)))
            .join(FILE)
    }

    #[test]
    fn defaults_to_system() {
        assert_eq!(Settings::load(&temp_path()).appearance, Appearance::System);
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path();
        let s = Settings {
            appearance: Appearance::Dark,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn bad_values_fall_back_to_system() {
        let path = temp_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"appearance":"sepia"}"#).unwrap();
        assert_eq!(Settings::load(&path).appearance, Appearance::System);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
