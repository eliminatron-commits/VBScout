//! Central product configuration.
//!
//! `product.json` at the repository root is the single source of truth for the
//! product name, identifiers, URLs, prices and edition limits. It is embedded at
//! compile time; nothing user-visible may hard-code these values, so the product
//! can be renamed or repriced in one place (see CLAUDE.md). The name is not
//! trademark-cleared yet – keep it swappable.

use std::sync::LazyLock;

use serde::Deserialize;

/// Raw contents of `product.json`, embedded at compile time.
pub const PRODUCT_JSON: &str = include_str!("../../../product.json");

static PRODUCT: LazyLock<Product> = LazyLock::new(|| {
    Product::parse(PRODUCT_JSON).unwrap_or_else(|problem| panic!("product.json is invalid: {problem}"))
});

/// Returns the embedded product configuration.
pub fn product() -> &'static Product {
    &PRODUCT
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    /// User-visible product name, e.g. "VBScout".
    pub name: String,
    /// Reverse-DNS application identifier (data directories, desktop integration).
    /// Must stay stable after the first public release.
    pub identifier: String,
    /// Public website, shown in reports and the about view. Neither the
    /// collector nor the app ever contacts it.
    pub website: String,
    pub support_email: String,
    /// Publisher of the release files (installer, winget); placeholder until the vendor is set.
    pub publisher: String,
    /// Download URL of a release file with `{version}` and `{file}` (winget manifests).
    pub release_url: String,
    /// Where translators find the catalogs (shown next to unreviewed languages).
    pub translations_url: String,
    pub result_file: ResultFileConfig,
    pub editions: Editions,
    pub pricing: Pricing,
    pub license: LicenseConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultFileConfig {
    /// File extension without the dot, e.g. "vbscout".
    pub extension: String,
    /// Media type written into every result file (`mimetype` entry and `result.json`).
    pub mime_type: String,
    /// Media types used under earlier product names that must stay readable.
    #[serde(default)]
    pub legacy_mime_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Editions {
    pub free: FreeEdition,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeEdition {
    /// Distinct machines the free evaluation merges; the collector is always free and unlimited.
    pub max_machines: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pricing {
    /// ISO 4217 currency code.
    pub currency: String,
    /// Organization license: one-time, unlimited machines of one organization.
    pub organization: Price,
    /// MSP license: yearly, unlimited customer environments, own branding.
    pub msp: Price,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    /// Amount in minor units (cents).
    pub amount_minor: u32,
    /// What one license covers: "organization" or "serviceProvider".
    pub per: String,
    /// "oneTime" (organization) or "yearly" (MSP).
    pub billing: String,
}

/// License keys (`vbs-license`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseConfig {
    /// Ed25519 public key of the key service, base64url (43 characters). `null` until the vendor
    /// has created the signing key (`tools/license-keys keygen`); such a build accepts no keys, and
    /// the release pipeline refuses to build it.
    pub public_key: Option<String>,
}

impl Product {
    /// Parses and validates a product configuration.
    pub fn parse(json: &str) -> Result<Self, String> {
        let product: Product = serde_json::from_str(json).map_err(|e| e.to_string())?;
        product.validate()?;
        Ok(product)
    }

    /// Checks the invariants other components rely on.
    pub fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if name.is_empty() || name.trim() != name || name.chars().count() > 40 {
            return Err(format!("name {name:?} must be 1–40 characters without surrounding spaces"));
        }
        let segments: Vec<&str> = self.identifier.split('.').collect();
        let valid_segment = |s: &&str| {
            s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        };
        if segments.len() < 2 || !segments.iter().all(valid_segment) {
            return Err(format!("identifier {:?} must be reverse-DNS (a.b.c)", self.identifier));
        }
        for (label, url) in [("website", &self.website), ("translationsUrl", &self.translations_url)] {
            if !url.starts_with("https://") {
                return Err(format!("{label} must be an https:// URL"));
            }
        }
        if self.publisher.trim().is_empty() || self.publisher.trim() != self.publisher {
            return Err("publisher must not be empty".into());
        }
        if !self.release_url.starts_with("https://")
            || !self.release_url.contains("{version}")
            || !self.release_url.contains("{file}")
        {
            return Err("releaseUrl must be an https:// URL with {version} and {file}".into());
        }
        if !self.support_email.contains('@') {
            return Err("supportEmail must be an e-mail address".into());
        }
        let ext = &self.result_file.extension;
        if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return Err(format!("resultFile.extension {ext:?} must be lowercase alphanumeric"));
        }
        for mime in std::iter::once(&self.result_file.mime_type).chain(&self.result_file.legacy_mime_types) {
            if !mime.starts_with("application/") || !mime.ends_with("+zip") {
                return Err(format!("media type {mime:?} must be application/…+zip"));
            }
        }
        if self.editions.free.max_machines == 0 {
            return Err("editions.free.maxMachines must be positive".into());
        }
        let currency = &self.pricing.currency;
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(format!("pricing.currency {currency:?} must be an ISO 4217 code"));
        }
        let expected = [
            ("organization", &self.pricing.organization, "organization", "oneTime"),
            ("msp", &self.pricing.msp, "serviceProvider", "yearly"),
        ];
        for (label, price, per, billing) in expected {
            if price.amount_minor == 0 || price.per != per || price.billing != billing {
                return Err(format!("pricing.{label} must be a positive {billing} price per {per}"));
            }
        }
        if let Some(key) = &self.license.public_key {
            let base64url = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
            if key.len() != 43 || !key.chars().all(base64url) {
                return Err("license.publicKey must be a base64url Ed25519 public key (43 characters) or null".into());
            }
        }
        Ok(())
    }

    /// Formats an amount in minor units for display, e.g. `19900` → `"199 EUR"`.
    pub fn format_price(&self, amount_minor: u32) -> String {
        let (major, minor) = (amount_minor / 100, amount_minor % 100);
        if minor == 0 {
            format!("{major} {}", self.pricing.currency)
        } else {
            format!("{major}.{minor:02} {}", self.pricing.currency)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> serde_json::Value {
        serde_json::from_str(PRODUCT_JSON).expect("product.json is JSON")
    }

    fn parse(value: &serde_json::Value) -> Result<Product, String> {
        Product::parse(&value.to_string())
    }

    #[test]
    fn embedded_configuration_is_valid() {
        let product = product();
        assert!(!product.name.is_empty());
        assert!(product.editions.free.max_machines > 0);
        assert_eq!(product.pricing.organization.billing, "oneTime");
        assert_eq!(product.pricing.msp.billing, "yearly");
    }

    #[test]
    fn rejects_invalid_values() {
        let cases: [(&str, serde_json::Value); 11] = [
            ("/publisher", " ".into()),
            ("/releaseUrl", "https://example.com/latest.exe".into()),
            ("/license/publicKey", "not a key".into()),
            ("/name", "".into()),
            ("/identifier", "VBScout".into()),
            ("/website", "http://insecure.example".into()),
            ("/resultFile/extension", "VB Scout".into()),
            ("/resultFile/mimeType", "application/json".into()),
            ("/editions/free/maxMachines", 0.into()),
            ("/pricing/organization/billing", "yearly".into()),
            ("/pricing/msp/billing", "oneTime".into()),
        ];
        for (pointer, bad) in cases {
            let mut value = valid();
            *value.pointer_mut(pointer).expect("pointer exists") = bad;
            assert!(parse(&value).is_err(), "{pointer} should be rejected");
        }
    }

    #[test]
    fn formats_prices() {
        let product = product();
        assert_eq!(product.format_price(19900), format!("199 {}", product.pricing.currency));
        assert_eq!(product.format_price(4950), format!("49.50 {}", product.pricing.currency));
    }
}
