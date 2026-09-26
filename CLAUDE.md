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
  hints and rule-of-thumb effort, and produces the management PDF and the technical Excel list (phase 4).

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
| Program size | `node scripts/check-size.mjs <exe> <maxMB>` (collector: 10) |
| Propagate product.json | `npm run sync:config` |
| Cross-check Windows code on Linux | `cargo clippy --target x86_64-pc-windows-msvc -p vbs-collector --all-targets -- -D warnings` |

The toolchain is pinned in `rust-toolchain.toml` (same as Stepwright). Building the app crate on Linux needs
WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev`); the product itself targets Windows x64 only.

CI (`.github/workflows/ci.yml`): checks (ubuntu) → Linux job (fmt, clippy, tests, Cargo-graph offline check, strace
read-only trace and network test of the collector) and Windows job (clippy, tests, release builds with static CRT,
size check, full scan of the runner, ETW read-only trace, network block tests of collector and app, app smoke test).
Logs are uploaded as artifacts.

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
                        platform/{windows/,other.rs} (all OS calls), modules/ (finding types, phases 2–3)
  vbs-evaluation/       evaluation logic without UI: import (phase 1); merge, de-dup, risk, effort, reports (phase 4)
  vbs-license/          license interface (offline only; Ed25519 format + org/MSP keys in phase 5)
src-tauri/              Tauri shell (crate vbs-app): commands, smoke test, settings, tauri.conf.json, capabilities, icons
src/                    Svelte 5 + TypeScript frontend (view only; talks to Rust via IPC)
tests/corpus/           positive/ and negative/ collections + expected.json (Definition of Done #4)
scripts/                sync-config, check-i18n/offline/readonly/size (Node, no deps); nettest/, readonly/ (dynamic)
docs/                   result-format.md, result.schema.json, examples/, research-notes.md (sources with dates)
```

Later phases add: `worker/` + `tools/` license keys (5), `packaging/` winget + installer (5), `website/` (6).

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
  checked" of that range. Texts: `rule.vbs101.title` / `.rationale`.
- Coverage source IDs: `files.localDrives`, `files.paths`, `files.networkPaths`, `<area>.<name>` for system sources.
- i18n keys: flat, dot-separated camelCase; plurals `_one/_few/_many/_other`; keys outside `t()` calls wrapped in
  `key("…")`; dynamic keys only with the prefixes `kind.`, `activation.`, `reason.`, `limitation.`, `classification.`,
  `findingStatus.`, `rule.` (their completeness is tested in `vbs-core`).
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
   as SYSTEM in `System32`). Registry keys only with `KEY_READ`, no hive
   loading, logs only queried once, no subscriptions, no process start. Proof in three layers: static
   (`check-readonly.mjs` with self-test), dynamic snapshot test (`tests/read_only.rs`), kernel traces with positive
   control (`scripts/readonly/`: strace, ETW Kernel-File/Kernel-Registry). Windows may update last-access times when
   files are read (volume policy, like any reader) – documented, not avoidable without write access.
4. **The walk never triggers side effects**: no links/junctions/mount points followed, no online-only cloud files
   (`RECALL_ON_DATA_ACCESS`/`OFFLINE`) opened – reported as not checkable – and no cloud directories
   (`RECALL_ON_OPEN`) listed, because both would download data. Non-regular files (FIFOs, devices) are never opened.
   SUBST drives are skipped (duplicates). Network paths only with `--include-unc`, read with the running account.
5. **Result file** (`docs/result-format.md`): ZIP with `mimetype` + `result.json`, versioned schema with JSON Schema,
   open enumerations (unknown values kept, unknown classification = review), migrations for breaking changes,
   limits against malicious files, deterministic finding order. Coverage is recorded per source with limitations
   (not elevated, files only, restricted paths, failed sources) – never a completeness promise.
6. **Rule catalog as data** (`rules/catalog.json`): every rule has `breaks`/`review`, at least one source with URL and
   "checked" date; uncertain cases are always `review`; "could not be checked" rules are always `review`. Texts are
   translated in `i18n/`. Timeline statements are always "expected" with source and date.
7. **Privacy**: evidence only as short excerpts of affected lines; secrets masked twice (report + writer) and never
   stored; machine ID pseudonymous; no user names beyond necessary paths.
8. **Editions**: collector always free. Evaluation free = finding list, ≤ `editions.free.maxMachines` machines, no
   PDF, no migration hints, no effort; organization license (one-time, org name in reports, unlimited machines);
   MSP license (yearly, expiry against the system clock, unlimited customer environments, own logo and company name).
   Keys are verified offline only (Ed25519, phase 5).
9. **i18n** (from Stepwright): one set of catalogs for Rust and UI, 8 languages; en and de reviewed, the others marked
   with a correction hint; completeness, placeholders, plurals and used keys checked in CI.
10. **Offline guarantee** (from Stepwright): static (`check-offline.mjs`: IPC-only CSP, capabilities, deny lists,
    sources, WebView2 switches, collector graph without tokio/sockets; `clippy.toml` bans `std::net` and
    `Command::new`) and dynamic (`scripts/nettest/`: blocks and logs every connection attempt with a positive
    control, for the collector and the app).

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
- Agents, services, scheduled re-runs or continuous monitoring; cloud upload; tracking on the website.
- Calling the project "Open Source" – it is **source-available** (FSL-1.1-ALv2).
- Hard-coding product name, prices, URLs, limits or the result file type outside `product.json`.
- Classifying a rule as `breaks` without a source; timeline claims without source and date or without "expected".
- Gating licensed features only in the UI.
- Platform APIs outside `crates/vbs-collector/src/platform/`; `unsafe` outside `platform/windows/`; msi.dll (MSI
  packages are parsed as files).
- Committing secrets (Paddle keys, license signing key – Cloudflare Worker secret only).
