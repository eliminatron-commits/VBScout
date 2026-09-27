//! Vendor tool for license keys – runs offline on the vendor's machine, never shipped.
//!
//! ```text
//! license-keys keygen --out <file>             new signing key (seed file, never overwritten)
//! license-keys public-key --key <file>         public key for product.json / wrangler.toml
//! license-keys issue --key <file> --type organization|msp --licensee <name>
//!              [--expires YYYY-MM-DD] [--issued YYYY-MM-DD] [--id L-XXXX]
//! license-keys inspect [--public-key <base64url>] <key>
//! ```
//!
//! The seed file is the only copy of the signing key besides the key service's Worker secret
//! `LICENSE_SIGNING_KEY`. Keep it out of the repository (see docs/licensing.md).

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use vbs_license::{
    Ed25519Verifier, License, LicenseKind, LicenseVerifier, Payload, encode_public_key, format_date, issue,
};

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("error: {problem}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage: license-keys keygen --out <file> | public-key --key <file> | \
issue --key <file> --type organization|msp --licensee <name> [--expires YYYY-MM-DD] [--issued YYYY-MM-DD] [--id <id>] | \
inspect [--public-key <base64url>] <key>";

fn run(args: Vec<std::ffi::OsString>) -> Result<String, String> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let command = match parser.next().map_err(|e| e.to_string())? {
        Some(Value(command)) => command.string().map_err(|e| e.to_string())?,
        _ => return Err(USAGE.into()),
    };
    let (mut out, mut key, mut kind, mut licensee, mut expires, mut issued, mut id, mut public, mut text) =
        (None, None, None, None, None, None, None, None, None);
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        let value = |parser: &mut lexopt::Parser| -> Result<String, String> {
            parser.value().map_err(|e| e.to_string())?.string().map_err(|e| e.to_string())
        };
        match arg {
            Long("out") => out = Some(PathBuf::from(value(&mut parser)?)),
            Long("key") => key = Some(PathBuf::from(value(&mut parser)?)),
            Long("type") => kind = Some(value(&mut parser)?),
            Long("licensee") => licensee = Some(value(&mut parser)?),
            Long("expires") => expires = Some(parse_date(&value(&mut parser)?)?),
            Long("issued") => issued = Some(parse_date(&value(&mut parser)?)?),
            Long("id") => id = Some(value(&mut parser)?),
            Long("public-key") => public = Some(value(&mut parser)?),
            Value(value) if text.is_none() => text = Some(value.string().map_err(|e| e.to_string())?),
            other => return Err(format!("{}\n{USAGE}", other.unexpected())),
        }
    }
    match command.as_str() {
        "keygen" => keygen(&out.ok_or("keygen needs --out <file>")?),
        "public-key" => {
            Ok(encode_public_key(&read_seed(&key.ok_or("public-key needs --key <file>")?)?.verifying_key()))
        }
        "issue" => {
            let signing_key = read_seed(&key.ok_or("issue needs --key <file>")?)?;
            let licensee = licensee.ok_or("issue needs --licensee <name>")?;
            let issued = issued.unwrap_or_else(|| OffsetDateTime::now_utc().date());
            let kind = match (kind.as_deref(), expires) {
                (Some("organization"), None) => LicenseKind::Organization { name: licensee },
                (Some("organization"), Some(_)) => return Err("organization licenses do not expire".into()),
                (Some("msp"), Some(expires)) => LicenseKind::Msp { company: licensee, expires },
                (Some("msp"), None) => return Err("msp licenses need --expires YYYY-MM-DD".into()),
                _ => return Err("--type must be organization or msp".into()),
            };
            let license = License { key_id: id.unwrap_or_else(new_key_id), kind, issued };
            let key = issue(&signing_key, &license).map_err(|e| format!("cannot issue: {e}"))?;
            // Self-check with the public key of this signing key.
            Ed25519Verifier::new(signing_key.verifying_key()).decode(&key).map_err(|e| e.to_string())?;
            Ok(key)
        }
        "inspect" => inspect(&text.ok_or("inspect needs a key")?, public.as_deref()),
        _ => Err(USAGE.into()),
    }
}

