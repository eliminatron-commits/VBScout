//! The collector writes a result file that is valid against the published schema
//! (`docs/result.schema.json`), and the evaluation reads it. On Windows the scan includes the system
//! sources of the machine running the test, so real task, registry, WMI and event log findings are
//! validated as well.

mod common;

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;

fn schema_validator() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(common::repo_root().join("docs/result.schema.json")).unwrap())
            .unwrap();
    jsonschema::options().should_validate_formats(true).build(&schema).expect("the schema itself is valid")
}

/// A path in a form that compares across `\\?\` prefixes, separators and case.
fn comparable(path: &str) -> String {
    path.trim_start_matches(r"\\?\").replace('\\', "/").to_lowercase()
}

#[test]
fn result_file_is_schema_valid_and_readable_by_the_evaluation() {
    let base = tempfile::tempdir().unwrap();
    let (out_dir, cwd, temp) = (base.path().join("out"), base.path().join("cwd"), base.path().join("temp"));
    for dir in [&out_dir, &cwd, &temp] {
        fs::create_dir_all(dir).unwrap();
    }
    let scan = common::corpus().join("negative");
    let args =
        [OsStr::new("--path"), scan.as_os_str(), OsStr::new("--out"), out_dir.as_os_str(), OsStr::new("--quiet")];
    let output = common::run_collector(&args, &cwd, &temp);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let names = common::entries(&out_dir);
    assert_eq!(names.len(), 1);
    assert!(names[0].ends_with(&format!(".{}", vbs_core::file_extension())), "{names:?}");
    let path = out_dir.join(&names[0]);

    // Container layout: `mimetype` first and stored, then result.json.
    let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
    {
        let mut mimetype = archive.by_index(0).unwrap();
        assert_eq!(mimetype.name(), vbs_core::MIMETYPE_ENTRY);
        assert_eq!(mimetype.compression(), zip::CompressionMethod::Stored);
        let mut media_type = String::new();
        mimetype.read_to_string(&mut media_type).unwrap();
        assert_eq!(media_type, vbs_core::media_type());
    }
    let mut json = String::new();
    archive.by_name(vbs_core::RESULT_ENTRY).unwrap().read_to_string(&mut json).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let validator = schema_validator();
    let problems: Vec<String> =
        validator.iter_errors(&value).map(|e| format!("{} at {}", e, e.instance_path())).collect();
    assert!(problems.is_empty(), "schema violations: {problems:#?}");

    // The shared reader accepts it and knows every value in it …
    let loaded = vbs_core::read_file(&path).unwrap();
    assert_eq!(loaded.unknown_values, 0);
    let result = loaded.result;
    // The negative collection holds nothing to report; findings can only come from the system
    // sources (Windows), and the schema check above covers them.
    let root = comparable(&scan.to_string_lossy());
    let from_corpus: Vec<&str> = result
        .findings
        .iter()
        .map(|finding| finding.location.path.as_str())
        .filter(|path| comparable(path).starts_with(&root))
        .collect();
    assert!(from_corpus.is_empty(), "findings in the negative collection: {from_corpus:#?}");
    if !cfg!(windows) {
        assert!(result.findings.is_empty(), "no system sources outside Windows: {:#?}", result.findings);
    }
    assert_eq!(result.scope.paths.len(), 1);
    assert!(!result.scope.local_drives);
    assert!(result.coverage.sources.iter().any(|source| source.source == "files.paths" && source.entries > 0));

    // … and so does the evaluation's import.
    let loaded = vbs_evaluation::import::LoadedState::default();
    let batch = vbs_evaluation::import::import(std::slice::from_ref(&out_dir), &loaded, None);
    assert!(batch.errors.is_empty(), "{:?}", batch.errors);
    assert_eq!(batch.files.len(), 1);
    assert_eq!(batch.files[0].result.scan_id, result.scan_id);
}

#[test]
fn the_schema_rejects_what_the_format_forbids() {
    let validator = schema_validator();
    let example: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(common::repo_root().join("docs/examples/result.example.json")).unwrap(),
    )
    .unwrap();
    let problems: Vec<String> =
        validator.iter_errors(&example).map(|e| format!("{} at {}", e, e.instance_path())).collect();
    assert!(problems.is_empty(), "the documented example must be valid: {problems:#?}");
    // The example also deserialises into the model (schema and code agree).
    let parsed: vbs_core::ScanResult = serde_json::from_value(example.clone()).unwrap();
    assert!(!parsed.findings.is_empty());

    let mutations: [(&str, serde_json::Value); 5] = [
        ("/schemaVersion", 2.into()),
        ("/findings/0/rule", "R-1".into()),
        ("/findings/0/evidence/0/text", "x".repeat(241).into()),
        ("/machine/hostname", "".into()),
        ("/findings/0/unexpected", true.into()),
    ];
    for (pointer, bad) in mutations {
        let mut value = example.clone();
        match value.pointer_mut(pointer) {
            Some(slot) => *slot = bad,
            None => {
                let (parent, key) = pointer.rsplit_once('/').unwrap();
                value.pointer_mut(parent).unwrap().as_object_mut().unwrap().insert(key.into(), bad);
            }
        }
        assert!(!validator.is_valid(&value), "{pointer} should be rejected");
    }
}
