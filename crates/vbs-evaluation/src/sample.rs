//! Synthetic result files: machines of a small organization with the typical mix of VBScript
//! dependencies – logon scripts on a share, scheduled tasks and services on servers, macros,
//! Windows' own scripts and items that could not be checked. Used by the tests, the merge
//! performance test, the report examples and the app's self-check. Nothing here reads the system.

use std::collections::BTreeMap;

use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use vbs_core::model::{
    Activation, Coverage, CoverageMode, Detail, Evidence, FileFacts, Finding, FindingStatus, Generator, Limitation,
    LimitationCode, Location, LocationKind, Machine, NotCheckableReason, OperatingSystem, ProductType, SCHEMA_VERSION,
    ScanResult, Scope, SourceCoverage, SourceStatus, TimeRange,
};

/// Every tenth machine is a server.
pub fn is_server(index: usize) -> bool {
    index.is_multiple_of(10)
}

/// Host name of machine `index`.
pub fn hostname(index: usize) -> String {
    if is_server(index) { format!("SRV-{:03}", index / 10 + 1) } else { format!("PC-{:04}", index + 1) }
}

struct Builder {
    findings: Vec<Finding>,
}

impl Builder {
    fn add(
        &mut self,
        rule: &str,
        activation: Activation,
        location: (LocationKind, &str, Option<&str>),
        target: Option<&str>,
        file: Option<(u64, &str)>,
        evidence: &[(Option<u32>, &str)],
    ) -> &mut Finding {
        let catalog_rule = vbs_core::rules::catalog().rule(rule);
        let (kind, location_kind, path, item) = (
            catalog_rule.map(|rule| rule.kind.clone()).unwrap_or(vbs_core::model::FindingKind::ScriptFile),
            location.0,
            location.1,
            location.2,
        );
        let not_checkable = rule.ends_with("00");
        self.findings.push(Finding {
            id: format!("f{}", self.findings.len() + 1),
            rule: rule.into(),
            kind,
            classification: catalog_rule
                .map(|rule| rule.classification.clone())
                .unwrap_or(vbs_core::model::Classification::Review),
            status: if not_checkable { FindingStatus::NotCheckable } else { FindingStatus::Detected },
            reason: not_checkable.then_some(NotCheckableReason::AccessDenied),
            activation,
            location: Location { kind: location_kind, path: path.into(), item: item.map(str::to_owned) },
            target: target.map(str::to_owned),
            file: file.map(|(size, sha256)| FileFacts {
                size,
                modified_at: Some(OffsetDateTime::UNIX_EPOCH + Duration::days(18_000)),
                sha256: (!sha256.is_empty()).then(|| format!("{sha256:0>64}")),
                network: path.starts_with(r"\\"),
            }),
            evidence: evidence
                .iter()
                .map(|(line, text)| Evidence {
                    line: *line,
                    text: (*text).to_owned(),
                    masked: text.contains("********"),
                })
                .collect(),
            details: BTreeMap::new(),
        });
        self.findings.last_mut().expect("just added")
    }

    fn script(&mut self, path: &str, size: u64, sha256: &str, activation: Activation, lines: &[(u32, &str)]) {
        let evidence: Vec<(Option<u32>, &str)> = lines.iter().map(|(line, text)| (Some(*line), *text)).collect();
        self.add("VBS-101", activation, (LocationKind::File, path, None), None, Some((size, sha256)), &evidence);
    }
}

fn source(id: &str, status: SourceStatus, entries: u64, range: Option<TimeRange>) -> SourceCoverage {
    SourceCoverage {
        source: id.into(),
        status,
        reason: None,
        roots: Vec::new(),
        entries,
        inspected: entries / 50,
        errors: 0,
        skipped: 0,
        error_samples: Vec::new(),
        duration_ms: 1_000,
        time_range: range,
    }
}

