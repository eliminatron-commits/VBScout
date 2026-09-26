//! VBScript custom actions of installer packages: the cached packages of installed products
//! (system part: `%SystemRoot%\Installer`, found through the registry) and `.msi` files
//! anywhere else on the scanned drives (file part). Packages are read as files, never
//! through msi.dll.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use vbs_core::model::{Activation, Detail, FileFacts, Location, LocationKind, NotCheckableReason};
use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};
use vbs_core::views::{Bitness, Hive, SystemView, ViewError};

use super::{read_error, view_reason};
use crate::analysis::msi::{self, Package, ScriptAction, SourceKind};
use crate::analysis::script::{self, Language};
use crate::analysis::servicing;

#[derive(Default)]
pub struct InstallerPackages {
    /// `%SystemRoot%\Installer` once the system part has examined it: the file part leaves
    /// the cached packages there alone (no double findings).
    cache: OnceLock<PathBuf>,
}

static INFO: ModuleInfo = ModuleInfo {
    id: "msi-custom-action",
    system_source: Some("installer.packages"),
    rules: &["VBS-401", "VBS-402", CREDENTIAL_RULE],
    fallback_rule: "VBS-400",
    needs_admin: true,
};

const USER_DATA: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData";

impl Module for InstallerPackages {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["msi"]
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        if let Some(cache) = self.cache.get()
            && file.path().parent().is_some_and(|parent| same_path(parent, cache))
        {
            return; // an installed product's cached package – examined by the system part
        }
        let package = match file.open() {
            Ok(reader) => msi::analyze(reader),
            Err(error) => {
                if let Some(reason) = error.reason() {
                    report.not_checkable("VBS-400", file.location(), reason).file(file.facts()).emit();
                }
                return;
            }
        };
        match package {
            Ok(package) => report_package(report, &package, file.location(), Some(file.facts()), &[]),
            Err(_) if servicing_data(file) => {} // a differential of the component store, not a package
            Err(error) => report
                .not_checkable("VBS-400", file.location(), NotCheckableReason::Corrupt)
                .file(file.facts())
                .detail("readError", read_error(&error.0))
                .emit(),
        }
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        if let Some(root) = &system.env.system_root {
            let _ = self.cache.set(root.join("Installer"));
        }
        let sids = match system.registry.subkeys(Hive::LocalMachine, USER_DATA, Bitness::Native) {
            Ok(sids) => sids,
            Err(ViewError::NotFound) => return,
            Err(error) => {
                report.set_source_status(vbs_core::model::SourceStatus::Failed, error.reason_code());
                return;
            }
        };
        report.add_root(format!("HKLM\\{USER_DATA}"));
        for sid in sids {
            let products_key = format!(r"{USER_DATA}\{sid}\Products");
            let Ok(products) = system.registry.subkeys(Hive::LocalMachine, &products_key, Bitness::Native) else {
                continue;
            };
            for packed in products {
                report.count_entries(1);
                let properties_key = format!(r"{products_key}\{packed}\InstallProperties");
                let value = |name: &str| {
                    system
                        .registry
                        .value(Hive::LocalMachine, &properties_key, name, Bitness::Native)
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_text().map(str::to_owned))
                        .filter(|v| !v.trim().is_empty())
                };
                let Some(local_package) = value("LocalPackage") else { continue };
                let product_code = unpack_guid(&packed).unwrap_or_else(|| packed.clone());
                let location = Location { kind: LocationKind::MsiPackage, path: product_code, item: None };
                let mut details = vec![("packagePath", Detail::Text(local_package.clone()))];
                for (key, name) in
                    [("productName", "DisplayName"), ("productVersion", "DisplayVersion"), ("publisher", "Publisher")]
                {
                    if let Some(text) = value(name) {
                        details.push((key, Detail::Text(text)));
                    }
                }
                let path = PathBuf::from(system.env.expand(&local_package));
                let analyzed = system.files.open(&path).map(|reader| analyze_reader(reader));
                match analyzed {
                    Ok(Ok(package)) => {
                        report.count_inspected(1);
                        report_package(report, &package, location, None, &details);
                    }
                    Ok(Err(error)) => {
                        report.count_inspected(1);
                        let mut builder = report
                            .not_checkable("VBS-400", location, NotCheckableReason::Corrupt)
                            .detail("readError", read_error(&error.0));
                        for (key, value) in &details {
                            builder = builder.detail(key, value.clone());
                        }
                        builder.emit();
                    }
                    Err(ViewError::NotFound) => {
                        report.record_error(format!("{} (cached package missing)", path.display()))
                    }
                    Err(error) => {
                        if matches!(error, ViewError::AccessDenied) {
                            report.record_error(format!("{} ({error})", path.display()));
                        } else {
                            let mut builder = report.not_checkable("VBS-400", location, view_reason(&error));
                            if let ViewError::Failed(message) = &error {
                                builder = builder.detail("readError", read_error(message));
                            }
                            for (key, value) in &details {
                                builder = builder.detail(key, value.clone());
                            }
                            builder.emit();
                        }
                    }
                }
            }
        }
    }
}

