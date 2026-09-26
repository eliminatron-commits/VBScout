//! Definition of Done #5 (file part): 100,000 files are scanned in under 10 minutes.
//!
//! Creates 100,000 files in 1,000 folders – ordinary files plus candidates of every file
//! module (scripts, shortcuts, batch/PowerShell, installer packages) – and runs a complete
//! file scan with all modules. Runs in CI in release mode:
//! `cargo test --release -p vbs-collector --test performance -- --ignored --nocapture`

mod common;

use std::fs;
use std::time::{Duration, Instant};

use vbs_collector::engine::{self, Plan};
use vbs_collector::modules;
use vbs_collector::platform::Host;
use vbs_collector::read_only::ReadOnlyFiles;
use vbs_core::model::{Machine, OperatingSystem};
use vbs_core::views::{SystemEnvironment, Unavailable};

const FILES: usize = 100_000;
const FOLDERS: usize = 1_000;
const LIMIT: Duration = Duration::from_secs(600);

fn host() -> Host {
    Host {
        machine: Machine {
            hostname: "PERF".into(),
            fqdn: None,
            domain: None,
            machine_id: None,
            os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
        },
        env: SystemEnvironment { elevated: true, ..SystemEnvironment::default() },
        local_roots: Vec::new(),
        registry: Box::new(Unavailable),
        files: Box::new(ReadOnlyFiles),
        event_logs: Box::new(Unavailable),
        wmi: Box::new(Unavailable),
        supported: true,
    }
}

#[test]
#[ignore = "creates 100,000 files; run explicitly (CI does)"]
fn scans_100_000_files_in_under_10_minutes() {
    let root = tempfile::tempdir().unwrap();
    let corpus = common::corpus().join("positive");
    let samples: Vec<(&str, Vec<u8>)> = [
        ("vbs", "scripts/backup.vbs"),
        ("lnk", "shortcuts/Inventory.lnk"),
        ("cmd", "invocations/nightly.cmd"),
        ("ps1", "invocations/deploy.ps1"),
        ("msi", "installer/legacy-inventory.msi"),
        ("hta", "scripts/admin-console.hta"),
    ]
    .into_iter()
    .map(|(extension, path)| (extension, fs::read(corpus.join(path)).unwrap()))
    .collect();

    let created = Instant::now();
    let filler = vec![b'x'; 2048];
    let mut planted = 0;
    for index in 0..FILES {
        let folder = root.path().join(format!("d{:03}", index % FOLDERS)).join(format!("s{}", index % 7));
        if index < FOLDERS * 7 {
            fs::create_dir_all(&folder).unwrap();
        }
        // Every 50th file is a candidate for a module (2 %), the rest are ordinary files.
        if index % 50 == 0 {
            let (extension, bytes) = &samples[(index / 50) % samples.len()];
            fs::write(folder.join(format!("f{index}.{extension}")), bytes).unwrap();
            planted += 1;
        } else {
            let extension = ["txt", "dll", "log", "xml", "js", "json", "png", "exe"][index % 8];
            fs::write(folder.join(format!("f{index}.{extension}")), &filler).unwrap();
        }
    }
    println!("created {FILES} files ({planted} candidates) in {:.1} s", created.elapsed().as_secs_f64());

    let plan = Plan {
        paths: vec![root.path().to_path_buf()],
        network_paths: Vec::new(),
        system_sources: false,
        threads: engine::default_threads(),
    };
    let clock = Instant::now();
    let result = engine::scan(&plan, &host(), &modules::all());
    let elapsed = clock.elapsed();
    let entries: u64 = result.coverage.sources.iter().map(|s| s.entries).sum();
    let inspected: u64 = result.coverage.sources.iter().map(|s| s.inspected).sum();
    println!(
        "scanned {entries} entries ({inspected} inspected, {} findings) in {:.1} s – {:.0} entries/s",
        result.findings.len(),
        elapsed.as_secs_f64(),
        entries as f64 / elapsed.as_secs_f64().max(0.001)
    );
    assert!(entries >= FILES as u64, "every file was listed");
    assert_eq!(inspected, planted as u64, "every candidate was inspected");
    assert!(result.findings.iter().all(|f| f.reason != Some(vbs_core::model::NotCheckableReason::InternalError)));
    assert!(elapsed < LIMIT, "100,000 files took {elapsed:?} (limit {LIMIT:?})");
}
