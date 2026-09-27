//! Merging result files into one assessment.
//!
//! 1. **Machines** – the newest scan per machine is evaluated; older scans of the same machine
//!    and machines beyond the edition's limit are set aside and listed, never dropped silently.
//! 2. **Items** – every finding is an occurrence on one machine. Occurrences of the same thing
//!    become one item: the same file on a network share seen from several machines; the same entry,
//!    or a file with the same content at the same path, on several machines; Windows' own files,
//!    grouped by name ([`Origin::Windows`]).
//! 3. **Links** – an entry (task, autostart value, service, shortcut, calling script, macro, log
//!    record) that starts a script the scan found is linked to it; the script inherits the entry's
//!    activation, so a "dormant" file that a task runs every night counts as running automatically.
//! 4. **Risk** – automatic > use recorded in a log > Office macro > started by users > installer >
//!    dormant file; `breaks` before `review` ([`Risk`]).
//! 5. **Effort** – rules of thumb from the rule catalog ([`crate::effort`]); identical copies and
//!    Windows components are not counted twice or at all.
//! 6. **Coverage** – what the scans could and could not see, including the time span of the event
//!    logs ([`crate::coverage`]); never a completeness promise.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use time::OffsetDateTime;
use uuid::Uuid;
use vbs_core::model::{
    Activation, Classification, Coverage, Detail, Evidence, FileFacts, Finding, FindingKind, FindingStatus, Generator,
    Location, LocationKind, Machine, NotCheckableReason, ScanResult, Scope,
};

use crate::coverage::{self, CoverageSummary};
use crate::effort::{self, Effort};
use crate::import::ImportedFile;
use crate::paths;

/// Identity of a machine across scans: host name, DNS domain and the pseudonymous machine ID.
///
/// All three are compared, so cloned machines that share a machine ID but not a name stay apart,
/// and a renamed machine counts as a new one (nothing is merged by mistake).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MachineKey {
    hostname: String,
    domain: String,
    machine_id: String,
}

impl MachineKey {
    pub fn of(result: &ScanResult) -> Self {
        let machine = &result.machine;
        Self {
            hostname: machine.hostname.to_lowercase(),
            domain: machine.domain.as_deref().unwrap_or_default().to_lowercase(),
            machine_id: machine.machine_id.as_deref().unwrap_or_default().to_lowercase(),
        }
    }
}

/// One evaluated machine (the newest scan of it).
#[derive(Debug, Clone)]
pub struct MachineInfo {
    pub scan_id: Uuid,
    pub file: PathBuf,
    pub machine: Machine,
    pub generator: Generator,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub scope: Scope,
    pub coverage: Coverage,
    pub detected: usize,
    pub not_checkable: usize,
}

/// A result file that was loaded but is not part of the assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAside {
    pub file: PathBuf,
    pub hostname: String,
    pub started_at: OffsetDateTime,
    pub why: SetAsideReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAsideReason {
    /// A newer scan of the same machine is evaluated instead.
    Superseded,
    /// The edition's machine limit was reached.
    MachineLimit,
}

impl SetAsideReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SetAsideReason::Superseded => "superseded",
            SetAsideReason::MachineLimit => "machineLimit",
        }
    }
}

/// Where an item comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// Scripts, entries and documents of the organization (or of installed products).
    Own,
    /// Part of Windows: files in the component store, identical copies of them (same content),
    /// Windows data such as the User Access Logging databases, and the Windows folder of container
    /// image layers. Maintained by Microsoft – listed, but no migration work.
    Windows,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Own => "own",
            Origin::Windows => "windows",
        }
    }
}

/// Risk of an item, derived from how it runs and how certain the impact is.
///
/// | | automatic, logged | macro, manual, installer | dormant |
/// |---|---|---|---|
/// | `breaks` | high | medium | low |
/// | `review` | medium | low | low |
///
/// Windows components are `info`: Microsoft maintains them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Risk {
    High,
    Medium,
    Low,
    Info,
}

impl Risk {
    pub const ALL: [Risk; 4] = [Risk::High, Risk::Medium, Risk::Low, Risk::Info];

    /// `risk.<value>` translation keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::High => "high",
            Risk::Medium => "medium",
            Risk::Low => "low",
            Risk::Info => "info",
        }
    }

    fn of(origin: Origin, classification: &Classification, activation: &Activation) -> Risk {
        if origin == Origin::Windows {
            return Risk::Info;
        }
        let rank = activation_rank(activation);
        match (*classification == Classification::Breaks, rank) {
            (true, 5..) => Risk::High,
            (true, 2..=4) | (false, 5..) => Risk::Medium,
            _ => Risk::Low,
        }
    }
}