/// The result file of machine `index`, scanned at `started`.
pub fn machine(index: usize, started: OffsetDateTime) -> ScanResult {
    let server = is_server(index);
    let mut b = Builder { findings: Vec::new() };
    use Activation::*;

    // Windows' own scripts: in System32 and, with the same content, in the component store.
    let store = r"C:\Windows\WinSxS\amd64_microsoft-windows-security-spp-tools_31bf3856ad364e35_10.0.26100.1_none_7f1e2c3b4a5d6e7f";
    b.script(
        r"C:\Windows\System32\slmgr.vbs",
        146_000,
        "a11",
        Dormant,
        &[(1, "' Windows Software Licensing Management Tool")],
    );
    b.script(
        &format!(r"{store}\slmgr.vbs"),
        146_000,
        "a11",
        Dormant,
        &[(1, "' Windows Software Licensing Management Tool")],
    );
    b.script(r"C:\Windows\System32\winrm.vbs", 210_000, "a12", Dormant, &[(3, "Option Explicit")]);
    b.script(
        r"C:\Windows\WinSxS\amd64_microsoft-windows-w..for-management-core_31bf3856ad364e35_10.0.26100.1_none_1a2b3c4d5e6f7081\winrm.vbs",
        210_000,
        "a12",
        Dormant,
        &[(3, "Option Explicit")],
    );

    // The logon script on the domain share, applied by Group Policy on every machine.
    let logon = r"\\corp.example\NETLOGON\logon.vbs";
    b.script(
        logon,
        6_500,
        "b21",
        Dormant,
        &[
            (12, r#"objNetwork.MapNetworkDrive "H:", "\\fs01\home\" & strUser"#),
            (27, r#"objNetwork.AddWindowsPrinterConnection "\\print01\Floor2""#),
        ],
    );
    b.add(
        "VBS-341",
        Automatic,
        (
            LocationKind::Registry,
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Logon\0\0",
            Some("Script"),
        ),
        Some(logon),
        None,
        &[(None, logon)],
    );

    if server {
        let backup = r"C:\Scripts\backup.vbs";
        b.script(
            backup,
            12_400,
            "c31",
            Dormant,
            &[
                (8, r#"Set objShell = CreateObject("WScript.Shell")"#),
                (41, r#"objShell.Run "robocopy D:\Data \\nas01\backup /MIR", 0, True"#),
            ],
        );
        b.add(
            "VBS-901",
            Dormant,
            (LocationKind::File, backup, None),
            None,
            Some((12_400, "c31")),
            &[(Some(15), r#"strPassword = "********""#)],
        )
        .details
        .insert("secrets".into(), Detail::Number(1));
        b.add(
            "VBS-301",
            Automatic,
            (LocationKind::ScheduledTask, r"\Backup Nightly", Some("Exec 1")),
            Some(backup),
            None,
            &[(None, r#"cscript.exe //nologo "C:\Scripts\backup.vbs""#)],
        );
        let monitor = r"C:\Services\monitor.vbs";
        b.script(monitor, 41_000, "c32", Dormant, &[(102, "Do While True")]);
        b.add(
            "VBS-321",
            Automatic,
            (LocationKind::Service, "DiskMonitor", Some("Parameters\\Application")),
            Some(monitor),
            None,
            &[(None, r#"C:\Windows\System32\cscript.exe C:\Services\monitor.vbs"#)],
        );
        b.add(
            "VBS-331",
            Automatic,
            (LocationKind::Wmi, r"root\subscription:ActiveScriptEventConsumer.Name='CleanupTemp'", None),
            None,
            None,
            &[(None, "ScriptingEngine = VBScript")],
        );
        b.add(
            "VBS-401",
            Installer,
            (LocationKind::MsiPackage, "{6F2C1A7E-3B4D-4E5F-8A9B-0C1D2E3F4A5B}", Some("SetInstallDir")),
            None,
            None,
            &[(None, "Custom action type 38: VBScript in the Target column")],
        )
        .details
        .insert("productName".into(), Detail::Text("Contoso Monitoring Agent".into()));
        let report = r"C:\Scripts\report.vbs";
        b.script(report, 3_200, "c33", Dormant, &[(5, r#"Set objExcel = CreateObject("Excel.Application")"#)]);
        b.add(
            "VBS-502",
            Logged,
            (LocationKind::EventLog, "Microsoft-Windows-Sysmon/Operational", Some("Sysmon 1")),
            Some(report),
            None,
            &[(None, r#"cscript.exe C:\Scripts\report.vbs /weekly"#)],
        );
        let mut ual = b
            .add(
                "VBS-600",
                Dormant,
                (LocationKind::File, r"C:\Windows\System32\LogFiles\Sum\Current.mdb", None),
                None,
                Some((4_194_304, "")),
                &[],
            )
            .clone();
        ual.reason = Some(NotCheckableReason::Locked);
        let last = b.findings.len() - 1;
        b.findings[last] = ual;
    } else {
        let agent = r"C:\ProgramData\Contoso\agent.vbs";
        b.script(agent, 2_100, "d41", Dormant, &[(3, r#"Set objWMI = GetObject("winmgmts:\\.\root\cimv2")"#)]);
        b.add(
            "VBS-311",
            Automatic,
            (LocationKind::Registry, r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run", Some("ContosoAgent")),
            Some(agent),
            None,
            &[(None, r#"wscript.exe "C:\ProgramData\Contoso\agent.vbs""#)],
        );
        b.add(
            "VBS-211",
            Manual,
            (LocationKind::File, r"C:\Users\Public\Desktop\Inventory.lnk", None),
            Some(r"\\corp.example\Tools\inventory.vbs"),
            Some((1_450, "e01")),
            &[(None, r#"wscript.exe \\corp.example\Tools\inventory.vbs"#)],
        );
        if index % 3 == 1 {
            b.add(
                "VBS-601",
                Macro,
                (LocationKind::File, r"C:\Users\Public\Documents\Monthly report.xlsm", Some("Module1")),
                None,
                Some((88_000, "f51")),
                &[(Some(14), r#"Set re = CreateObject("VBScript.RegExp")"#)],
            );
        }
        if index % 5 == 1 {
            b.add(
                "VBS-105",
                Manual,
                (LocationKind::File, r"C:\Tools\Helpdesk\helpdesk.hta", None),
                None,
                Some((22_000, "f52")),
                &[(Some(48), r#"<script language="VBScript">"#)],
            );
        }
        if index % 7 == 3 {
            let mut placeholder = b
                .add(
                    "VBS-100",
                    Dormant,
                    (LocationKind::File, &format!(r"C:\Users\user{index}\OneDrive - Corp\Tools\export.vbs"), None),
                    None,
                    Some((4_096, "")),
                    &[],
                )
                .clone();
            placeholder.reason = Some(NotCheckableReason::CloudPlaceholder);
            let last = b.findings.len() - 1;
            b.findings[last] = placeholder;
        }
        if index % 11 == 5 {
            let mut protected = b
                .add(
                    "VBS-600",
                    Macro,
                    (LocationKind::File, r"C:\Users\Public\Documents\Budget.xlsm", Some("_VBA_PROJECT_CUR")),
                    None,
                    Some((120_000, "")),
                    &[],
                )
                .clone();
            protected.reason = Some(NotCheckableReason::PasswordProtected);
            let last = b.findings.len() - 1;
            b.findings[last] = protected;
        }
    }

    // Coverage: the Application log reaches back 2–6 weeks, Sysmon on some machines.
    let limited = index % 13 == 12;
    let log_days = 14 + (index % 5) as i64 * 7;
    let deprecation = TimeRange { from: started - Duration::days(log_days), to: started };
    let mut sources = vec![
        source("files.localDrives", SourceStatus::Partial, 400_000 + index as u64 * 17, None),
        source("tasks.scheduled", SourceStatus::Complete, 160, None),
        source("autostart.entries", SourceStatus::Complete, 24, None),
        source("services.configuration", SourceStatus::Complete, 290, None),
        source("wmi.subscriptions", SourceStatus::Complete, 2, None),
        source("policies.scripts", SourceStatus::Complete, 1, None),
        source("installer.packages", SourceStatus::Complete, 80, None),
        source("eventLog.vbscriptDeprecation", SourceStatus::Complete, 120, Some(deprecation)),
    ];
    sources[0].reason = Some("unreadableItems".into());
    sources[0].errors = 3;
    sources[0].skipped = 12;
    sources[0].error_samples = vec![r"C:\System Volume Information (Access is denied. (os error 5))".into()];
    if index.is_multiple_of(7) {
        sources.push(source(
            "eventLog.sysmon",
            SourceStatus::Complete,
            5_000,
            Some(TimeRange { from: started - Duration::days(3), to: started }),
        ));
    } else {
        let mut sysmon = source("eventLog.sysmon", SourceStatus::Unavailable, 0, None);
        sysmon.reason = Some("notInstalled".into());
        sources.push(sysmon);
    }
    let mut limitations = Vec::new();
    if limited {
        limitations.push(Limitation { code: LimitationCode::NotElevated, detail: None });
        sources[1].status = SourceStatus::Failed;
        sources[1].reason = Some("accessDenied".into());
    }

    ScanResult {
        format: vbs_core::media_type().into(),
        schema_version: SCHEMA_VERSION,
        scan_id: Uuid::from_u128(0x5ca9_0000_0000_4000_8000_0000_0000_0000 | index as u128),
        generator: Generator {
            product: vbs_config::product().name.clone(),
            component: "collector".into(),
            version: "0.1.0".into(),
            rules_as_of: vbs_core::rules::catalog().as_of_text(),
            platform: "windows".into(),
        },
        started_at: started,
        finished_at: started + Duration::seconds(300 + (index % 60) as i64 * 7),
        machine: Machine {
            hostname: hostname(index),
            fqdn: Some(format!("{}.corp.example", hostname(index).to_lowercase())),
            domain: Some("corp.example".into()),
            machine_id: Some(format!("{index:064x}")),
            os: if server {
                OperatingSystem {
                    family: "windows".into(),
                    name: Some("Windows Server 2022 Standard".into()),
                    version: Some("10.0.20348.4171".into()),
                    build: Some(20348),
                    display_version: Some("21H2".into()),
                    edition: Some("ServerStandard".into()),
                    product_type: Some(ProductType::Server),
                    architecture: Some("x64".into()),
                }
            } else {
                OperatingSystem {
                    family: "windows".into(),
                    name: Some("Windows 11 Pro".into()),
                    version: Some("10.0.26100.4652".into()),
                    build: Some(26100),
                    display_version: Some("24H2".into()),
                    edition: Some("Professional".into()),
                    product_type: Some(ProductType::Workstation),
                    architecture: Some("x64".into()),
                }
            },
        },
        scope: Scope {
            local_drives: true,
            paths: Vec::new(),
            network_paths: vec![r"\\corp.example\NETLOGON".into()],
            system_sources: true,
        },
        coverage: Coverage {
            mode: if limited { CoverageMode::Limited } else { CoverageMode::Full },
            elevated: !limited,
            limitations,
            sources,
        },
        findings: b.findings,
    }
}

/// `count` machines scanned over one day.
pub fn organization(count: usize, first_scan: OffsetDateTime) -> Vec<ScanResult> {
    (0..count).map(|index| machine(index, first_scan + Duration::seconds(index as i64 * 60))).collect()
}

/// Machine `index` with `extra` further script files, as a machine with a long history has them:
/// Windows' own scripts in the component store (the same on every machine), scripts deployed to
/// many machines, and scripts of its own. Used by the merge performance test.
pub fn busy_machine(index: usize, started: OffsetDateTime, extra: usize) -> ScanResult {
    let mut result = machine(index, started);
    let mut b = Builder { findings: std::mem::take(&mut result.findings) };
    for number in 0..extra {
        let (path, hash) = match number % 4 {
            0 => (
                format!(
                    r"C:\Windows\WinSxS\amd64_microsoft-windows-component{:03}_31bf3856ad364e35_10.0.26100.1_none_{:016x}\tool{:03}.vbs",
                    number % 60,
                    number % 60,
                    number % 60
                ),
                format!("a{:03}", number % 60),
            ),
            1 => (format!(r"C:\ProgramData\Corp\Deploy\job{:03}.vbs", number % 80), format!("b{:03}", number % 80)),
            _ => (
                format!(r"C:\Users\user{index}\Documents\Scripts\script{number:04}.vbs"),
                format!("c{index:05}{number:05}"),
            ),
        };
        b.script(
            &path,
            1_000 + number as u64 * 37 % 60_000,
            &hash,
            Activation::Dormant,
            &[(4, r#"Set objFSO = CreateObject("Scripting.FileSystemObject")"#), (19, r#"WScript.Echo "done""#)],
        );
    }
    for (index, finding) in b.findings.iter_mut().enumerate() {
        finding.id = format!("f{}", index + 1);
    }
    result.findings = b.findings;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assessment::{Assessment, Origin};
    use crate::import::ImportedFile;
    use time::macros::datetime;

    #[test]
    fn samples_are_valid_result_files() {
        for result in organization(30, datetime!(2026-09-25 08:00 UTC)) {
            let bytes = vbs_core::to_bytes(&result).expect("valid result");
            let loaded = vbs_core::read(std::io::Cursor::new(bytes)).expect("readable");
            assert_eq!(loaded.unknown_values, 0);
        }
    }

    #[test]
    fn samples_merge_as_expected() {
        let files: Vec<ImportedFile> = organization(30, datetime!(2026-09-25 08:00 UTC))
            .into_iter()
            .enumerate()
            .map(|(index, result)| ImportedFile { path: format!("m{index}.vbscout").into(), result, unknown_values: 0 })
            .collect();
        let assessment = Assessment::build(&files, None);
        let logon = assessment.items.iter().find(|item| item.first().location.path.ends_with("logon.vbs")).unwrap();
        assert_eq!(logon.machines, 30, "one network script for all machines");
        assert_eq!(logon.activation, Activation::Automatic, "started by the logon script policy");
        let windows: Vec<_> = assessment.items.iter().filter(|item| item.origin == Origin::Windows).collect();
        assert!(windows.iter().any(|item| item.first().location.path.ends_with("slmgr.vbs") && item.machines == 30));
        assert!(assessment.summary.high > 0 && assessment.summary.not_checkable > 0);
    }
}