fn keygen(out: &Path) -> Result<String, String> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no system randomness: {e}"))?;
    let signing_key = SigningKey::from_bytes(&seed);
    // create_new: an existing key file is never overwritten.
    let mut file =
        OpenOptions::new().write(true).create_new(true).open(out).map_err(|e| format!("{}: {e}", out.display()))?;
    file.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(seed)).as_bytes()).map_err(|e| e.to_string())?;
    let public = encode_public_key(&signing_key.verifying_key());
    Ok(format!(
        "signing key written to {}\n\
         public key: {public}\n\n\
         next steps (docs/licensing.md):\n\
         1. product.json → \"license\": {{ \"publicKey\": \"{public}\" }}\n\
         2. worker/wrangler.toml → LICENSE_PUBLIC_KEY = \"{public}\"\n\
         3. npx wrangler secret put LICENSE_SIGNING_KEY  (paste the content of the key file)\n\
         4. store the key file offline (password manager); never commit it",
        out.display()
    ))
}

fn read_seed(path: &Path) -> Result<SigningKey, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let seed: [u8; 32] = URL_SAFE_NO_PAD
        .decode(text.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| format!("{}: not a signing key (base64url, 32 bytes)", path.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn inspect(key: &str, public: Option<&str>) -> Result<String, String> {
    let public = match public {
        Some(public) => public.to_owned(),
        None => vbs_config::product()
            .license
            .public_key
            .clone()
            .ok_or("product.json has no license.publicKey – pass --public-key <base64url>".to_owned())?,
    };
    let verifier = Ed25519Verifier::from_base64(&public).map_err(|e| e.to_string())?;
    let today = OffsetDateTime::now_utc().date();
    let license = verifier.decode(key).map_err(|e| format!("rejected: {e}"))?;
    let payload = Payload::of(&license);
    let status = match verifier.verify(key, today) {
        Ok(_) => "valid".to_owned(),
        Err(e) => e.to_string(),
    };
    Ok(format!(
        "id: {}\ntype: {}\nlicensee: {}\nissued: {}\nexpires: {}\nstatus: {status}",
        payload.id,
        payload.kind,
        payload.licensee,
        payload.issued,
        license.expires().map(format_date).unwrap_or_else(|| "never".into())
    ))
}

fn parse_date(text: &str) -> Result<Date, String> {
    Date::parse(text, format_description!("[year]-[month]-[day]")).map_err(|_| format!("{text:?} is not YYYY-MM-DD"))
}

/// `L-` and eight characters of Crockford base32 (as the key service does).
fn new_key_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = OffsetDateTime::now_utc().unix_timestamp_nanos().to_le_bytes()[..8].try_into().unwrap_or_default();
    }
    let suffix: String = bytes.iter().map(|b| char::from(ALPHABET[usize::from(b % 32)])).collect();
    format!("L-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> Result<String, String> {
        run(args.iter().map(std::ffi::OsString::from).collect())
    }

    #[test]
    fn keygen_issue_inspect() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("signing.key");
        let key_path = key_file.to_str().unwrap();
        let output = run_args(&["keygen", "--out", key_path]).unwrap();
        let public = run_args(&["public-key", "--key", key_path]).unwrap();
        assert!(output.contains(&public));
        assert!(run_args(&["keygen", "--out", key_path]).is_err(), "never overwrites a key");

        let key = run_args(&[
            "issue",
            "--key",
            key_path,
            "--type",
            "msp",
            "--licensee",
            "IT Service Nord",
            "--issued",
            "2026-09-27",
            "--expires",
            "2099-10-11",
            "--id",
            "L-TEST0001",
        ])
        .unwrap();
        let shown = run_args(&["inspect", "--public-key", &public, &key]).unwrap();
        assert!(shown.contains("licensee: IT Service Nord") && shown.contains("status: valid"), "{shown}");
        assert!(shown.contains("expires: 2099-10-11") && shown.contains("id: L-TEST0001"));

        let org = run_args(&["issue", "--key", key_path, "--type", "organization", "--licensee", "ACME"]).unwrap();
        assert!(run_args(&["inspect", "--public-key", &public, &org]).unwrap().contains("expires: never"));

        // Wrong inputs.
        assert!(run_args(&["issue", "--key", key_path, "--type", "msp", "--licensee", "X"]).is_err());
        assert!(run_args(&["issue", "--key", key_path, "--type", "team", "--licensee", "X"]).is_err());
        assert!(run_args(&["issue", "--key", key_path, "--type", "organization", "--licensee", " X"]).is_err());
        let other = SigningKey::from_bytes(&[9; 32]);
        let foreign = encode_public_key(&other.verifying_key());
        assert!(run_args(&["inspect", "--public-key", &foreign, &key]).unwrap_err().starts_with("rejected"));
        assert!(new_key_id().starts_with("L-") && new_key_id().len() == 10);
    }
}
