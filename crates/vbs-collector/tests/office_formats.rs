//! The Office and Access readers against files made by Office itself (third-party test files
//! from Apache POI, oletools and Jackcess in `tests/corpus/*/office/real/`, origin and licence in
//! `tests/corpus/THIRD-PARTY.md`) and against the generated collection: every project and
//! module is found, the source reads as other readers read it (the expected texts are those
//! Apache POI's own tests check), and protected files are recognised.

mod common;

use std::fs::File;
use std::path::PathBuf;

use vbs_collector::analysis::office::{self, Container, Document, Obstacle};
use vbs_collector::analysis::ovba::Project;

fn corpus(path: &str) -> PathBuf {
    common::corpus().join(path)
}

fn analyze(path: &str) -> Document {
    let file = corpus(path);
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    office::analyze(File::open(&file).unwrap(), &name)
}

fn project(document: &Document, index: usize) -> &Project {
    document.projects[index].project.as_ref().unwrap_or_else(|e| panic!("project {index}: {e}"))
}

fn source<'a>(project: &'a Project, module: &str) -> &'a str {
    let found = project.modules.iter().find(|m| m.name == module).unwrap_or_else(|| panic!("no module {module}"));
    found.source.as_deref().unwrap_or_else(|e| panic!("module {module}: {e}"))
}

fn module_names(project: &Project) -> Vec<&str> {
    project.modules.iter().map(|m| m.name.as_str()).collect()
}

#[test]
fn excel_and_word_files_made_by_office() {
    let xls = analyze("negative/office/real/poi-SimpleMacro.xls");
    assert_eq!(xls.container, Container::Compound);
    assert_eq!(xls.projects[0].storage, "_VBA_PROJECT_CUR");
    let workbook = project(&xls, 0);
    assert_eq!(module_names(workbook), ["Module1", "ThisWorkbook", "Sheet1", "Sheet2", "Sheet3"]);
    assert!(source(workbook, "Module1").contains("ActiveCell.FormulaR1C1 = \"This is a macro workbook\""));
    assert!(!workbook.protection.locked);

    let xlsm = analyze("negative/office/real/poi-SimpleMacro.xlsm");
    assert_eq!((xlsm.container, xlsm.projects[0].storage.as_str()), (Container::Package, "xl/vbaProject.bin"));
    assert_eq!(source(project(&xlsm, 0), "Module1"), source(workbook, "Module1"));

    let doc = analyze("negative/office/real/poi-SimpleMacro.doc");
    assert_eq!(doc.projects[0].storage, "Macros");
    assert_eq!(module_names(project(&doc, 0)), ["ThisDocument", "Module1"]);
    let docm = analyze("negative/office/real/poi-SimpleMacro.docm");
    assert!(source(project(&docm, 0), "Module1").contains("Sub TestMacro()"));
    let pptm = analyze("negative/office/real/poi-SimpleMacro.pptm");
    assert_eq!(pptm.projects[0].storage, "ppt/vbaProject.bin");
    assert!(source(project(&pptm, 0), "Module1").contains("Sub TestMacro()"));
}

#[test]
fn unusual_files_from_apache_pois_bug_reports() {
    // 29 modules (govdocs1 609751.xls).
    let many = analyze("negative/office/real/poi-59830-modules.xls");
    let workbook = project(&many, 0);
    assert_eq!(workbook.modules.len(), 29);
    assert!(source(workbook, "Module20").contains("here start of superscripting"));
    // Saved on a Mac: code page 10000 (govdocs1 147240.xls).
    let mac = analyze("negative/office/real/poi-60273-mac.xls");
    assert_eq!(project(&mac, 0).code_page, 10000);
    assert!(source(project(&mac, 0), "Module1").contains("9/8/2004"));
    // The module offset points to zeros; the source sits elsewhere in the stream.
    let offset = analyze("negative/office/real/poi-60279-offset.doc");
    assert!(source(project(&offset, 0), "ThisDocument").contains("Attribute VB_Customizable = True"));
    let dirty = analyze("negative/office/real/poi-60158.docm");
    assert!(source(project(&dirty, 0), "NewMacros").contains("' dirty"));
}

#[test]
fn password_protection_and_encryption() {
    for name in ["encrypted.docm", "encrypted.xlsm", "encrypted.pptm", "encrypted.xlsb"] {
        let document = analyze(&format!("positive/office/real/oletools-{name}"));
        assert!(document.projects.is_empty(), "{name}");
        assert_eq!(document.unreadable[0].obstacle, Obstacle::PasswordProtected, "{name}");
        assert_eq!(document.encryption, Some("agile"), "{name}");
    }
    // Binary workbooks and documents encrypted with a password keep their (here: no) VBA storage.
    for name in ["oletools-encrypted.xls", "oletools-encrypted.doc", "poi-password.xls", "poi-xor-encryption-abc.xls"] {
        let document = analyze(&format!("negative/office/real/{name}"));
        assert!(document.content_encrypted, "{name}");
        assert!(document.projects.is_empty() && document.unreadable.is_empty(), "{name}");
    }
    let protected = analyze("positive/office/protected/password-to-open.xlsm");
    assert_eq!(protected.unreadable[0].obstacle, Obstacle::PasswordProtected);
    let locked = analyze("positive/office/locked-project.xls");
    assert!(project(&locked, 0).protection.locked);
}

