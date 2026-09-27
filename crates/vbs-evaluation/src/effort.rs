//! Rule-of-thumb effort per item and in total.
//!
//! Every rule of the catalog (`rules/catalog.json`) carries a range in hours and what it refers
//! to ([`EffortBasis`]). The estimate is deliberately simple and always presented as a rule of
//! thumb ("Faustwert"), never as a quote:
//!
//! * `fixed` – the range as given (checking or changing one item).
//! * `scriptSize` – code to rewrite: the range times a size factor of the script file
//!   (≤ 8 KiB × 1, ≤ 32 KiB × 2, ≤ 128 KiB × 4, larger × 8).
//! * `entry` – changing the entry that starts a script. The script is counted where the scan found
//!   it; if the scan did not find it (network path, other drive), the effort of a typical script
//!   (`VBS-101`) is added once per distinct script.
//! * Items on several machines are counted once – the change is assumed to be rolled out
//!   centrally (Group Policy, Intune, RMM). Identical copies (same rule and content) are counted
//!   with the first one; Windows components are not counted (Microsoft maintains them).
//! * Rules from a newer collector that this version does not know count as a manual check.

use std::collections::HashSet;

use vbs_core::model::FindingStatus;
use vbs_core::rules::{self, EffortBasis};

use crate::assessment::Item;
use crate::paths;

/// Effort of one item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effort {
    /// Counted in the totals.
    Hours(Estimate),
    /// Same rule and identical content as another item (its number), which carries the effort.
    SameAs(usize),
    /// Part of Windows – maintained by Microsoft, no migration work.
    Windows,
}

impl Default for Effort {
    fn default() -> Self {
        Effort::Hours(Estimate { min: 0.0, max: 0.0, size_factor: 1, typical_script: false })
    }
}

impl Effort {
    /// The counted range in hours, if the item is counted.
    pub fn hours(&self) -> Option<(f64, f64)> {
        match self {
            Effort::Hours(estimate) => Some((estimate.min, estimate.max)),
            Effort::SameAs(_) | Effort::Windows => None,
        }
    }
}

/// A range in hours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub min: f64,
    pub max: f64,
    /// Factor for the size of the script file (1 = typical size).
    pub size_factor: u32,
    /// Includes a typical script that the entry starts but the scan did not find.
    pub typical_script: bool,
}

/// Range for rules this version does not know (from a newer collector): a manual check.
const UNKNOWN_RULE: (f64, f64) = (0.25, 1.0);

/// Size factor of a script file.
pub fn size_factor(bytes: Option<u64>) -> u32 {
    match bytes {
        None => 1,
        Some(size) if size <= 8 * 1024 => 1,
        Some(size) if size <= 32 * 1024 => 2,
        Some(size) if size <= 128 * 1024 => 4,
        Some(_) => 8,
    }
}

/// Fills in the effort of every item; `items` must be in priority order (the first of several
/// identical items carries the effort).
pub fn estimate(items: &mut [Item]) {
    let catalog = rules::catalog();
    let typical = catalog.rule("VBS-101").map_or((1.0, 4.0), |rule| (rule.effort.min_hours, rule.effort.max_hours));
    let mut counted_scripts: HashSet<String> = HashSet::new();
    for item in items.iter_mut() {
        item.effort = if !item.is_own() {
            Effort::Windows
        } else if let Some(number) = item.same_content_as {
            Effort::SameAs(number)
        } else {
            let ((min, max), basis) = catalog.rule(&item.rule).map_or((UNKNOWN_RULE, EffortBasis::Fixed), |rule| {
                ((rule.effort.min_hours, rule.effort.max_hours), rule.effort.basis)
            });
            let mut estimate = Estimate { min, max, size_factor: 1, typical_script: false };
            match basis {
                EffortBasis::Fixed => {}
                EffortBasis::ScriptSize => {
                    estimate.size_factor = size_factor(item.first().file.as_ref().map(|file| file.size));
                    estimate.min *= f64::from(estimate.size_factor);
                    estimate.max *= f64::from(estimate.size_factor);
                }
                EffortBasis::Entry => {
                    if item.starts.is_empty() && item.status == FindingStatus::Detected {
                        let missing = missing_scripts(item);
                        if missing.iter().any(|script| !counted_scripts.contains(script)) {
                            estimate.min += typical.0;
                            estimate.max += typical.1;
                            estimate.typical_script = true;
                        }
                        counted_scripts.extend(missing);
                    }
                }
            }
            Effort::Hours(estimate)
        };
    }
}

