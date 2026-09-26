//! Localized console output. Progress goes to stdout (suppressed with
//! `--quiet`), problems always go to stderr.

use std::fmt::Display;

use vbs_i18n::{Lang, t_args};

pub struct Console {
    lang: Lang,
    quiet: bool,
}

impl Console {
    pub fn new(lang: Lang, quiet: bool) -> Self {
        Self { lang, quiet }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Translates `key` with `args`.
    pub fn text(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        t_args(self.lang, key, args)
    }

    /// Progress and summary lines (hidden with `--quiet`).
    pub fn info(&self, key: &str, args: &[(&str, &dyn Display)]) {
        if !self.quiet {
            println!("{}", self.text(key, args));
        }
    }

    /// Problems – always shown.
    pub fn error(&self, key: &str, args: &[(&str, &dyn Display)]) {
        eprintln!("{}", self.text(key, args));
    }
}

/// Comma-separated list of the supported language tags.
pub fn language_list() -> String {
    Lang::ALL.iter().map(|lang| lang.tag()).collect::<Vec<_>>().join(", ")
}