/// Rank of an activation for the risk: automatic 6, logged 5, macro 4, manual 3, installer 2,
/// dormant 1. Values from newer versions rank like `manual` – never as harmless as a dormant file.
pub fn activation_rank(activation: &Activation) -> u8 {
    match activation {
        Activation::Automatic => 6,
        Activation::Logged => 5,
        Activation::Macro => 4,
        Activation::Manual | Activation::Unknown(_) => 3,
        Activation::Installer => 2,
        Activation::Dormant => 1,
    }
}

/// One finding on one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// Index into [`Assessment::machines`].
    pub machine: usize,
    /// ID of the finding in that machine's result file (`f12`).
    pub finding: String,
    pub location: Location,
    pub target: Option<String>,
    pub file: Option<FileFacts>,
    pub activation: Activation,
}

/// One dependency (or item that could not be checked), merged over all machines.
#[derive(Debug, Clone)]
pub struct Item {
    /// Rank in the priority order, starting at 1 – shown as "#12".
    pub number: usize,
    pub rule: String,
    pub kind: FindingKind,
    /// Effective classification: values from newer versions count as `review`.
    pub classification: Classification,
    pub status: FindingStatus,
    pub reason: Option<NotCheckableReason>,
    pub origin: Origin,
    /// The highest activation of its occurrences and of the entries that start it.
    pub activation: Activation,
    /// The activation the collector reported (highest over the occurrences).
    pub reported_activation: Activation,
    pub risk: Risk,
    /// Items (numbers) that start this script or record its use.
    pub started_by: Vec<usize>,
    /// Scripts (item numbers) this item starts.
    pub starts: Vec<usize>,
    /// Evidence and details of the first occurrence.
    pub evidence: Vec<Evidence>,
    pub details: BTreeMap<String, Detail>,
    /// All occurrences, ordered by machine and location.
    pub occurrences: Vec<Occurrence>,
    /// Number of distinct machines.
    pub machines: usize,
    /// An item with the same rule and identical file content that carries the effort.
    pub same_content_as: Option<usize>,
    pub effort: Effort,
}

impl Item {
    pub fn first(&self) -> &Occurrence {
        &self.occurrences[0]
    }

    pub fn is_own(&self) -> bool {
        self.origin == Origin::Own
    }
}

/// Counts over the items of the organization (Windows components separately).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub machines: usize,
    /// Own items (all risks, including not checkable ones).
    pub items: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub breaks: usize,
    pub review: usize,
    pub not_checkable: usize,
    /// Own items of the security rule (hard-coded credentials).
    pub credentials: usize,
    /// Windows component items and their occurrences.
    pub windows_items: usize,
    pub windows_occurrences: usize,
    /// Rule-of-thumb total of the counted items, in hours.
    pub effort_min: f64,
    pub effort_max: f64,
    pub by_kind: Vec<KindTally>,
}

/// Own items of one finding type.
#[derive(Debug, Clone, PartialEq)]
pub struct KindTally {
    pub kind: FindingKind,
    pub items: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub not_checkable: usize,
    /// Distinct machines with at least one of these items.
    pub machines: usize,
    pub effort_min: f64,
    pub effort_max: f64,
}

/// The merged view of all loaded result files.
#[derive(Debug, Clone)]
pub struct Assessment {
    pub machines: Vec<MachineInfo>,
    pub set_aside: Vec<SetAside>,
    /// All items in priority order (`items[i].number == i + 1`).
    pub items: Vec<Item>,
    pub summary: Summary,
    pub coverage: CoverageSummary,
}

impl Assessment {
    /// Builds the assessment of `files` (in load order). `machine_limit` is the edition's limit
    /// of distinct machines (checked on import as well).
    pub fn build(files: &[ImportedFile], machine_limit: Option<usize>) -> Assessment {
        let (chosen, set_aside) = choose_scans(files, machine_limit);
        let machines: Vec<MachineInfo> = chosen.iter().map(|&index| machine_info(&files[index])).collect();
        let results: Vec<&ScanResult> = chosen.iter().map(|&index| &files[index].result).collect();

        let mut drafts = group(&results);
        let links = link(&mut drafts);
        let mut items = finish(drafts, &links, &results);
        effort::estimate(&mut items);
        let summary = summarize(&items, machines.len());
        let coverage = coverage::summarize(&machines, &items);
        Assessment { machines, set_aside, items, summary, coverage }
    }

    pub fn item(&self, number: usize) -> Option<&Item> {
        number.checked_sub(1).and_then(|index| self.items.get(index))
    }

    /// Own items (the organization's migration work) in priority order.
    pub fn own_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|item| item.is_own())
    }

    /// Windows components in priority order.
    pub fn windows_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|item| !item.is_own())
    }
}

