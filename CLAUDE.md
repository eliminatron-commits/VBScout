# CLAUDE.md – VBScout

Auditor that finds VBScript dependencies in Windows environments before Microsoft disables VBScript by default
(phase 2 of the deprecation, expected around 2027 – see `docs/research-notes.md` for sources and dates). Target
groups: MSPs/IT service providers auditing customer environments, and IT departments of mid-sized organisations with
old in-house tools and Office macros. Second product of the vendor of Stepwright (`eliminatron-commits/stepwright`),
whose license system, key service, i18n structure and build pipeline it reuses. Sold via Paddle (merchant of
record); the code is **source-available** under FSL-1.1-ALv2. Built in 6 phases – **re-entry point: `PROGRESS.md`**.

Two programs:

* **Collector** (`vbs-collector`, always free): single portable CLI file (< 10 MB, no installation, Windows 10/11 and
  Server 2016–2025), distributed by the customer via Intune/GPO/RMM, run per machine. **Strictly read-only**; writes
  exactly one `.vbscout` result file.
* **Evaluation** (`vbs-app`, Tauri 2): merges any number of result files, de-duplicates, rates risk, adds migration
  hints and rule-of-thumb effort, and produces the management PDF and the technical Excel list.

## Commands

| Task | Command |
|---|---|
| Install JS dependencies | `npm ci` |
| All static checks | `npm run check:all` (config sync, i18n, offline policy, read-only policy + self-test, svelte-check) |
| Rust tests / lint | `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all --check` |
| Collector (release) | `cargo build --release -p vbs-collector` → `target/release/vbs-collector(.exe)` |
| Run the collector | `vbs-collector [--out <file\|folder>] [--path <folder>]… [--include-unc <\\server\share>]… [--files-only] [--lang <tag>] [--quiet]` |
| Evaluation (dev) | `npx tauri dev` |
| Evaluation (release) | `npx tauri build --no-bundle` → `target/release/vbs-app(.exe)` |
| App smoke test | `vbs-app --smoke-test[=secs]` – exit 0 = started, result file written + imported (2 = UI not ready, 3 = self-check failed); Linux headless: `xvfb-run -a dbus-run-session -- target/release/vbs-app --smoke-test` |
| Read-only trace | Linux: `bash scripts/readonly/linux.sh <collector> <folder>` · Windows (elevated pwsh): `scripts/readonly/windows.ps1 -Exe <exe> -ScanPath <folder>` |
| Network block test | Linux: `bash scripts/nettest/linux.sh [--gui] <out> -- <program> [args…]` · Windows (elevated pwsh): `scripts/nettest/windows.ps1 -Exe <exe> -Arguments … [-WebView]` |
| Scan performance (DoD #5) | `cargo test --release -p vbs-collector --test performance -- --ignored --nocapture` (100,000 files < 10 min) |
| Merge performance (DoD #5) | `cargo test --release -p vbs-evaluation --test merge_performance -- --ignored --nocapture` (1,000 result files < 1 min) |
| Reports without the app | `cargo run -p vbs-evaluation --example report -- --out <folder> [--lang de] [--sample 30] [--edition free\|organization:<name>\|msp:<company>] [--customer <name>] [--logo <png>] [--list] [result files or folders…]` |
| Check reports with other tools | `python3 scripts/reports/check.py <folder> [--free <folder>]` – PDF text via pdftotext (poppler), Excel via the Python standard library |
| System test with real Windows artefacts | elevated pwsh on a disposable machine: `scripts/systemtest/windows.ps1 -Exe <exe>`; limited scan without admin rights: `scripts/systemtest/nonadmin.ps1 -Exe <exe> -ScanPath <folder>` |
| Test collection | `cargo test -p vbs-collector --test corpus`; print what `positive/` yields: `… -- --ignored --nocapture print_positive_cases`; binary fixtures: `python3 tests/corpus/make-binaries.py` (needs msitools, hivex, pylnk3, olefile; for Office files Java 17+, mdbtools, oletools, msoffcrypto-tool – `tests/corpus/office_fixtures.py`) |
| Office readers vs. real files | `cargo test -p vbs-collector --test office_formats`; compare a folder of documents: `VBS_OFFICE_SAMPLES=<folder> cargo test -p vbs-collector --test office_formats -- --ignored --nocapture` |
| Program size | `node scripts/check-size.mjs <exe> <maxMB>` (collector: 10) |
| Program imports | `node scripts/check-imports.mjs <exe> [--out <list>]` – only reviewed Windows system DLLs, no C runtime, no network DLLs (`--self-test` in `check:all`) |
| Propagate product.json | `npm run sync:config` |
| Cross-check Windows code on Linux | `cargo clippy --target x86_64-pc-windows-msvc -p vbs-collector --all-targets -- -D warnings` |
| License keys (vendor, offline) | `cargo run -p license-keys -- keygen --out <file>` · `issue --key <file> --type organization\|msp --licensee <name> [--expires YYYY-MM-DD]` · `inspect <key>` |
| Key service (Worker) | `cd worker && npm test` (Node test runner, no deps) · `npm run check:worker` (tests + fixture that Rust verifies) · deploy: `npx wrangler deploy` (docs/licensing.md) |
| Release files (Windows) | `pwsh scripts/release/package.ps1 [-SkipBuild] [-OutDir dist-release]` – collector, NSIS setup, portable app, SHA256SUMS, winget manifests, third-party notices; `scripts/release/installer-test.ps1 -Setup <exe>`; `node scripts/release/check-release.mjs [--warn]` (public key set, no placeholders) |

The toolchain is pinned in `rust-toolchain.toml` (same as Stepwright). Building the app crate on Linux needs
WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev`); the product itself targets Windows x64 only.

CI (`.github/workflows/ci.yml`): checks (ubuntu) → Linux job (fmt, clippy, tests, Cargo-graph offline check,
scan and merge performance tests, strace read-only trace and network test of the collector, reports in eight languages
from the corpus scan and sample machines, read back with pdftotext and the Python standard library) and Windows jobs
on windows-2025 and windows-2022 (clippy, tests, release build with static CRT, size and import check, scan and merge
performance tests, ETW read-only trace, network block test, non-admin run, system test with artefacts made by Windows,
full scan of the runner without internal errors and without findings for Windows servicing data or ESE databases,
reports of that full scan; app build with NSIS installer, smoke test – which renders both reports in every language –,
network tests, release files and a silent install/start/uninstall test on 2025). Logs, results, reports and the
release files are uploaded as artifacts. Release (`.github/workflows/release.yml`, tag `v<version>` or manual): release
check (refuses placeholders and a missing public key), `check:all`, `package.ps1` with optional code signing
(secrets `VBS_SIGN_CERT_BASE64`/`VBS_SIGN_CERT_PASSWORD`), smoke test of the packaged app, draft GitHub release.

## Layout

```text
product.json            single source of truth: name, identifier, URLs, prices, free machine limit, result file type
rules/catalog.json      rule catalog: ID, finding kind, breaks/review, sources with "checked" date (texts in i18n/)
i18n/<lang>.json        translations for UI, console, reports, rule texts; languages.json = review status; README
crates/
  vbs-config/           product.json embedded + validated (vbs_config::product()) – from Stepwright's sw-config
  vbs-i18n/             catalogs, t()/t_args()/t_count(), CLDR plurals, review status – from Stepwright's sw-i18n
  vbs-core/             result model + .vbscout container (read/write/validate/migrate), rule catalog, module
                        interface (Module, Report, CandidateFile), read-only system views, secret masking
  vbs-collector/        collector CLI: cli, engine, walk, read_only (only way to open files), output (ONLY writer),
                        platform/{windows/,other.rs} (all OS calls: registry, event logs, WMI via COM),
                        analysis/ (format readers without I/O: command lines, markup, .lnk, .msi, .vbe, scripts.ini,
                        event XML, registry hive files, VBA projects, Office documents, Access databases,
                        Windows servicing data),
                        modules/ (one file per finding type)
  vbs-evaluation/       evaluation logic without UI: import (machine limit), assessment (newest scan per machine,
                        de-duplication, Windows components, entry→script links, risk), effort (rules of thumb),
                        coverage, edition (limits enforced here), report/ (pdf with a small layout engine,
                        xlsx, text formatting), sample (synthetic results); assets/fonts (Liberation Sans, OFL)
  vbs-license/          license keys: `VBS1-<payload>.<signature>` (Ed25519, public key from product.json), verified
                        offline only; feature `issue` (signing) for the vendor tool and tests only
src-tauri/              Tauri shell (crate vbs-app): commands, smoke test, settings, tauri.conf.json, capabilities, icons
src/                    Svelte 5 + TypeScript frontend (view only; talks to Rust via IPC): App.svelte (shell, import,
                        tabs, license page), components/ (overview, findings with detail, machines, reports, license)
tests/corpus/           positive/ and negative/ collections incl. system/ and office/ fixtures + expected.json (DoD #4),
                        make-binaries.py, office_fixtures.py + tools/OfficeFixtures.java, THIRD-PARTY.md (real Office files)
scripts/                sync-config, check-i18n/offline/readonly/size (Node, no deps); nettest/, readonly/, systemtest/ (dynamic);
                        reports/check.py (reads the reports with pdftotext and the Python standard library)
                        release/ (names, package.ps1, sign.ps1, installer-test.ps1, winget.mjs, third-party.mjs, check-release)
docs/                   result-format.md, result.schema.json, examples/, research-notes.md (sources with dates),
                        licensing.md (key format, key service, Paddle setup, operations)
tools/license-keys/     vendor CLI: create the signing key, issue/inspect keys (never shipped)
worker/                 key service (Cloudflare Worker, plain JS): Paddle webhook → signed key in KV, /license/<txn>;
                        test/ with Node's test runner and the fixture the Rust verifier checks
packaging/winget/       winget manifest templates (app: NSIS per user, collector: portable)
LICENSE, PRIVACY.md     FSL-1.1-ALv2 (licensor to be filled in), privacy notice of the programs and the purchase
```

Phase 6 adds `website/`.

## Naming

- Product name, identifier, website, prices, the free machine limit and the result file extension/media type exist
  **only** in `product.json`. Rust reads `vbs_config::product()`, the frontend `src/lib/product.ts`, translations use
  `{product}`. `npm run sync:config` writes derived values into `tauri.conf.json`; CI fails on drift. "VBScout" is not
  trademark-cleared – keep it swappable.
- `vbs-` is an internal code prefix (crates, binaries `vbs-collector`/`vbs-app`, env vars `VBS_…`), independent of the
  product name. Packaging (phase 5) gives the release files product names.
- Result files: extension and media type from `product.json` (`.vbscout`, `application/vnd.vbscout.result+zip`).
  Frozen after the first public release; a rename adds the old media type to `legacyMimeTypes`.
- Rule IDs `VBS-nnn`, stable forever: 1xx script files, 2xx scripts/shortcuts starting VBScript, 3xx tasks, autostart,
  services, WMI, logon scripts, 4xx MSI, 5xx event logs, 6xx Office macros, 9xx security; `x00` = "could not be
  checked" of that range (the finding states the actual kind, e.g. `VBS-200` for a shortcut). Within a range each kind
  has a decade: `x1` = runs VBScript (`breaks`), `x2` = starts a script of unknown language (`review`), e.g. 301/302
  tasks, 311/312 autostart, 321/322 services, 331/332 WMI, 341/342 logon scripts. Office macros (one kind) have a
  decade per mechanism: 601 VBScript regular expressions, 611/612 script engines (Script Control, `execScript`),
  621/622 starting scripts, 632 Windows Script Host objects (`review` only). Texts: `rule.vbs101.title` /
  `.rationale`.
- Coverage source IDs: `files.localDrives`, `files.paths`, `files.networkPaths`, `<area>.<name>` for system sources
  (`tasks.scheduled`, `autostart.entries`, `services.configuration`, `wmi.subscriptions`, `policies.scripts`,
  `installer.packages`, `eventLog.vbscriptDeprecation`, `eventLog.sysmon`; list in `docs/result-format.md`).
- i18n keys: flat, dot-separated camelCase; plurals `_one/_few/_many/_other`; keys outside `t()` calls wrapped in
  `key("…")` (the report code's `text.t/args/count("…")` is recognised too); dynamic keys only with the prefixes
  `kind.`, `activation.`, `reason.`, `limitation.`, `classification.`, `findingStatus.`, `rule.`, `hint.`, `risk.`,
  `origin.`, `source.`, `sourceStatus.`, `sourceReason.`, `setAside.`, `locationKind.`, `productType.`, `coverage.`,
  `edition.` (completeness tested in `vbs-core` and `vbs-evaluation`). `format.number` (1234.5 written in the
  language) and `format.date` (`{year}`, `{month}`, `{day}`) set number and date formats of the reports.
- Rule catalog entries carry `effort` (`minHours`, `maxHours`, `basis`: `fixed`, `scriptSize` or `entry`) and `hint`
  (the text `hint.<id>`, shared by rules that are migrated the same way).
- Commits: one per phase, `Phase X: <Name>`.

## Design decisions

1. **Collector and evaluation are separate programs.** The collector is a small synchronous Rust CLI without GUI,
   async runtime or network stack (checked on the resolved dependency graph). The evaluation is a Tauri 2 app in
   Stepwright's style; its frontend only displays, all rules (edition limits, import checks) live in Rust.
2. **One module interface for all finding types** (`vbs_core::module`): a module declares file extensions and/or a
   system source, inspects `CandidateFile`s from the walk and/or examines the read-only `SystemView` (registry, known
   files, event logs, WMI), and reports through `Report`. The report takes kind and classification from the rule
   catalog, masks and shortens evidence on arrival and offers `not_checkable` – a module never touches the OS and
   never writes. Platform code implements the views (`platform/windows/`), tests use in-memory views, so every
   module is testable on any machine. A panic in a module becomes a "not checkable (internalError)" finding.
3. **Read-only by construction and by proof.** Files are opened only in `read_only.rs` (read access, full sharing),
   the result only in `output.rs` (`create_new`, never overwrite; `--out` names an existing folder or a file with the
   result extension; folders are never created; without `--out` never inside the Windows directory, e.g. a GPO run
   as SYSTEM in `System32`). Registry keys only with `KEY_READ` (no WOW64 view flags – they tag handles, which a
   kernel trace lists as a "set"; the 32-bit view is read at `SOFTWARE\WOW6432Node`), no hive loading (hives of users
   who are not logged on are read as files), logs only queried once, no subscriptions, no process start. Proof in
   three layers: static (`check-readonly.mjs` with self-test), dynamic snapshot test (`tests/read_only.rs`), kernel
   traces with positive control (`scripts/readonly/`: strace, ETW Kernel-File/Kernel-Registry). Windows may update
   last-access times when files are read (volume policy, like any reader) – documented, not avoidable without write
   access.
4. **The walk never triggers side effects**: no links/junctions/mount points followed, no online-only cloud files
   (`RECALL_ON_DATA_ACCESS`/`OFFLINE`) opened – reported as not checkable – and no cloud directories
   (`RECALL_ON_OPEN`) listed, because both would download data. Non-regular files (FIFOs, devices) are never opened.
   SUBST drives are skipped (duplicates). Network paths only with `--include-unc`, read with the running account.
5. **Detection** (`crates/vbs-collector/src/analysis/command.rs`): a command runs VBScript when it starts the Script
   Host with a `.vbs`/`.vbe` file or `//E:VBScript`, starts a `.vbs`/`.vbe` directly or contains `vbscript:` code;
   `.wsf`/`.hta`/`.wsc` and the Script Host with other files are "unknown language" (`review`); JScript is never a
   finding. System modules look one level into batch/PowerShell/KiXtart files an entry starts, through the file
   view – which refuses network paths, mapped network drives, device paths, links and cloud placeholders (a script on
   the network becomes `notCheckable`/`networkLocation`). User hives that are not loaded are never loaded: the
   profile's `NTUSER.DAT` is read as a file (`analysis/regf.rs`); if that fails, the autostart source is `partial`
   (`userHivesNotRead`). Installer packages are read as compound files
   (`cfb` crate), scripts decoded from UTF-8/UTF-16/ANSI, `.vbe` decoded (reversible Script Encoder substitution).
   Windows servicing data under a candidate's name – component store differentials (`WinSxS\…\f\`, `r\`, `n\`;
   MSDelta `PA30`/`PA31`) and compressed payloads (`DCN`/`DCS`/`DCD`/`DCM` v1) – is recognised by content and is no
   finding (`analysis/servicing.rs`): the real file is checked where Windows puts it. Files are never skipped by path.
   **Office macros** (`analysis/office.rs`, `ovba.rs`, `jet.rs`, `vba.rs`): documents are recognised by content
   (compound file, Open XML package, Access database; RTF/HTML/CSV exports, lock files and Windows' ESE databases named
   `.mdb` – User Access Logging – are no finding), every VBA
   project is found – including embedded objects and Access 97–2016 system tables – and its source decompressed
   ([MS-OVBA]). Reported: `VBScript.RegExp` and references to `vbscript.dll` (601), the Script Control and `execScript`
   by language (611/612), commands in string literals (joined across `&`, analysed like other commands; a bare `.vbs` path only where
   the line starts something) (621/622), WSH objects (632); the Scripting Runtime and JScript are not findings. A
   project "locked for viewing" is read (its source is not encrypted); a password to open, rights management, compiled
   code without source and unsupported formats (Access 97, Excel 5.0/95 module sheets, `ActiveMime`) are
   `notCheckable`.
6. **Result file** (`docs/result-format.md`): ZIP with `mimetype` + `result.json`, versioned schema with JSON Schema,
   open enumerations (unknown values kept, unknown classification = review), migrations for breaking changes,
   limits against malicious files, deterministic finding order. Coverage is recorded per source with limitations
   (not elevated, files only, restricted paths, failed sources) – never a completeness promise.
7. **Rule catalog as data** (`rules/catalog.json`): every rule has `breaks`/`review`, at least one source with URL and
   "checked" date; uncertain cases are always `review`; "could not be checked" rules are always `review`. Texts are
   translated in `i18n/`. Timeline statements are always "expected" with source and date.
8. **Privacy**: evidence only as short excerpts of affected lines; secrets masked twice (report + writer) and never
   stored; machine ID pseudonymous; no user names beyond necessary paths.
9. **Editions**: collector always free. Evaluation free = finding list, ≤ `editions.free.maxMachines` machines, no
   PDF, no migration hints, no effort; organization license (one-time, org name in reports, unlimited machines);
   MSP license (yearly, expiry against the system clock, unlimited customer environments, own logo and company name).
   Keys are verified offline only (Ed25519, `docs/licensing.md`; the key service issues them for Paddle
   transactions, MSP keys valid through the paid period + grace days); the entered key is stored next to the
   settings and checked again at every start. Enforced in Rust (`vbs_evaluation::edition`): at import (files
   of further machines are not loaded), in the views sent to the UI (no hint keys, no effort values) and in the report
   writers (PDF refused; Excel without hint and effort columns). The customer/environment name is a setting in every
   edition; the logo is stored as a setting but only printed with an MSP license.
10. **i18n** (from Stepwright): one set of catalogs for Rust and UI, 8 languages; en and de reviewed, the others marked
   with a correction hint; completeness, placeholders, plurals and used keys checked in CI.
11. **Offline guarantee** (from Stepwright): static (`check-offline.mjs`: IPC-only CSP, capabilities, deny lists,
    sources, WebView2 switches, collector graph without tokio/sockets; `clippy.toml` bans `std::net` and
    `Command::new`) and dynamic (`scripts/nettest/`: blocks and logs every connection attempt with a positive
    control, for the collector and the app).
12. **Evaluation** (`crates/vbs-evaluation`): the newest scan per machine (host name + DNS domain + machine ID) is
    evaluated; older scans and machines beyond the edition's limit are listed as set aside. Findings are merged into
    items: the same network file (normalised path), the same entry or the same file content at the same path on
    several machines, Windows' own files by name. **Windows components** = files in the component store, servicing
    and update caches, User Access Logging databases, Windows folders of container image layers, or files with the same
    content as a component-store file – never recognised by file name alone; listed separately (risk "info", no
    effort). Entries (tasks, autostart, services, WMI, policies, shortcuts, calling scripts, macros, log records) link
    to the scripts they start (exact path, or a file name that is unique on that machine); a script inherits the
    activation of what starts it. **Risk**: `breaks` + automatic/logged = high; `breaks` + macro/manual/installer or
    `review` + automatic/logged = medium; everything else low. **Effort**: rule-of-thumb ranges from the catalog –
    `scriptSize` × 1/2/4/8 (≤ 8/32/128 KiB/larger), `entry` plus a typical script once per script the scan did not
    find – counted once per item (central rollout assumed) and once per identical content; always labelled as a rule
    of thumb. **Reports**: PDF with krilla and the embedded Liberation Sans (OFL, subset; text measured with rustybuzz),
    Excel with rust_xlsxwriter, both in memory; every text from the catalogs; coverage shown without a completeness
    promise.

## Forbidden approaches

- **Any change to a scanned system**: writing, creating, renaming or deleting files or folders other than the one
  result file;
  overwriting an existing file (even an old result); registry writes or loading user hives; clearing, exporting or
  writing event logs; changing tasks, services, WMI or policies; adjusting privileges; starting programs; disabling
  VBScript. Only `crates/vbs-collector/src/output.rs` may write; only `read_only.rs` opens files.
- **Any network connection** of collector or evaluation – no telemetry, update checks, online activation, crash
  reports, remote fonts/scripts, Tauri http/updater/websocket/upload/shell/opener plugins, HTTP/TLS crates, `std::net`,
  `fetch`, `window.open`. No network scan, remote execution, handling of admin credentials, Active Directory queries.
  Network paths are read only when given explicitly with `--include-unc`.
- **AI/LLM at runtime** (no local or remote models).
- **Automatic conversion** of VBScript to PowerShell or anything else – the product finds and explains, it never
  rewrites code.
- **Silent skipping**: every candidate that cannot be analysed (protected, encrypted, locked, corrupt, too large,
  cloud placeholder, module failure) becomes a "not checkable" finding; every unreadable source shows in coverage.
- Storing secrets in results, reports or logs; evidence beyond short excerpts of affected lines.
- Effort figures without the rule-of-thumb label; dropping items from the evaluation (Windows components, older
  scans and machines beyond the limit are listed separately, never hidden).
- Agents, services, scheduled re-runs or continuous monitoring; cloud upload; tracking on the website.
- Calling the project "Open Source" – it is **source-available** (FSL-1.1-ALv2).
- Hard-coding product name, prices, URLs, limits or the result file type outside `product.json`.
- Classifying a rule as `breaks` without a source; timeline claims without source and date or without "expected".
- Gating licensed features only in the UI.
- Platform APIs outside `crates/vbs-collector/src/platform/`; `unsafe` outside `platform/windows/`; msi.dll (MSI
  packages are parsed as files); Office automation, OLE or the Access database engine (documents and databases are
  parsed as files).
- Committing secrets (Paddle keys, license signing key – Cloudflare Worker secret only; `product.json` holds only the
  public key). Signing code (`vbs-license` feature `issue`) in the app.
- An installer that downloads anything (WebView2 bootstrapper) – `webviewInstallMode` stays `skip` (checked).