#[test]
fn access_databases_of_every_version() {
    // Access 97 without code (Jackcess test data): MSysModules2 holds only system rows.
    let access97 = analyze("negative/office/real/jackcess-testV1997.mdb");
    assert_eq!(access97.container, Container::Access);
    assert!(access97.projects.is_empty() && access97.unreadable.is_empty());
    // Empty databases as Access made them (shipped with Jackcess): the project has no modules.
    for (path, storage) in [
        ("negative/office/access/empty.accdb", "MSysAccessStorage/VBA/VBAProject"),
        ("negative/office/access/empty.mdb", "MSysAccessStorage/VBA/VBAProject"),
        ("negative/office/access/empty-2000.mdb", "MSysAccessObjects/VBA/VBAProject"),
    ] {
        let document = analyze(path);
        assert_eq!(document.projects[0].storage, storage, "{path}");
        assert!(project(&document, 0).modules.is_empty(), "{path}");
    }
    // Projects written by Jackcess (2003, 2010), in the chunked compound file (2000), and in a
    // database encoded with RC4 per page.
    for (path, module, text) in [
        ("positive/office/access/inventory.accdb", "Validation", "VBScript_RegExp_55.RegExp"),
        ("positive/office/access/orders.mdb", "Form_Orders", "export-orders.vbs"),
        ("positive/office/access/legacy-2000.mdb", "Formulas", "sc.Language = \"VBS\""),
        ("positive/office/access/encoded.mdb", "Checks", "VBScript.RegExp"),
    ] {
        let document = analyze(path);
        assert!(source(project(&document, 0), module).contains(text), "{path}");
    }
}

/// Prints what the reader finds in every file of `VBS_OFFICE_SAMPLES` (a folder) – for comparing
/// with other readers:
/// `VBS_OFFICE_SAMPLES=<folder> cargo test -p vbs-collector --test office_formats -- --ignored --nocapture`
#[test]
#[ignore = "needs a folder of sample documents"]
fn print_samples() {
    let Ok(folder) = std::env::var("VBS_OFFICE_SAMPLES") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths.into_iter().filter(|p| p.is_file()) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let document = office::analyze(File::open(&path).unwrap(), &name);
        println!(
            "== {name}: {} encryption={:?} contentEncrypted={}",
            document.container.as_str(),
            document.encryption,
            document.content_encrypted
        );
        for unreadable in &document.unreadable {
            println!("   UNREADABLE {:?} {:?}: {}", unreadable.location, unreadable.obstacle, unreadable.message);
        }
        for found in &document.projects {
            match &found.project {
                Err(error) => println!("   PROJECT {} (embedded {:?}) ERROR {error}", found.storage, found.embedded),
                Ok(project) => {
                    println!(
                        "   PROJECT {} (embedded {:?}) name={:?} cp={} {:?} refs={}",
                        found.storage,
                        found.embedded,
                        project.name,
                        project.code_page,
                        project.protection,
                        project.references.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(",")
                    );
                    for module in &project.modules {
                        match &module.source {
                            Ok(source) => println!("      {} ({} bytes)", module.name, source.len()),
                            Err(error) => println!("      {} ERROR {error}", module.name),
                        }
                    }
                }
            }
        }
    }
}

/// Damaged files never make the readers panic (a panic would only cost this one file an
/// `internalError` finding, but it must not happen): every Office file of the collections with
/// flipped bytes, and cut short, deterministically. More rounds: `VBS_DAMAGE_ROUNDS=1000`.
#[test]
fn damaged_documents_do_not_panic() {
    let rounds: usize = std::env::var("VBS_DAMAGE_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
    let mut files = Vec::new();
    for collection in ["positive/office", "negative/office"] {
        collect(&corpus(collection), &mut files);
    }
    assert!(files.len() > 40, "office files found: {}", files.len());
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for path in files {
        let original = std::fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for round in 0..rounds {
            let mut bytes = original.clone();
            if bytes.is_empty() {
                continue;
            }
            if round % 4 == 3 {
                bytes.truncate((next() as usize) % bytes.len());
            } else {
                for flip in 0..(1 + next() % 16) {
                    // Half of the changes hit the first 64 KiB, where headers, directories and the
                    // system tables of the databases live.
                    let span = if flip % 2 == 0 { bytes.len().min(64 * 1024) } else { bytes.len() };
                    let at = (next() as usize) % span;
                    bytes[at] ^= (next() as u8) | 1;
                }
            }
            let result = std::panic::catch_unwind(|| {
                let document = office::analyze(std::io::Cursor::new(&bytes), &name);
                for found in &document.projects {
                    if let Ok(project) = &found.project {
                        let _ = vbs_collector::analysis::vba::analyze(project);
                    }
                }
            });
            assert!(result.is_ok(), "{} (round {round}) made the reader panic", path.display());
        }
    }
}

fn collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}