/// Script files an entry starts that the scan did not find: absolute paths as such, names written
/// relative or with variables by their file name.
fn missing_scripts(item: &Item) -> Vec<String> {
    item.occurrences
        .iter()
        .filter_map(|occurrence| occurrence.target.as_deref())
        .map(paths::normalize)
        .filter(|target| paths::is_script_file(target))
        .map(|target| if paths::is_absolute(&target) { target } else { paths::file_name(&target).to_owned() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assessment::Assessment;
    use crate::assessment::tests::{finding, scan, script, task};
    use time::macros::datetime;
    use vbs_core::model::{Activation, LocationKind};

    const T: time::OffsetDateTime = datetime!(2026-09-26 10:00 UTC);

    fn hours(assessment: &Assessment, path: &str) -> Estimate {
        let item = assessment.items.iter().find(|item| item.first().location.path == path).unwrap();
        match item.effort {
            Effort::Hours(estimate) => estimate,
            other => panic!("{path}: {other:?}"),
        }
    }

    #[test]
    fn size_classes() {
        assert_eq!(size_factor(None), 1);
        assert_eq!(size_factor(Some(8 * 1024)), 1);
        assert_eq!(size_factor(Some(8 * 1024 + 1)), 2);
        assert_eq!(size_factor(Some(100_000)), 4);
        assert_eq!(size_factor(Some(10_000_000)), 8);
    }

    #[test]
    fn scripts_scale_with_size_and_entries_add_missing_scripts_once() {
        let files = vec![scan(
            "SRV-1",
            T,
            vec![
                script(r"C:\Small.vbs", "s", 2_000),
                script(r"C:\Large.vbs", "l", 60_000),
                task(r"\Found", r"C:\Small.vbs"),
                task(r"\Missing A", r"\\fs01\scripts\nightly.vbs"),
                task(r"\Missing B", r"\\FS01\Scripts\Nightly.vbs"),
                task(r"\Inline", "mshta.exe"),
            ],
        )];
        let assessment = Assessment::build(&files, None);
        let rule = |id: &str| rules::catalog().rule(id).unwrap().effort;
        let (script_rule, task_rule) = (rule("VBS-101"), rule("VBS-301"));
        let small = hours(&assessment, r"C:\Small.vbs");
        assert_eq!((small.min, small.max, small.size_factor), (script_rule.min_hours, script_rule.max_hours, 1));
        let large = hours(&assessment, r"C:\Large.vbs");
        assert_eq!((large.min, large.size_factor), (script_rule.min_hours * 4.0, 4));
        let found = hours(&assessment, r"\Found");
        assert_eq!((found.min, found.typical_script), (task_rule.min_hours, false));
        let missing: Vec<Estimate> =
            [r"\Missing A", r"\Missing B"].iter().map(|path| hours(&assessment, path)).collect();
        assert_eq!(missing.iter().filter(|estimate| estimate.typical_script).count(), 1, "one script, counted once");
        assert!(!hours(&assessment, r"\Inline").typical_script, "no script file to count");
        let total: f64 = assessment.items.iter().filter_map(|item| item.effort.hours()).map(|(min, _)| min).sum();
        assert!((assessment.summary.effort_min - total).abs() < 1e-9);
    }

    #[test]
    fn not_checkable_items_are_a_manual_check() {
        let files =
            vec![scan("PC", T, vec![finding("VBS-600", Activation::Dormant, LocationKind::File, r"C:\x.xlsm")])];
        let assessment = Assessment::build(&files, None);
        let estimate = hours(&assessment, r"C:\x.xlsm");
        let rule = rules::catalog().rule("VBS-600").unwrap().effort;
        assert_eq!((estimate.min, estimate.max), (rule.min_hours, rule.max_hours));
    }
}