/// The newest scan per machine, in load order, within the machine limit.
fn choose_scans(files: &[ImportedFile], machine_limit: Option<usize>) -> (Vec<usize>, Vec<SetAside>) {
    let mut set_aside = Vec::new();
    let mut newest: HashMap<MachineKey, usize> = HashMap::new();
    let set_aside_entry = |file: &ImportedFile, why| SetAside {
        file: file.path.clone(),
        hostname: file.result.machine.hostname.clone(),
        started_at: file.result.started_at,
        why,
    };
    for (index, file) in files.iter().enumerate() {
        match newest.get(&MachineKey::of(&file.result)).copied() {
            Some(other) if files[other].result.started_at >= file.result.started_at => {
                set_aside.push(set_aside_entry(file, SetAsideReason::Superseded));
            }
            Some(other) => {
                set_aside.push(set_aside_entry(&files[other], SetAsideReason::Superseded));
                newest.insert(MachineKey::of(&file.result), index);
            }
            None => {
                newest.insert(MachineKey::of(&file.result), index);
            }
        }
    }
    let mut chosen: Vec<usize> = newest.into_values().collect();
    chosen.sort_unstable();
    if let Some(limit) = machine_limit
        && chosen.len() > limit
    {
        for &index in &chosen[limit..] {
            set_aside.push(set_aside_entry(&files[index], SetAsideReason::MachineLimit));
        }
        chosen.truncate(limit);
    }
    set_aside.sort_by(|a, b| a.file.cmp(&b.file));
    (chosen, set_aside)
}

fn machine_info(file: &ImportedFile) -> MachineInfo {
    let result = &file.result;
    let not_checkable = result.findings.iter().filter(|f| f.status == FindingStatus::NotCheckable).count();
    MachineInfo {
        scan_id: result.scan_id,
        file: file.path.clone(),
        machine: result.machine.clone(),
        generator: result.generator.clone(),
        started_at: result.started_at,
        finished_at: result.finished_at,
        scope: result.scope.clone(),
        coverage: result.coverage.clone(),
        detected: result.findings.len() - not_checkable,
        not_checkable,
    }
}

/// Grouping key of an occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// Windows' own files: one item per rule, status and file name.
    Windows { rule: String, status: String, reason: String, name: String },
    /// A file on a network share: the same path is the same file, whichever machine saw it.
    Network { rule: String, status: String, reason: String, path: String, item: String },
    /// Everything else: the same entry, or the same file content at the same path.
    Local {
        rule: String,
        status: String,
        reason: String,
        location: String,
        path: String,
        item: String,
        target: String,
        sha256: String,
    },
}

/// An item while it is being built (indices refer to the draft list).
struct Draft<'a> {
    finding: &'a Finding,
    origin: Origin,
    occurrences: Vec<Occurrence>,
    /// Normalised location path of each occurrence (same order).
    paths: Vec<String>,
}

fn status_text(finding: &Finding) -> (String, String) {
    (
        finding.status.as_str().to_owned(),
        finding.reason.as_ref().map(|reason| reason.as_str().to_owned()).unwrap_or_default(),
    )
}

/// Groups the findings of all machines into drafts, in a deterministic order.
fn group<'a>(results: &[&'a ScanResult]) -> Vec<Draft<'a>> {
    // Content that exists in a component store is Windows' own, wherever a copy of it lies.
    let component_hashes: HashSet<&str> = results
        .iter()
        .flat_map(|result| &result.findings)
        .filter(|finding| {
            finding.location.kind == LocationKind::File
                && paths::in_component_store(&paths::normalize(&finding.location.path))
        })
        .filter_map(|finding| finding.file.as_ref()?.sha256.as_deref())
        .collect();

    let mut drafts: Vec<Draft<'a>> = Vec::new();
    let mut index: HashMap<Key, usize> = HashMap::new();
    for (machine, result) in results.iter().enumerate() {
        for finding in &result.findings {
            let path = paths::normalize(&finding.location.path);
            let is_file = finding.location.kind == LocationKind::File;
            let sha256 = finding.file.as_ref().and_then(|file| file.sha256.as_deref());
            let windows = is_file
                && (paths::is_windows_location(&path) || sha256.is_some_and(|hash| component_hashes.contains(hash)));
            let origin = if windows { Origin::Windows } else { Origin::Own };
            let (status, reason) = status_text(finding);
            let rule = finding.rule.clone();
            let item = finding.location.item.as_deref().unwrap_or_default().to_lowercase();
            let key = if windows {
                Key::Windows { rule, status, reason, name: paths::file_name(&path).to_owned() }
            } else if is_file && paths::is_network(&path) {
                Key::Network { rule, status, reason, path: path.clone(), item }
            } else {
                Key::Local {
                    rule,
                    status,
                    reason,
                    location: finding.location.kind.as_str().to_owned(),
                    path: path.clone(),
                    item,
                    target: finding.target.as_deref().map(paths::normalize).unwrap_or_default(),
                    sha256: sha256.unwrap_or_default().to_owned(),
                }
            };
            let occurrence = Occurrence {
                machine,
                finding: finding.id.clone(),
                location: finding.location.clone(),
                target: finding.target.clone(),
                file: finding.file.clone(),
                activation: finding.activation.clone(),
            };
            let draft = *index.entry(key).or_insert_with(|| {
                drafts.push(Draft { finding, origin, occurrences: Vec::new(), paths: Vec::new() });
                drafts.len() - 1
            });
            drafts[draft].occurrences.push(occurrence);
            drafts[draft].paths.push(path);
        }
    }
    drafts
}

