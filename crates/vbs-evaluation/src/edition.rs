//! Editions of the evaluation and what they unlock.
//!
//! The limits are enforced in Rust – by the import, the assessment views and the report writers –
//! never only in the user interface:
//!
//! * **Free**: the finding list for up to `editions.free.maxMachines` machines (`product.json`);
//!   no PDF report, no migration hints, no effort estimates.
//! * **Organization** (one-time license): everything, unlimited machines of one organization; its
//!   name appears in the reports.
//! * **MSP** (yearly license): everything, unlimited customer environments; the provider's company
//!   name and logo appear in the reports. Expiry is checked against the system clock.
//!
//! Licenses are verified offline by `vbs-license` (signed keys arrive in phase 5); a missing or
//! expired license means the free edition.

use time::Date;
use vbs_license::{License, LicenseKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edition {
    Free { max_machines: u32 },
    Organization { name: String },
    Msp { company: String, expires: Date },
}

impl Edition {
    /// The free edition with the machine limit from the product configuration.
    pub fn free() -> Self {
        Edition::Free { max_machines: vbs_config::product().editions.free.max_machines }
    }

    /// The edition a verified license grants on `today` (the system date).
    pub fn from_license(license: Option<&License>, today: Date) -> Self {
        match license {
            Some(license) if !license.is_expired(today) => match &license.kind {
                LicenseKind::Organization { name } => Edition::Organization { name: name.clone() },
                LicenseKind::Msp { company, expires } => Edition::Msp { company: company.clone(), expires: *expires },
            },
            _ => Edition::free(),
        }
    }

    /// `free`, `organization` or `msp` (`edition.<kind>` translation keys).
    pub fn kind(&self) -> &'static str {
        match self {
            Edition::Free { .. } => "free",
            Edition::Organization { .. } => "organization",
            Edition::Msp { .. } => "msp",
        }
    }

    pub fn is_free(&self) -> bool {
        matches!(self, Edition::Free { .. })
    }

    /// How many distinct machines may be evaluated together; `None` = unlimited.
    pub fn machine_limit(&self) -> Option<usize> {
        match self {
            Edition::Free { max_machines } => Some(usize::try_from(*max_machines).unwrap_or(usize::MAX)),
            _ => None,
        }
    }

    /// The management PDF report.
    pub fn allows_pdf(&self) -> bool {
        !self.is_free()
    }

    /// Migration hints per finding.
    pub fn allows_hints(&self) -> bool {
        !self.is_free()
    }

    /// Rule-of-thumb effort estimates per finding and in total.
    pub fn allows_effort(&self) -> bool {
        !self.is_free()
    }

    /// The provider's own logo in the reports.
    pub fn allows_logo(&self) -> bool {
        matches!(self, Edition::Msp { .. })
    }

    /// Name printed in the reports: the organization or the service provider.
    pub fn licensee(&self) -> Option<&str> {
        match self {
            Edition::Free { .. } => None,
            Edition::Organization { name } => Some(name),
            Edition::Msp { company, .. } => Some(company),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn license(kind: LicenseKind) -> License {
        License { key_id: "k".into(), kind, issued: date!(2026 - 09 - 27) }
    }

    #[test]
    fn free_edition_limits() {
        let free = Edition::free();
        assert_eq!(free.kind(), "free");
        assert_eq!(free.machine_limit(), Some(25));
        assert!(!free.allows_pdf() && !free.allows_hints() && !free.allows_effort() && !free.allows_logo());
        assert_eq!(free.licensee(), None);
        assert_eq!(Edition::from_license(None, date!(2026 - 09 - 27)), free);
    }

    #[test]
    fn licenses_unlock_features() {
        let today = date!(2026 - 09 - 27);
        let organization =
            Edition::from_license(Some(&license(LicenseKind::Organization { name: "ACME GmbH".into() })), today);
        assert_eq!(organization.machine_limit(), None);
        assert!(organization.allows_pdf() && organization.allows_hints() && organization.allows_effort());
        assert!(!organization.allows_logo(), "the logo is part of the MSP license");
        assert_eq!(organization.licensee(), Some("ACME GmbH"));

        let msp = license(LicenseKind::Msp { company: "IT Service Nord".into(), expires: date!(2027 - 09 - 26) });
        let edition = Edition::from_license(Some(&msp), today);
        assert!(edition.allows_logo());
        assert_eq!(edition.licensee(), Some("IT Service Nord"));
        assert!(Edition::from_license(Some(&msp), date!(2027 - 09 - 26)).allows_pdf(), "valid on its last day");
        assert!(Edition::from_license(Some(&msp), date!(2027 - 09 - 27)).is_free(), "expired → free edition");
    }
}
