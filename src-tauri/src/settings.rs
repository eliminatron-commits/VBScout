//! Persisted user settings of the evaluation app (taken over from Stepwright).

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use vbs_i18n::Lang;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// UI language tag; `None` follows the operating system.
    pub ui_language: Option<String>,
}

impl Settings {
    /// Loads settings; a missing or unreadable file yields the defaults.
    pub fn load(path: &Path) -> Settings {
        fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Saves settings atomically (temporary file + rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
        fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temp, path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// Effective UI language: the explicit choice, else the OS locale, else English.
    pub fn ui_lang(&self, os_locale: Option<&str>) -> Lang {
        self.ui_language
            .as_deref()
            .and_then(Lang::from_tag)
            .or_else(|| os_locale.map(Lang::negotiate))
            .unwrap_or(Lang::En)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let settings = Settings { ui_language: Some("pl".into()) };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn language_resolution() {
        let system = Settings::default();
        assert_eq!(system.ui_lang(Some("de-CH")), Lang::De);
        assert_eq!(system.ui_lang(None), Lang::En);
        let explicit = Settings { ui_language: Some("pt-BR".into()) };
        assert_eq!(explicit.ui_lang(Some("de-DE")), Lang::PtBr);
    }
}