/// Links between drafts: `starts[i]` are the scripts draft `i` starts.
struct Links {
    starts: Vec<Vec<usize>>,
    started_by: Vec<Vec<usize>>,
}

/// Findings that can start a script, and scripts that can be started.
fn starts_scripts(kind: &FindingKind) -> bool {
    !matches!(kind, FindingKind::ScriptFile | FindingKind::HardcodedCredential)
}

fn is_script(draft: &Draft<'_>) -> bool {
    matches!(draft.finding.kind, FindingKind::ScriptFile | FindingKind::ScriptInvocation)
        && draft.finding.location.kind == LocationKind::File
        && draft.finding.status == FindingStatus::Detected
}

fn link(drafts: &mut [Draft<'_>]) -> Links {
    // Where the scripts are: (machine, path) and network path → drafts; (machine, file name) for
    // targets written relative or with variables.
    let mut by_path: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
    let mut by_network_path: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_name: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
    for (index, draft) in drafts.iter().enumerate() {
        if !is_script(draft) {
            continue;
        }
        for (occurrence, path) in draft.occurrences.iter().zip(&draft.paths) {
            if paths::is_network(path) {
                by_network_path.entry(path.as_str()).or_default().push(index);
            } else {
                by_path.entry((occurrence.machine, path.as_str())).or_default().push(index);
            }
            by_name.entry((occurrence.machine, paths::file_name(path))).or_default().push(index);
        }
    }
    let mut starts: Vec<Vec<usize>> = vec![Vec::new(); drafts.len()];
    for (index, draft) in drafts.iter().enumerate() {
        if !starts_scripts(&draft.finding.kind) {
            continue;
        }
        for occurrence in &draft.occurrences {
            let Some(target) = occurrence.target.as_deref().map(paths::normalize) else { continue };
            let found = if paths::is_network(&target) {
                by_network_path.get(target.as_str())
            } else if paths::is_absolute(&target) {
                by_path.get(&(occurrence.machine, target.as_str()))
            } else {
                // `%ScriptDir%\backup.vbs`, `backup.vbs`: only a unique name on that machine counts.
                by_name
                    .get(&(occurrence.machine, paths::file_name(&target)))
                    .filter(|candidates| candidates.iter().collect::<HashSet<_>>().len() == 1)
            };
            for &script in found.into_iter().flatten() {
                if script != index && !starts[index].contains(&script) {
                    starts[index].push(script);
                }
            }
        }
    }
    let mut started_by: Vec<Vec<usize>> = vec![Vec::new(); drafts.len()];
    for (index, scripts) in starts.iter().enumerate() {
        for &script in scripts {
            started_by[script].push(index);
        }
    }
    Links { starts, started_by }
}

/// The highest activation of a draft's occurrences.
fn reported_activation(draft: &Draft<'_>) -> Activation {
    draft
        .occurrences
        .iter()
        .map(|occurrence| &occurrence.activation)
        .max_by_key(|activation| activation_rank(activation))
        .cloned()
        .unwrap_or(Activation::Dormant)
}

/// Activations after following the links: a script is at least as active as what starts it.
fn propagate(drafts: &[Draft<'_>], links: &Links) -> Vec<Activation> {
    let mut activation: Vec<Activation> = drafts.iter().map(reported_activation).collect();
    // Chains (task → batch file → .vbs) are short; the bound only guards against cycles.
    for _ in 0..16 {
        let mut changed = false;
        for (index, starters) in links.started_by.iter().enumerate() {
            for &starter in starters {
                if activation_rank(&activation[starter]) > activation_rank(&activation[index]) {
                    activation[index] = activation[starter].clone();
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    activation
}

/// Turns drafts into items in priority order.
fn finish(drafts: Vec<Draft<'_>>, links: &Links, results: &[&ScanResult]) -> Vec<Item> {
    let activations = propagate(&drafts, links);
    let mut items: Vec<(usize, Item)> = drafts
        .into_iter()
        .enumerate()
        .map(|(index, mut draft)| {
            // Occurrences by machine, then location; the first one represents the item.
            let mut order: Vec<usize> = (0..draft.occurrences.len()).collect();
            order.sort_by(|&a, &b| {
                let (a, b) = (&draft.occurrences[a], &draft.occurrences[b]);
                (a.machine, &a.location.path, &a.location.item).cmp(&(b.machine, &b.location.path, &b.location.item))
            });
            let mut occurrences: Vec<Option<Occurrence>> = draft.occurrences.drain(..).map(Some).collect();
            let occurrences: Vec<Occurrence> = order.iter().filter_map(|&i| occurrences[i].take()).collect();
            let first = &occurrences[0];
            let finding = results[first.machine]
                .findings
                .iter()
                .find(|finding| finding.id == first.finding)
                .unwrap_or(draft.finding);
            let classification = finding.classification.effective();
            let activation = activations[index].clone();
            let risk = Risk::of(draft.origin, &classification, &activation);
            let machines = occurrences.iter().map(|o| o.machine).collect::<HashSet<_>>().len();
            let item = Item {
                number: 0,
                rule: finding.rule.clone(),
                kind: finding.kind.clone(),
                classification,
                status: finding.status.clone(),
                reason: finding.reason.clone(),
                origin: draft.origin,
                reported_activation: reported_activation_of(&occurrences),
                activation,
                risk,
                started_by: Vec::new(),
                starts: Vec::new(),
                evidence: finding.evidence.clone(),
                details: finding.details.clone(),
                occurrences,
                machines,
                same_content_as: None,
                effort: Effort::default(),
            };
            (index, item)
        })
        .collect();

    items.sort_by(|(_, a), (_, b)| priority(a).cmp(&priority(b)));
    let mut numbers = vec![0usize; items.len()];
    for (rank, (draft, item)) in items.iter_mut().enumerate() {
        item.number = rank + 1;
        numbers[*draft] = rank + 1;
    }
    let mut items: Vec<Item> = items
        .into_iter()
        .map(|(draft, mut item)| {
            item.starts = links.starts[draft].iter().map(|&d| numbers[d]).collect();
            item.started_by = links.started_by[draft].iter().map(|&d| numbers[d]).collect();
            item.starts.sort_unstable();
            item.started_by.sort_unstable();
            item
        })
        .collect();
    mark_identical_content(&mut items);
    items
}

fn reported_activation_of(occurrences: &[Occurrence]) -> Activation {
    occurrences
        .iter()
        .map(|occurrence| &occurrence.activation)
        .max_by_key(|activation| activation_rank(activation))
        .cloned()
        .unwrap_or(Activation::Dormant)
}

/// Sort key: own items first, then risk, how it runs, how certain, detected before not
/// checkable, spread over machines, rule and location.
fn priority(item: &Item) -> impl Ord + '_ {
    let first = item.first();
    (
        item.origin,
        item.risk,
        Reverse(activation_rank(&item.activation)),
        item.classification != Classification::Breaks,
        item.status != FindingStatus::Detected,
        Reverse(item.machines),
        item.rule.as_str(),
        first.location.path.to_lowercase(),
        first.location.item.as_deref().unwrap_or_default().to_lowercase(),
    )
}

/// Items with the same rule, module and file content at another path (a script copied to several
/// folders or machines): the first one in priority order carries the effort.
fn mark_identical_content(items: &mut [Item]) {
    let mut first: HashMap<(String, String, String), usize> = HashMap::new();
    for item in items.iter_mut() {
        if !item.is_own() || item.status != FindingStatus::Detected {
            continue;
        }
        let occurrence = item.first();
        let Some(sha256) = occurrence.file.as_ref().and_then(|file| file.sha256.clone()) else { continue };
        let module = occurrence.location.item.as_deref().unwrap_or_default().to_lowercase();
        match first.get(&(item.rule.clone(), module.clone(), sha256.clone())) {
            Some(&number) => item.same_content_as = Some(number),
            None => {
                first.insert((item.rule.clone(), module, sha256), item.number);
            }
        }
    }
}

fn summarize(items: &[Item], machines: usize) -> Summary {
    let mut summary = Summary { machines, ..Summary::default() };
    let mut kinds: BTreeMap<String, (KindTally, HashSet<usize>)> = BTreeMap::new();
    for item in items {
        if !item.is_own() {
            summary.windows_items += 1;
            summary.windows_occurrences += item.occurrences.len();
            continue;
        }
        summary.items += 1;
        let (min, max) = item.effort.hours().unwrap_or((0.0, 0.0));
        summary.effort_min += min;
        summary.effort_max += max;
        let (tally, tally_machines) = &mut kinds.entry(item.kind.as_str().to_owned()).or_insert_with(|| {
            let tally = KindTally {
                kind: item.kind.clone(),
                items: 0,
                high: 0,
                medium: 0,
                low: 0,
                not_checkable: 0,
                machines: 0,
                effort_min: 0.0,
                effort_max: 0.0,
            };
            (tally, HashSet::new())
        });
        tally.items += 1;
        tally.effort_min += min;
        tally.effort_max += max;
        tally_machines.extend(item.occurrences.iter().map(|occurrence| occurrence.machine));
        match item.risk {
            Risk::High => {
                summary.high += 1;
                tally.high += 1;
            }
            Risk::Medium => {
                summary.medium += 1;
                tally.medium += 1;
            }
            Risk::Low | Risk::Info => {
                summary.low += 1;
                tally.low += 1;
            }
        }
        if item.status == FindingStatus::NotCheckable {
            summary.not_checkable += 1;
            tally.not_checkable += 1;
        } else if item.classification == Classification::Breaks {
            summary.breaks += 1;
        } else {
            summary.review += 1;
        }
        if item.kind == FindingKind::HardcodedCredential {
            summary.credentials += 1;
        }
    }
    // Finding types in the order of the result format; types from newer versions at the end.
    let order = |kind: &FindingKind| FindingKind::KNOWN.iter().position(|known| known == kind).unwrap_or(usize::MAX);
    summary.by_kind = kinds
        .into_values()
        .map(|(mut tally, machines)| {
            tally.machines = machines.len();
            tally
        })
        .collect();
    summary.by_kind.sort_by_key(|tally| (order(&tally.kind), tally.kind.as_str().to_owned()));
    summary
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::effort::Effort;
    use time::macros::datetime;
    use vbs_core::model::*;

    /// A result of one machine with the given findings.
    pub(crate) fn scan(hostname: &str, started: OffsetDateTime, findings: Vec<Finding>) -> ImportedFile {
        let result = ScanResult {
            format: vbs_core::media_type().into(),
            schema_version: SCHEMA_VERSION,
            scan_id: Uuid::new_v4(),
            generator: Generator {
                product: "test".into(),
                component: "collector".into(),
                version: "0.1.0".into(),
                rules_as_of: "2026-09-26".into(),
                platform: "windows".into(),
            },
            started_at: started,
            finished_at: started,
            machine: Machine {
                hostname: hostname.into(),
                fqdn: None,
                domain: Some("corp.example".into()),
                machine_id: Some(format!("id-{}", hostname.to_lowercase())),
                os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
            },
            scope: Scope { local_drives: true, paths: vec![], network_paths: vec![], system_sources: true },
            coverage: Coverage { mode: CoverageMode::Full, elevated: true, limitations: vec![], sources: vec![] },
            findings: findings
                .into_iter()
                .enumerate()
                .map(|(index, mut finding)| {
                    finding.id = format!("f{}", index + 1);
                    finding
                })
                .collect(),
        };
        ImportedFile { path: PathBuf::from(format!("{hostname}.vbscout")), result, unknown_values: 0 }
    }

    /// A finding as the collector reports it (classification and kind from the catalog).
    pub(crate) fn finding(rule: &str, activation: Activation, kind: LocationKind, path: &str) -> Finding {
        let catalog_rule = vbs_core::rules::catalog().rule(rule).expect("rule exists");
        Finding {
            id: String::new(),
            rule: rule.into(),
            kind: catalog_rule.kind.clone(),
            classification: catalog_rule.classification.clone(),
            status: if rule.ends_with("00") { FindingStatus::NotCheckable } else { FindingStatus::Detected },
            reason: rule.ends_with("00").then_some(NotCheckableReason::AccessDenied),
            activation,
            location: Location { kind, path: path.into(), item: None },
            target: None,
            file: None,
            evidence: vec![],
            details: BTreeMap::new(),
        }
    }

    pub(crate) fn script(path: &str, sha256: &str, size: u64) -> Finding {
        let mut finding = finding("VBS-101", Activation::Dormant, LocationKind::File, path);
        finding.file = Some(FileFacts {
            size,
            modified_at: None,
            sha256: Some(sha256.into()),
            network: paths::is_network(&paths::normalize(path)),
        });
        finding
    }

    pub(crate) fn task(name: &str, target: &str) -> Finding {
        let mut finding = finding("VBS-301", Activation::Automatic, LocationKind::ScheduledTask, name);
        finding.target = Some(target.into());
        finding
    }

    const T0: OffsetDateTime = datetime!(2026-09-20 10:00 UTC);
    const T1: OffsetDateTime = datetime!(2026-09-26 10:00 UTC);

    #[test]
    fn newest_scan_per_machine_and_machine_limit() {
        let files = vec![
            scan("PC-1", T0, vec![script(r"C:\a.vbs", "aa", 10)]),
            scan("PC-2", T0, vec![]),
            scan("pc-1", T1, vec![]),
            scan("PC-3", T1, vec![]),
        ];
        let assessment = Assessment::build(&files, None);
        let hosts: Vec<&str> = assessment.machines.iter().map(|m| m.machine.hostname.as_str()).collect();
        assert_eq!(hosts, ["PC-2", "pc-1", "PC-3"], "load order of the evaluated scans");
        assert_eq!(assessment.set_aside.len(), 1);
        assert_eq!(assessment.set_aside[0].why, SetAsideReason::Superseded);
        assert_eq!(assessment.set_aside[0].started_at, T0);
        assert!(assessment.items.is_empty(), "the old scan's finding is not evaluated");

        let limited = Assessment::build(&files, Some(2));
        assert_eq!(limited.machines.len(), 2);
        assert_eq!(limited.set_aside.iter().filter(|s| s.why == SetAsideReason::MachineLimit).count(), 1);

        // Same name, different machine ID (e.g. another customer): two machines.
        let mut other = scan("PC-2", T1, vec![]);
        other.result.machine.machine_id = Some("another".into());
        let both = Assessment::build(&[scan("PC-2", T0, vec![]), other], None);
        assert_eq!(both.machines.len(), 2);
    }

    #[test]
    fn network_files_and_deployed_copies_are_merged() {
        let share = r"\\FS01\Scripts\logon.vbs";
        let files = vec![
            scan("PC-1", T1, vec![script(share, "11", 100), script(r"C:\Tools\inv.vbs", "22", 100)]),
            scan("PC-2", T1, vec![script(&share.to_lowercase(), "11", 100), script(r"c:\tools\INV.vbs", "22", 100)]),
            scan("PC-3", T1, vec![script(r"C:\Tools\inv.vbs", "33", 100)]),
        ];
        let assessment = Assessment::build(&files, None);
        assert_eq!(assessment.items.len(), 3, "{:#?}", assessment.items);
        let shared = assessment.items.iter().find(|item| item.first().location.path == share).unwrap();
        assert_eq!((shared.machines, shared.occurrences.len()), (2, 2));
        let deployed: Vec<&Item> =
            assessment.items.iter().filter(|item| item.first().location.path.ends_with("inv.vbs")).collect();
        assert_eq!(deployed.len(), 2, "a different version on PC-3 stays apart");
        assert_eq!(deployed.iter().map(|item| item.machines).sum::<usize>(), 3);
    }

    #[test]
    fn identical_content_is_counted_once() {
        let files =
            vec![scan("PC-1", T1, vec![script(r"C:\A\copy.vbs", "same", 100), script(r"C:\B\copy.vbs", "same", 100)])];
        let assessment = Assessment::build(&files, None);
        let [first, second] = assessment.items.as_slice() else { panic!("two items") };
        assert_eq!(first.same_content_as, None);
        assert_eq!(second.same_content_as, Some(first.number));
        assert!(matches!(second.effort, Effort::SameAs(number) if number == first.number));
        assert_eq!(assessment.summary.effort_min, first.effort.hours().unwrap().0);
    }

    #[test]
    fn scripts_inherit_the_activation_of_what_starts_them() {
        let mut invocation = finding("VBS-201", Activation::Dormant, LocationKind::File, r"C:\Jobs\run.cmd");
        invocation.target = Some(r"C:\Jobs\job.vbs".into());
        let mut alert =
            finding("VBS-502", Activation::Logged, LocationKind::EventLog, "Microsoft-Windows-Sysmon/Operational");
        alert.target = Some(r"c:\tools\report.vbs".into());
        let files = vec![scan(
            "SRV-1",
            T1,
            vec![
                script(r"C:\Jobs\job.vbs", "j", 100),
                invocation,
                task(r"\Nightly", r"C:\Jobs\run.cmd"),
                script(r"C:\Tools\report.vbs", "r", 100),
                alert,
                script(r"C:\Old\unused.vbs", "u", 100),
            ],
        )];
        let assessment = Assessment::build(&files, None);
        let by_path = |path: &str| assessment.items.iter().find(|item| item.first().location.path == path).unwrap();
        let job = by_path(r"C:\Jobs\job.vbs");
        assert_eq!(job.activation, Activation::Automatic, "task → run.cmd → job.vbs");
        assert_eq!(job.reported_activation, Activation::Dormant);
        assert_eq!(job.risk, Risk::High);
        let cmd = by_path(r"C:\Jobs\run.cmd");
        assert_eq!(cmd.starts, vec![job.number]);
        assert_eq!(job.started_by, vec![cmd.number]);
        assert_eq!(by_path(r"C:\Tools\report.vbs").activation, Activation::Logged);
        let unused = by_path(r"C:\Old\unused.vbs");
        assert_eq!((unused.activation.clone(), unused.risk), (Activation::Dormant, Risk::Low));
        // Priority: the high risks first, the dormant file last.
        assert_eq!(assessment.items.last().unwrap().number, unused.number);
        assert!(assessment.items.windows(2).all(|pair| pair[0].risk <= pair[1].risk));
    }

    #[test]
    fn relative_targets_link_by_unique_name_only() {
        let files = vec![scan(
            "PC-1",
            T1,
            vec![
                task(r"\A", "backup.vbs"),
                task(r"\B", r"%ScriptDir%\dup.vbs"),
                script(r"C:\Scripts\backup.vbs", "b", 10),
                script(r"C:\One\dup.vbs", "d1", 10),
                script(r"C:\Two\dup.vbs", "d2", 10),
            ],
        )];
        let assessment = Assessment::build(&files, None);
        let task_a = assessment.items.iter().find(|item| item.first().location.path == r"\A").unwrap();
        let task_b = assessment.items.iter().find(|item| item.first().location.path == r"\B").unwrap();
        assert_eq!(task_a.starts.len(), 1);
        assert!(task_b.starts.is_empty(), "two candidates – no guess");
    }

    #[test]
    fn windows_components_are_recognised_and_grouped() {
        let store = r"C:\Windows\WinSxS\amd64_microsoft-windows-slmgr_31bf3856ad364e35_10.0.26100.1_none_0\slmgr.vbs";
        let mut placeholder = finding(
            "VBS-100",
            Activation::Dormant,
            LocationKind::File,
            r"C:\ProgramData\docker\windowsfilter\158c\Files\Windows\System32\slmgr.vbs",
        );
        placeholder.reason = Some(NotCheckableReason::CloudPlaceholder);
        let mut ual = finding(
            "VBS-600",
            Activation::Dormant,
            LocationKind::File,
            r"C:\Windows\System32\LogFiles\Sum\Current.mdb",
        );
        ual.reason = Some(NotCheckableReason::Locked);
        let files = vec![
            scan(
                "SRV-1",
                T1,
                vec![
                    script(store, "slmgr-hash", 150_000),
                    script(r"C:\Windows\System32\slmgr.vbs", "slmgr-hash", 150_000),
                    script(r"C:\Windows\System32\custom.vbs", "own-hash", 100),
                    placeholder,
                    ual,
                ],
            ),
            // Another machine scanned without the component store: the same content is still Windows'.
            scan("SRV-2", T1, vec![script(r"C:\Windows\SysWOW64\slmgr.vbs", "slmgr-hash", 150_000)]),
        ];
        let assessment = Assessment::build(&files, None);
        let windows: Vec<&Item> = assessment.windows_items().collect();
        assert_eq!(windows.len(), 3, "{windows:#?}");
        let slmgr = windows.iter().find(|item| item.rule == "VBS-101").unwrap();
        assert_eq!((slmgr.occurrences.len(), slmgr.machines), (3, 2));
        assert!(windows.iter().all(|item| item.risk == Risk::Info && item.effort == Effort::Windows));
        let own: Vec<&Item> = assessment.own_items().collect();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].first().location.path, r"C:\Windows\System32\custom.vbs");
        assert_eq!(assessment.summary.windows_items, 3);
        assert_eq!(assessment.summary.windows_occurrences, 5);
        assert_eq!(assessment.summary.items, 1);
        assert_eq!(assessment.items[0].number, 1);
        assert!(assessment.items[0].is_own(), "own items come first");
    }

    #[test]
    fn risk_matrix() {
        use Activation::*;
        let breaks = Classification::Breaks;
        let review = Classification::Review;
        assert_eq!(Risk::of(Origin::Own, &breaks, &Automatic), Risk::High);
        assert_eq!(Risk::of(Origin::Own, &breaks, &Logged), Risk::High);
        assert_eq!(Risk::of(Origin::Own, &breaks, &Macro), Risk::Medium);
        assert_eq!(Risk::of(Origin::Own, &breaks, &Installer), Risk::Medium);
        assert_eq!(Risk::of(Origin::Own, &breaks, &Dormant), Risk::Low);
        assert_eq!(Risk::of(Origin::Own, &review, &Automatic), Risk::Medium);
        assert_eq!(Risk::of(Origin::Own, &review, &Macro), Risk::Low);
        assert_eq!(Risk::of(Origin::Own, &breaks, &Unknown("onDemand".into())), Risk::Medium);
        assert_eq!(Risk::of(Origin::Windows, &breaks, &Automatic), Risk::Info);
    }

    #[test]
    fn summary_counts_own_items() {
        let mut credential = finding("VBS-901", Activation::Dormant, LocationKind::File, r"C:\a.vbs");
        credential.file = Some(FileFacts { size: 10, modified_at: None, sha256: Some("a".into()), network: false });
        let files = vec![scan(
            "PC-1",
            T1,
            vec![
                script(r"C:\a.vbs", "a", 10),
                credential,
                task(r"\T", r"C:\a.vbs"),
                finding("VBS-300", Activation::Automatic, LocationKind::ScheduledTask, r"\Locked"),
            ],
        )];
        let assessment = Assessment::build(&files, None);
        let summary = &assessment.summary;
        assert_eq!(summary.items, 4);
        assert_eq!((summary.high, summary.medium, summary.low), (2, 1, 1));
        assert_eq!((summary.breaks, summary.review, summary.not_checkable), (2, 1, 1));
        assert_eq!(summary.credentials, 1);
        let kinds: Vec<&str> = summary.by_kind.iter().map(|tally| tally.kind.as_str()).collect();
        assert_eq!(kinds, ["scriptFile", "scheduledTask", "hardcodedCredential"]);
        assert!(summary.effort_min > 0.0 && summary.effort_min <= summary.effort_max);
    }
}
