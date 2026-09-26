//! Upgrades `result.json` of older schema versions to [`SCHEMA_VERSION`].
//!
//! To introduce schema version N+1: bump `SCHEMA_VERSION`, append a function
//! `vN_to_vN1` to [`MIGRATIONS`] and add a test with a version-N fixture.
//! Collectors are distributed widely and updated rarely, so the evaluation must
//! keep reading every schema version ever released.

use serde_json::Value;

use crate::error::FormatError;
use crate::model::SCHEMA_VERSION;

type Migration = fn(Value) -> Result<Value, FormatError>;

/// `MIGRATIONS[i]` upgrades a result from version `i + 1` to `i + 2`.
const MIGRATIONS: &[Migration] = &[];

const _: () = assert!(MIGRATIONS.len() as u32 + 1 == SCHEMA_VERSION, "one migration per schema version");

/// Brings `result` to the current schema version.
pub(crate) fn upgrade(mut result: Value) -> Result<Value, FormatError> {
    let version = result
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| FormatError::Invalid("schemaVersion is missing".into()))?;
    let version = u32::try_from(version).map_err(|_| FormatError::UnsupportedSchema(u32::MAX))?;
    if version > SCHEMA_VERSION {
        return Err(FormatError::NewerSchema { found: version, supported: SCHEMA_VERSION });
    }
    if version == 0 {
        return Err(FormatError::UnsupportedSchema(0));
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize - 1) {
        result = migration(result)?;
        result["schemaVersion"] = Value::from(index as u32 + 2);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_version_passes_unchanged() {
        let result = json!({ "schemaVersion": SCHEMA_VERSION, "x": 1 });
        assert_eq!(upgrade(result.clone()).unwrap(), result);
    }

    #[test]
    fn rejects_newer_and_invalid_versions() {
        let newer = upgrade(json!({ "schemaVersion": SCHEMA_VERSION + 1 }));
        assert!(matches!(newer, Err(FormatError::NewerSchema { .. })));
        assert!(matches!(upgrade(json!({ "schemaVersion": 0 })), Err(FormatError::UnsupportedSchema(0))));
        assert!(matches!(upgrade(json!({})), Err(FormatError::Invalid(_))));
    }
}