fn analyze_reader(mut reader: Box<dyn vbs_core::module::ReadSeek + '_>) -> Result<Package, msi::MsiError> {
    msi::analyze(ReadSeekRef(&mut reader))
}

/// `&mut dyn ReadSeek` as a sized `Read + Seek`.
struct ReadSeekRef<'a, 'b>(&'a mut Box<dyn vbs_core::module::ReadSeek + 'b>);

impl Read for ReadSeekRef<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for ReadSeekRef<'_, '_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

fn report_package(
    report: &mut Report,
    package: &Package,
    location: Location,
    file: Option<FileFacts>,
    details: &[(&str, Detail)],
) {
    for action in &package.vbscript_actions {
        let breaks = action.scheduled && !action.continue_on_error;
        let rule = if breaks { "VBS-401" } else { "VBS-402" };
        let item_location = Location { item: Some(action.name.clone()), ..location.clone() };
        let mut builder = report
            .finding(rule, item_location.clone())
            .activation(Activation::Installer)
            .detail("customActionType", i64::from(action.action_type))
            .detail("scriptSource", action.source.as_str())
            .detail("scheduled", action.scheduled)
            .detail("continueOnError", action.continue_on_error);
        if let Some(facts) = file.clone() {
            builder = builder.file(facts);
        }
        for (key, value) in details {
            builder = builder.detail(key, value.clone());
        }
        if let (Some(name), false) = (&package.product_name, details.iter().any(|(k, _)| *k == "productName")) {
            builder = builder.detail("productName", name.clone());
        }
        if let Some(condition) = &action.condition {
            builder = builder.detail("condition", condition.clone());
        }
        if let Some(target) = target(action) {
            builder = builder.target(target);
        }
        let code =
            action.script.as_deref().map(|text| script::code_lines(text, Language::VbScript)).unwrap_or_default();
        for (number, line) in script::vbscript_evidence(&code) {
            builder = builder.evidence(Some(number), line);
        }
        if code.is_empty()
            && let Some(function) = &action.target
        {
            builder = builder.evidence(None, &format!("function {function}"));
        }
        builder.emit();
        report.credentials(
            item_location,
            file.clone(),
            Activation::Installer,
            code.iter().map(|(n, l)| (Some(*n), *l)),
        );
    }
}

fn target(action: &ScriptAction) -> Option<String> {
    let source = action.source_ref.as_deref()?;
    Some(match action.source {
        SourceKind::Binary => format!("Binary.{source}"),
        SourceKind::InstalledFile => format!("File.{source}"),
        SourceKind::Property => format!("Property.{source}"),
        SourceKind::Inline => return None,
    })
}

/// Whether a file that is no valid package is Windows servicing data ([`servicing`]).
fn servicing_data(file: &dyn CandidateFile) -> bool {
    let mut head = [0u8; 16];
    file.open().is_ok_and(|mut reader| reader.read_exact(&mut head).is_ok()) && servicing::is_servicing_data(&head)
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['\\', '/']))
}

