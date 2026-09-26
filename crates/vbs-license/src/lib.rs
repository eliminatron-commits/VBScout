//! License interface, taken over from Stepwright's `sw-license` and extended by
//! VBScout's license types.
//!
//! Keys are verified **offline only** – there is no activation server, no
//! machine counting online and no network access of any kind. Expiry is checked
//! against the system clock. Phase 5 adds the signed key format (Ed25519,
//! public key embedded in the app, shared with Stepwright's key service)
//! behind [`LicenseVerifier`]; until then [`UnavailableVerifier`] rejects every
//! key and the evaluation runs as the free edition. The collector is always
//! free and never needs a license.

use thiserror::Error;
use time::Date;

/// A successfully verified license.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    /// Short key identifier for support requests (not personal data).
    pub key_id: String,
    pub kind: LicenseKind,
    pub issued: Date,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseKind {
    /// One-time license for one organization: unlimited machines of that
    /// organization; its name appears in every report.
    Organization { name: String },
    /// Yearly license for a managed service provider: unlimited customer
    /// environments; the provider's company name (and logo) appear in reports.
    Msp { company: String, expires: Date },
}

impl License {
    /// Name printed in reports.
    pub fn licensee(&self) -> &str {
        match &self.kind {
            LicenseKind::Organization { name } => name,
            LicenseKind::Msp { company, .. } => company,
        }
    }

    /// Last day of validity; organization licenses do not expire.
    pub fn expires(&self) -> Option<Date> {
        match &self.kind {
            LicenseKind::Organization { .. } => None,
            LicenseKind::Msp { expires, .. } => Some(*expires),
        }
    }

    /// Whether the license has expired on `today` (the system date).
    pub fn is_expired(&self, today: Date) -> bool {
        self.expires().is_some_and(|expires| today > expires)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LicenseError {
    #[error("the license key is malformed")]
    Malformed,
    #[error("the license key has been tampered with or is not genuine")]
    InvalidSignature,
    #[error("the license key belongs to a different product")]
    WrongProduct,
    #[error("the license expired on {0}")]
    Expired(Date),
    #[error("license keys cannot be verified by this build")]
    Unavailable,
}

/// Verifies a license key completely offline.
pub trait LicenseVerifier: Send + Sync {
    fn verify(&self, key: &str, today: Date) -> Result<License, LicenseError>;
}

/// Placeholder until the signed key format exists: rejects every key.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableVerifier;

impl LicenseVerifier for UnavailableVerifier {
    fn verify(&self, _key: &str, _today: Date) -> Result<License, LicenseError> {
        Err(LicenseError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn placeholder_rejects_every_key() {
        assert_eq!(UnavailableVerifier.verify("VBS1-anything", date!(2026 - 09 - 26)), Err(LicenseError::Unavailable));
    }

    #[test]
    fn license_kinds() {
        let organization = License {
            key_id: "k1".into(),
            kind: LicenseKind::Organization { name: "ACME GmbH".into() },
            issued: date!(2026 - 09 - 26),
        };
        assert_eq!(organization.licensee(), "ACME GmbH");
        assert!(!organization.is_expired(date!(2099 - 01 - 01)));

        let msp = License {
            key_id: "k2".into(),
            kind: LicenseKind::Msp { company: "IT Service Nord".into(), expires: date!(2027 - 09 - 25) },
            issued: date!(2026 - 09 - 26),
        };
        assert_eq!(msp.licensee(), "IT Service Nord");
        assert!(!msp.is_expired(date!(2027 - 09 - 25)), "valid through the last day");
        assert!(msp.is_expired(date!(2027 - 09 - 26)));
    }
}
