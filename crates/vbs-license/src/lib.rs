//! License keys of the evaluation, taken over from Stepwright's `sw-license` interface and completed
//! by the signed key format (Stepwright itself had no key format yet – see `docs/licensing.md`).
//!
//! Keys are verified **offline only** – there is no activation server, no machine counting online
//! and no network access of any kind. Expiry is checked against the system clock. The collector is
//! always free and never needs a license.
//!
//! # Key format (version 1)
//!
//! `VBS1-<payload>.<signature>`: `payload` is a small JSON object, `signature` its Ed25519
//! signature (RFC 8032, over exactly the payload bytes); both base64url without padding.
//! Whitespace and line breaks inside a pasted key are ignored. The payload names the product code
//! (`vbs`, so a key of another product of the vendor is refused), the license type, the licensee
//! printed in reports, the issue date and – for MSP licenses – the last day of validity:
//!
//! ```json
//! {"v":1,"product":"vbs","id":"L-2K7Q9M4X","type":"msp","licensee":"IT Service Nord",
//!  "issued":"2026-09-27","expires":"2027-10-11"}
//! ```
//!
//! The public key is embedded from `product.json` (`license.publicKey`); the private key exists only
//! as a secret of the key service (`worker/`) and in the vendor's offline tool (`tools/license-keys`).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Date;
use time::macros::format_description;

/// Prefix of every key of this product (format version 1).
pub const KEY_PREFIX: &str = "VBS1-";
/// Product code inside the payload. An internal code like the `vbs-` crate prefix – independent of
/// the (swappable) product name, so a rename keeps existing keys valid.
pub const PRODUCT_CODE: &str = "vbs";
/// Current payload version.
pub const FORMAT_VERSION: u32 = 1;
/// Longest key accepted (a real key has about 300 characters).
const MAX_KEY_LEN: usize = 4096;
/// Longest licensee name printed in reports.
pub const MAX_LICENSEE_CHARS: usize = 120;

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

    /// `organization` or `msp` (the `type` of the key payload).
    pub fn kind_code(&self) -> &'static str {
        match self.kind {
            LicenseKind::Organization { .. } => "organization",
            LicenseKind::Msp { .. } => "msp",
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
    #[error("the license key was made for a newer version (format {0})")]
    UnsupportedVersion(u32),
    #[error("the license expired on {0}")]
    Expired(Date),
    #[error("license keys cannot be verified by this build")]
    Unavailable,
}

impl LicenseError {
    /// Stable code for the user interface (`license.error.<code>` translation keys).
    pub fn code(&self) -> &'static str {
        match self {
            LicenseError::Malformed => "malformed",
            LicenseError::InvalidSignature => "invalidSignature",
            LicenseError::WrongProduct => "wrongProduct",
            LicenseError::UnsupportedVersion(_) => "unsupportedVersion",
            LicenseError::Expired(_) => "expired",
            LicenseError::Unavailable => "unavailable",
        }
    }
}

/// Verifies a license key completely offline.
pub trait LicenseVerifier: Send + Sync {
    /// Checks format, product and signature – but not the expiry, so an expired license can still
    /// be shown with its end date.
    fn decode(&self, key: &str) -> Result<License, LicenseError>;

    /// [`decode`](Self::decode) plus the expiry check against `today` (the system date).
    fn verify(&self, key: &str, today: Date) -> Result<License, LicenseError> {
        let license = self.decode(key)?;
        match license.expires() {
            Some(expires) if license.is_expired(today) => Err(LicenseError::Expired(expires)),
            _ => Ok(license),
        }
    }
}

/// Used when a build has no public key configured: rejects every key.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableVerifier;

impl LicenseVerifier for UnavailableVerifier {
    fn decode(&self, _key: &str) -> Result<License, LicenseError> {
        Err(LicenseError::Unavailable)
    }
}

/// Verifies signed keys against one Ed25519 public key.
#[derive(Debug, Clone)]
pub struct Ed25519Verifier {
    public_key: VerifyingKey,
}

impl Ed25519Verifier {
    pub fn new(public_key: VerifyingKey) -> Self {
        Self { public_key }
    }

    /// Parses a public key written as base64url (32 bytes, as in `product.json`).
    pub fn from_base64(public_key: &str) -> Result<Self, LicenseError> {
        let bytes: [u8; 32] = URL_SAFE_NO_PAD
            .decode(public_key.trim())
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(LicenseError::Malformed)?;
        VerifyingKey::from_bytes(&bytes).map(Self::new).map_err(|_| LicenseError::Malformed)
    }
}