/// Product codes are stored "packed" in the registry: the first three groups reversed,
/// the remaining bytes with swapped nibbles.
fn unpack_guid(packed: &str) -> Option<String> {
    if packed.len() != 32 || !packed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let reverse = |range: std::ops::Range<usize>| packed[range].chars().rev().collect::<String>();
    let swapped: Vec<String> = (16..32).step_by(2).map(|i| packed[i..i + 2].chars().rev().collect()).collect();
    Some(format!(
        "{{{}-{}-{}-{}-{}}}",
        reverse(0..8),
        reverse(8..12),
        reverse(12..16),
        swapped[..2].concat(),
        swapped[2..].concat()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::msi::tests::sample;
    use crate::modules::testing::{MemoryFile, inspect, scan_system};
    use vbs_core::model::FindingStatus;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, RegValue, SystemEnvironment};

    #[test]
    fn unpacks_product_codes() {
        assert_eq!(
            unpack_guid("B7F2E1B0A4F3E2D1C1B2A39485766758").as_deref(),
            Some("{0B1E2F7B-3F4A-1D2E-1C2B-3A4958677685}")
        );
        assert_eq!(unpack_guid("not packed"), None);
    }

    #[test]
    fn packages_on_disk() {
        let findings = inspect(&InstallerPackages::default(), &MemoryFile::new(r"D:\Software\inventory.msi", sample()));
        let summary: Vec<(&str, Option<&str>)> =
            findings.iter().map(|f| (f.rule.as_str(), f.location.item.as_deref())).collect();
        assert_eq!(
            summary,
            [
                ("VBS-401", Some("CheckLicense")),
                (CREDENTIAL_RULE, Some("CheckLicense")),
                ("VBS-401", Some("SetShortcut")),
                ("VBS-402", Some("Cleanup")),
                (CREDENTIAL_RULE, Some("Cleanup")),
                ("VBS-402", Some("Unused")),
            ]
        );
        assert!(findings.iter().all(|f| f.activation == Activation::Installer));
        assert_eq!(findings[0].details["productName"], Detail::Text("Contoso Inventory".into()));
        let broken = inspect(&InstallerPackages::default(), &MemoryFile::new("C:/x/broken.msi", b"garbage".to_vec()));
        assert_eq!((broken[0].rule.as_str(), &broken[0].status), ("VBS-400", &FindingStatus::NotCheckable));
        assert!(
            matches!(&broken[0].details["readError"], Detail::Text(text) if text.starts_with("not a valid package"))
        );
    }

    #[test]
    fn installed_products_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let installer = dir.path().join("Installer");
        std::fs::create_dir_all(&installer).unwrap();
        std::fs::write(installer.join("1a2b3c.msi"), sample()).unwrap();
        let key = format!(r"{USER_DATA}\S-1-5-18\Products\B7F2E1B0A4F3E2D1C1B2A39485766758\InstallProperties");
        let registry = MemoryRegistry::new().with_key(
            Hive::LocalMachine,
            &key,
            &[
                ("LocalPackage", RegValue::Text(installer.join("1a2b3c.msi").display().to_string())),
                ("DisplayName", RegValue::Text("Contoso Inventory".into())),
            ],
        );
        let env = SystemEnvironment { system_root: Some(dir.path().to_path_buf()), ..SystemEnvironment::default() };
        let module = InstallerPackages::default();
        let report = scan_system(&module, &env, &registry, &MemoryEventLogs::new(), &MemoryWmi::new());
        let finding = &report.findings()[0];
        assert_eq!(finding.location.path, "{0B1E2F7B-3F4A-1D2E-1C2B-3A4958677685}");
        assert_eq!(finding.location.kind, LocationKind::MsiPackage);
        // The file part leaves the cache to the system part.
        let cached = MemoryFile::new(&installer.join("1a2b3c.msi").display().to_string(), sample());
        assert!(inspect(&module, &cached).is_empty());
    }
}
