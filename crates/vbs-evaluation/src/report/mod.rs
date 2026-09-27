//! Reports: the management PDF ([`pdf`]) and the technical Excel list ([`xlsx`]).
//!
//! Both are built from an [`Assessment`](crate::assessment::Assessment) in one of the eight
//! languages; every text comes from the translation catalogs. What they contain depends on the
//! edition, enforced here and not in the user interface:
//!
//! * free edition – the Excel finding list without migration hints and effort; no PDF;
//! * licensed editions – hints, rule-of-thumb effort, the PDF, the licensee's name;
//! * MSP edition – additionally the provider's own logo.
//!
//! Effort values are always labelled as rules of thumb. Reports never contain more of a finding
//! than the result files do (short, masked excerpts of affected lines).

#[cfg(test)]
mod checks;
pub mod pdf;
pub mod text;
pub mod xlsx;

use thiserror::Error;
use time::OffsetDateTime;
use vbs_i18n::Lang;

use crate::edition::Edition;

/// Everything a report needs besides the assessment.
#[derive(Debug, Clone)]
pub struct ReportContext<'a> {
    pub lang: Lang,
    pub edition: &'a Edition,
    pub branding: &'a Branding,
    /// When the report is created.
    pub created: OffsetDateTime,
    /// Version of the evaluation that writes it.
    pub version: &'a str,
}

impl ReportContext<'_> {
    /// Migration hints are part of the report.
    pub fn hints(&self) -> bool {
        self.edition.allows_hints()
    }

    /// Rule-of-thumb effort is part of the report.
    pub fn effort(&self) -> bool {
        self.edition.allows_effort()
    }

    /// The licensee printed in the report (organization or service provider).
    pub fn licensee(&self) -> Option<&str> {
        self.edition.licensee()
    }

    /// The customer or environment the report covers.
    pub fn customer(&self) -> Option<&str> {
        self.branding.customer.as_deref().map(str::trim).filter(|name| !name.is_empty())
    }

    /// The provider's logo – MSP edition only.
    pub fn logo(&self) -> Option<&Logo> {
        self.edition.allows_logo().then_some(self.branding.logo.as_ref()).flatten()
    }
}

/// Names and logo in the report. The licensee's name comes from the license; the customer name
/// and the logo are settings of the evaluation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branding {
    /// Customer or environment the evaluation covers (e.g. the MSP's customer, a site).
    pub customer: Option<String>,
    /// Logo for the reports (used with an MSP license).
    pub logo: Option<Logo>,
}

/// A PNG or JPEG image, checked when it is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logo {
    bytes: Vec<u8>,
    format: LogoFormat,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogoError {
    #[error("the logo must be a PNG or JPEG image")]
    UnsupportedFormat,
    #[error("the logo is larger than {} MB", Logo::MAX_BYTES / (1024 * 1024))]
    TooLarge,
    #[error("the logo could not be read: {0}")]
    Invalid(String),
}

impl Logo {
    /// Largest accepted image file.
    pub const MAX_BYTES: usize = 2 * 1024 * 1024;

    /// Checks and decodes an image once, so a report never fails on it later.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Logo, LogoError> {
        if bytes.len() > Logo::MAX_BYTES {
            return Err(LogoError::TooLarge);
        }
        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            LogoFormat::Png
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            LogoFormat::Jpeg
        } else {
            return Err(LogoError::UnsupportedFormat);
        };
        let image = decode(&bytes, format).map_err(LogoError::Invalid)?;
        let (width, height) = image.size();
        if width == 0 || height == 0 {
            return Err(LogoError::Invalid("empty image".into()));
        }
        Ok(Logo { bytes, format, width, height })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn format(&self) -> LogoFormat {
        self.format
    }

    /// Size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn image(&self) -> Option<krilla::image::Image> {
        decode(&self.bytes, self.format).ok()
    }
}

fn decode(bytes: &[u8], format: LogoFormat) -> Result<krilla::image::Image, String> {
    let data: krilla::Data = bytes.to_vec().into();
    match format {
        LogoFormat::Png => krilla::image::Image::from_png(data, true),
        LogoFormat::Jpeg => krilla::image::Image::from_jpeg(data, true),
    }
}

/// Why a report could not be written.
#[derive(Debug, Error)]
pub enum ReportError {
    #[error("the PDF report requires a license")]
    NotLicensed,
    #[error("the Excel file could not be written: {0}")]
    Excel(#[from] rust_xlsxwriter::XlsxError),
    #[error("the PDF could not be written: {0}")]
    Pdf(String),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2×1 PNG (red, blue) written by Python's zlib – an independent encoder.
    pub(crate) const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x7B, 0x40, 0xE8, 0xDD, 0x00, 0x00, 0x00,
        0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0x00, 0x04, 0xFF, 0x01, 0x07, 0x00, 0x01, 0xFF,
        0x3D, 0x7D, 0x8C, 0x49, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn logos_are_checked_when_chosen() {
        let logo = Logo::from_bytes(PNG.to_vec()).unwrap();
        assert_eq!((logo.format(), logo.size()), (LogoFormat::Png, (2, 1)));
        assert_eq!(Logo::from_bytes(b"GIF89a....".to_vec()), Err(LogoError::UnsupportedFormat));
        assert!(matches!(Logo::from_bytes(PNG[..40].to_vec()), Err(LogoError::Invalid(_))));
        let mut large = PNG.to_vec();
        large.resize(Logo::MAX_BYTES + 1, 0);
        assert_eq!(Logo::from_bytes(large), Err(LogoError::TooLarge));
    }

    #[test]
    fn the_logo_needs_an_msp_license() {
        let branding =
            Branding { customer: Some("  Kunde A  ".into()), logo: Some(Logo::from_bytes(PNG.to_vec()).unwrap()) };
        let created = time::OffsetDateTime::UNIX_EPOCH;
        let organization = Edition::Organization { name: "ACME".into() };
        let context =
            ReportContext { lang: Lang::En, edition: &organization, branding: &branding, created, version: "1" };
        assert!(context.logo().is_none());
        assert_eq!(context.customer(), Some("Kunde A"));
        let msp = Edition::Msp { company: "IT Nord".into(), expires: time::macros::date!(2099 - 01 - 01) };
        let context = ReportContext { edition: &msp, ..context };
        assert!(context.logo().is_some());
        assert_eq!(context.licensee(), Some("IT Nord"));
    }
}