/// The verifier of this build: the public key from `product.json`, or [`UnavailableVerifier`] while
/// none is configured (development builds before the vendor created the signing key).
pub fn embedded_verifier() -> Box<dyn LicenseVerifier> {
    match vbs_config::product().license.public_key.as_deref().map(Ed25519Verifier::from_base64) {
        Some(Ok(verifier)) => Box::new(verifier),
        _ => Box::new(UnavailableVerifier),
    }
}

impl LicenseVerifier for Ed25519Verifier {
    fn decode(&self, key: &str) -> Result<License, LicenseError> {
        let key = normalize(key)?;
        let body = key.strip_prefix(KEY_PREFIX).ok_or_else(|| other_prefix(&key))?;
        let (payload, signature) = body.split_once('.').ok_or(LicenseError::Malformed)?;
        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| LicenseError::Malformed)?;
        let signature: [u8; 64] = URL_SAFE_NO_PAD
            .decode(signature)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(LicenseError::Malformed)?;
        self.public_key
            .verify_strict(&payload, &Signature::from_bytes(&signature))
            .map_err(|_| LicenseError::InvalidSignature)?;
        // Only signed content is interpreted from here on.
        Payload::parse(&payload)?.into_license()
    }
}

/// Removes whitespace a mail client or a line wrap may have added; refuses anything else.
fn normalize(key: &str) -> Result<String, LicenseError> {
    if key.len() > MAX_KEY_LEN {
        return Err(LicenseError::Malformed);
    }
    let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(LicenseError::Malformed);
    }
    Ok(key)
}

/// A key of another product (another prefix such as `SW1-`) or of a newer format (`VBS2-`).
fn other_prefix(key: &str) -> LicenseError {
    match key.split_once('-') {
        Some((prefix, _)) if prefix.len() > 3 && prefix.starts_with(&KEY_PREFIX[..3]) => {
            LicenseError::UnsupportedVersion(prefix[3..].parse().unwrap_or(0))
        }
        Some((prefix, _)) if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric()) => {
            LicenseError::WrongProduct
        }
        _ => LicenseError::Malformed,
    }
}

/// The signed JSON object of a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub v: u32,
    pub product: String,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub licensee: String,
    pub issued: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
}

impl Payload {
    fn parse(bytes: &[u8]) -> Result<Self, LicenseError> {
        // Look at the version first, so a newer format is reported as such.
        #[derive(Deserialize)]
        struct Version {
            v: u32,
        }
        let version: Version = serde_json::from_slice(bytes).map_err(|_| LicenseError::Malformed)?;
        if version.v != FORMAT_VERSION {
            return Err(LicenseError::UnsupportedVersion(version.v));
        }
        serde_json::from_slice(bytes).map_err(|_| LicenseError::Malformed)
    }

    /// The payload of `license` (used by the vendor's tool and in tests).
    pub fn of(license: &License) -> Self {
        Payload {
            v: FORMAT_VERSION,
            product: PRODUCT_CODE.into(),
            id: license.key_id.clone(),
            kind: license.kind_code().into(),
            licensee: license.licensee().into(),
            issued: format_date(license.issued),
            expires: license.expires().map(format_date),
        }
    }

    /// Validates the fields and turns them into a [`License`].
    pub fn into_license(self) -> Result<License, LicenseError> {
        if self.v != FORMAT_VERSION {
            return Err(LicenseError::UnsupportedVersion(self.v));
        }
        if self.product != PRODUCT_CODE {
            return Err(LicenseError::WrongProduct);
        }
        let id_ok = (1..=40).contains(&self.id.len())
            && self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let licensee = self.licensee.trim();
        let licensee_ok = !licensee.is_empty()
            && licensee == self.licensee
            && licensee.chars().count() <= MAX_LICENSEE_CHARS
            && !licensee.chars().any(char::is_control);
        if !id_ok || !licensee_ok {
            return Err(LicenseError::Malformed);
        }
        let issued = parse_date(&self.issued)?;
        let kind = match (self.kind.as_str(), self.expires.as_deref()) {
            ("organization", None) => LicenseKind::Organization { name: self.licensee },
            ("msp", Some(expires)) => {
                let expires = parse_date(expires)?;
                if expires < issued {
                    return Err(LicenseError::Malformed);
                }
                LicenseKind::Msp { company: self.licensee, expires }
            }
            _ => return Err(LicenseError::Malformed),
        };
        Ok(License { key_id: self.id, kind, issued })
    }
}

fn parse_date(text: &str) -> Result<Date, LicenseError> {
    Date::parse(text, format_description!("[year]-[month]-[day]")).map_err(|_| LicenseError::Malformed)
}

/// `YYYY-MM-DD`.
pub fn format_date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), u8::from(date.month()), date.day())
}

