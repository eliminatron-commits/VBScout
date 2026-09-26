//! Definition of Done #1, dynamic and cross-platform: the collector changes nothing and writes only
//! its result file.
//!
//! A copy of the test corpus plus special entries (a read-only script, links, a FIFO named like a
//! script) is scanned by the real binary. Before and after, every entry's type, size, content hash,
//! times, attributes and link target are compared. The working folder and the temp folder of the
//! process stay empty, and the output folder holds exactly the result file. A second run with the
//! same output path must fail without touching the first result (nothing is ever overwritten).
//!
//! The kernel-level traces in `scripts/readonly/` (strace on Linux, ETW on Windows) complement this
//! by watching every file system and registry operation of the process, wherever it happens.

mod common;

use std::ffi::OsStr;
use std::fs;

#[test]
fn collector_changes_nothing_and_writes_only_its_result() {
    let base = tempfile::tempdir().unwrap();
    let scan = base.path().join("scan");
    common::copy_tree(&common::corpus(), &scan);

    let readonly = scan.join("readonly-script.vbs");
    fs::write(&readonly, "WScript.Echo \"read-only file\"\r\n").unwrap();
    let mut permissions = fs::metadata(&readonly).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&readonly, permissions).unwrap();
    #[cfg(unix)]
    {
        let outside = base.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("elsewhere.vbs"), "MsgBox 1").unwrap();
        std::os::unix::fs::symlink(&outside, scan.join("link-to-outside")).unwrap();
        std::os::unix::fs::symlink(outside.join("elsewhere.vbs"), scan.join("linked-script.vbs")).unwrap();
        // Opening a FIFO for reading would block forever: the walk must never open non-regular files.
        #[allow(clippy::disallowed_methods)] // test setup only
        let fifo = std::process::Command::new("mkfifo").arg(scan.join("trap.vbs")).status().unwrap();
        assert!(fifo.success());
    }

    let (out_dir, cwd, temp) = (base.path().join("out"), base.path().join("cwd"), base.path().join("temp"));
    for dir in [&out_dir, &cwd, &temp] {
        fs::create_dir_all(dir).unwrap();
    }
    let result_name = format!("result.{}", vbs_core::file_extension());
    let result = out_dir.join(&result_name);
    let args = [OsStr::new("--path"), scan.as_os_str(), OsStr::new("--out"), result.as_os_str(), OsStr::new("--quiet")];

    let before = common::snapshot(&scan);
    let output = common::run_collector(&args, &cwd, &temp);
    assert!(output.status.success(), "collector failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "--quiet prints nothing on success");

    assert_eq!(common::snapshot(&scan), before, "the scanned tree must not change in any way");
    assert_eq!(
        common::entries(&out_dir),
        std::slice::from_ref(&result_name),
        "exactly one file is written: the result"
    );
    assert!(common::entries(&cwd).is_empty(), "nothing is written to the working folder");
    assert!(common::entries(&temp).is_empty(), "nothing is written to the temp folder");

    let first = fs::read(&result).unwrap();
    let again = common::run_collector(&args, &cwd, &temp);
    assert_eq!(again.status.code(), Some(1), "an existing result file is never overwritten");
    assert_eq!(fs::read(&result).unwrap(), first);
    assert_eq!(common::entries(&out_dir), [result_name]);
    assert_eq!(common::snapshot(&scan), before);
}

#[test]
fn invalid_options_change_nothing() {
    let base = tempfile::tempdir().unwrap();
    let (cwd, temp) = (base.path().join("cwd"), base.path().join("temp"));
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&temp).unwrap();
    for args in [
        vec!["--path", r"\\server\share"],
        vec!["--include-unc", "local-folder"],
        vec!["--path", "does-not-exist"],
        vec!["--unknown-option"],
    ] {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = common::run_collector(&args, &cwd, &temp);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?} explains the problem");
    }
    let missing_dir = base.path().join("missing").join("r.vbscout");
    let output = common::run_collector(&[OsStr::new("--out"), missing_dir.as_os_str()], &cwd, &temp);
    assert_eq!(output.status.code(), Some(1));
    assert!(common::entries(&cwd).is_empty() && common::entries(&temp).is_empty());
    assert!(!base.path().join("missing").exists());
}