/// Signs a license (vendor side: the offline tool and tests; the key service does the same in
/// JavaScript). Not used by the app.
#[cfg(any(test, feature = "issue"))]
pub fn issue(signing_key: &ed25519_dalek::SigningKey, license: &License) -> Result<String, LicenseError> {
    use ed25519_dalek::Signer as _;
    let payload = Payload::of(license);
    // Refuse to sign what the verifier would refuse.
    payload.clone().into_license()?;
    let bytes = serde_json::to_vec(&payload).map_err(|_| LicenseError::Malformed)?;
    let signature = signing_key.sign(&bytes);
    Ok(format!("{KEY_PREFIX}{}.{}", URL_SAFE_NO_PAD.encode(&bytes), URL_SAFE_NO_PAD.encode(signature.to_bytes())))
}

/// base64url of a public key, as written into `product.json`.
pub fn encode_public_key(public_key: &VerifyingKey) -> String {
    URL_SAFE_NO_PAD.encode(public_key.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use time::macros::date;

    const TODAY: Date = date!(2026 - 09 - 27);

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn verifier(seed: u8) -> Ed25519Verifier {
        Ed25519Verifier::new(signing_key(seed).verifying_key())
    }

    fn organization() -> License {
        License { key_id: "L-ORG1".into(), kind: LicenseKind::Organization { name: "ACME GmbH".into() }, issued: TODAY }
    }

    fn msp() -> License {
        License {
            key_id: "L-MSP1".into(),
            kind: LicenseKind::Msp { company: "IT Service Nord".into(), expires: date!(2027 - 10 - 11) },
            issued: TODAY,
        }
    }

    /// Signs arbitrary payload bytes (to test what the verifier does with signed nonsense).
    fn sign_raw(seed: u8, payload: &[u8]) -> String {
        use ed25519_dalek::Signer as _;
        let signature = signing_key(seed).sign(payload);
        format!("{KEY_PREFIX}{}.{}", URL_SAFE_NO_PAD.encode(payload), URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }

    #[test]
    fn genuine_keys_verify() {
        for license in [organization(), msp()] {
            let key = issue(&signing_key(1), &license).unwrap();
            assert!(key.starts_with(KEY_PREFIX));
            assert_eq!(verifier(1).verify(&key, TODAY), Ok(license));
        }
    }

    #[test]
    fn pasted_keys_with_line_breaks_verify() {
        let key = issue(&signing_key(1), &msp()).unwrap();
        let wrapped: String = key
            .as_bytes()
            .chunks(60)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n  ");
        assert_eq!(verifier(1).verify(&format!("  {wrapped}\n"), TODAY), Ok(msp()));
    }

    #[test]
    fn msp_expiry_against_the_system_date() {
        let key = issue(&signing_key(1), &msp()).unwrap();
        assert!(verifier(1).verify(&key, date!(2027 - 10 - 11)).is_ok(), "valid on its last day");
        assert_eq!(verifier(1).verify(&key, date!(2027 - 10 - 12)), Err(LicenseError::Expired(date!(2027 - 10 - 11))));
        // decode still shows what the expired license was.
        assert_eq!(verifier(1).decode(&key), Ok(msp()));
    }

    #[test]
    fn tampered_keys_are_rejected() {
        let key = issue(&signing_key(1), &organization()).unwrap();
        let (payload, signature) = key[KEY_PREFIX.len()..].split_once('.').unwrap();

        // Another licensee or type in the payload, signature kept.
        let json = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        for forged in [
            json.replace("ACME GmbH", "Other Corp"),
            json.replace(r#""type":"organization""#, r#""type":"msp","expires":"2099-12-31""#),
            json.replace("2026-09-27", "2026-09-28"),
        ] {
            assert_ne!(forged, json);
            let forged_key = format!("{KEY_PREFIX}{}.{signature}", URL_SAFE_NO_PAD.encode(&forged));
            assert_eq!(verifier(1).verify(&forged_key, TODAY), Err(LicenseError::InvalidSignature), "{forged}");
        }

        // Every single changed character of payload or signature.
        let decoded = |key: &str| {
            let (payload, signature) = key[KEY_PREFIX.len()..].split_once('.').unwrap();
            (URL_SAFE_NO_PAD.decode(payload).ok(), URL_SAFE_NO_PAD.decode(signature).ok())
        };
        for position in KEY_PREFIX.len()..key.len() {
            let mut bytes = key.clone().into_bytes();
            if bytes[position] == b'.' {
                continue;
            }
            bytes[position] = if bytes[position] == b'A' { b'B' } else { b'A' };
            let changed = String::from_utf8(bytes).unwrap();
            // A change in the unused low bits of the last base64 character may decode to the same
            // bytes (or not decode at all); every change of the decoded bytes must be refused.
            if decoded(&changed) != decoded(&key) {
                assert!(verifier(1).verify(&changed, TODAY).is_err(), "position {position} accepted");
            }
        }

        // A genuine signature by another key (e.g. a self-made signing key).
        let foreign = issue(&signing_key(2), &organization()).unwrap();
        assert_eq!(verifier(1).verify(&foreign, TODAY), Err(LicenseError::InvalidSignature));
        // Signature of another genuine key.
        let other = issue(&signing_key(1), &msp()).unwrap();
        let swapped = format!("{KEY_PREFIX}{payload}.{}", other.split_once('.').unwrap().1);
        assert_eq!(verifier(1).verify(&swapped, TODAY), Err(LicenseError::InvalidSignature));
    }

    #[test]
    fn malformed_keys_are_rejected() {
        let v = verifier(1);
        for key in ["", "   ", "VBS1-", "VBS1-abc", "VBS1-abc.def", "VBS1-!!.??", "hello world", "VBS1-a.b.c"] {
            assert_eq!(v.verify(key, TODAY), Err(LicenseError::Malformed), "{key:?}");
        }
        assert_eq!(v.verify(&"A".repeat(MAX_KEY_LEN + 1), TODAY), Err(LicenseError::Malformed));
        assert_eq!(v.verify("SW1-abc.def", TODAY), Err(LicenseError::WrongProduct));
        assert_eq!(v.verify("VBS2-abc.def", TODAY), Err(LicenseError::UnsupportedVersion(2)));
    }

    #[test]
    fn signed_but_invalid_payloads_are_rejected() {
        let cases: [(&str, LicenseError); 10] = [
            (r#"{"v":2,"product":"vbs"}"#, LicenseError::UnsupportedVersion(2)),
            (
                r#"{"v":1,"product":"sw","id":"L-1","type":"organization","licensee":"A","issued":"2026-09-27"}"#,
                LicenseError::WrongProduct,
            ),
            (r#"not json"#, LicenseError::Malformed),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"organization","licensee":"","issued":"2026-09-27"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"organization","licensee":" A","issued":"2026-09-27"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"organization","licensee":"A\u0007","issued":"2026-09-27"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"msp","licensee":"A","issued":"2026-09-27"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"msp","licensee":"A","issued":"2026-09-27","expires":"2026-09-26"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L-1","type":"team","licensee":"A","issued":"2026-09-27"}"#,
                LicenseError::Malformed,
            ),
            (
                r#"{"v":1,"product":"vbs","id":"L 1","type":"organization","licensee":"A","issued":"27.09.2026"}"#,
                LicenseError::Malformed,
            ),
        ];
        for (payload, expected) in cases {
            assert_eq!(verifier(1).verify(&sign_raw(1, payload.as_bytes()), TODAY), Err(expected), "{payload}");
        }
    }

    #[test]
    fn issue_refuses_invalid_licenses() {
        let mut license = organization();
        license.kind = LicenseKind::Organization { name: "x".repeat(MAX_LICENSEE_CHARS + 1) };
        assert_eq!(issue(&signing_key(1), &license), Err(LicenseError::Malformed));
    }

    #[test]
    fn public_key_roundtrip_and_unavailable_verifier() {
        let public = encode_public_key(&signing_key(3).verifying_key());
        assert_eq!(public.len(), 43);
        let key = issue(&signing_key(3), &msp()).unwrap();
        assert_eq!(Ed25519Verifier::from_base64(&public).unwrap().verify(&key, TODAY), Ok(msp()));
        assert!(Ed25519Verifier::from_base64("too-short").is_err());
        assert_eq!(UnavailableVerifier.verify(&key, TODAY), Err(LicenseError::Unavailable));
    }

    #[test]
    fn license_kinds() {
        assert_eq!(organization().licensee(), "ACME GmbH");
        assert!(!organization().is_expired(date!(2099 - 01 - 01)));
        assert_eq!(msp().licensee(), "IT Service Nord");
        assert_eq!((organization().kind_code(), msp().kind_code()), ("organization", "msp"));
    }

    /// A key made by the key service's JavaScript (`worker/test/fixture.mjs`) with the published test
    /// signing key verifies here – both implementations produce the same format.
    #[test]
    fn key_from_the_key_service_verifies() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../worker/test/fixtures/issued.json")).unwrap();
        let verifier = Ed25519Verifier::from_base64(fixture["publicKey"].as_str().unwrap()).unwrap();
        for case in fixture["keys"].as_array().unwrap() {
            let license = verifier.verify(case["key"].as_str().unwrap(), TODAY).unwrap();
            assert_eq!(license.kind_code(), case["type"].as_str().unwrap());
            assert_eq!(license.licensee(), case["licensee"].as_str().unwrap());
            assert_eq!(license.key_id, case["id"].as_str().unwrap());
        }
    }
}
