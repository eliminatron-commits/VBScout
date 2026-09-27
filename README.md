# VBScout

**Find the VBScript dependencies in a Windows environment before VBScript is switched off.** Microsoft has
deprecated VBScript; it is expected to be disabled by default in a later phase (around 2027 according to Microsoft's
[published timeline](https://techcommunity.microsoft.com/blog/windows-itpro-blog/vbscript-deprecation-timelines-and-next-steps/4148301),
as of 2026-09-26) and to be removed afterwards. VBScout shows what would break – scripts, scheduled tasks, logon
scripts, services, installer actions and Office macros – and helps plan the migration.

> **Status:** in development. The collector finds all finding types (system level and Office macros); the
> evaluation merges result files and creates the management PDF and the technical Excel list. Licensing, packages
> and the website follow.

## How it works

1. **Collector** – one portable program file, no installation. Run it on every computer (manually, or through GPO,
   Intune or an RMM tool). It only reads and writes a single result file (`.vbscout`).
2. **Evaluation** – a desktop app that merges any number of result files, ranks the risks and creates the reports.

## Safety first

- **Read-only.** The collector never changes anything on the scanned computer and writes exactly one file – its
  result. It never overwrites a file. Automated tests in CI prove this on Linux (system-call trace) and Windows
  (kernel trace of every file and registry change), each with a positive control.
- **Fully offline.** Neither the collector nor the evaluation opens a network connection – no telemetry, no update
  check, no online activation. Network shares are only read when you name them explicitly (`--include-unc`).
- **No silent gaps.** Everything that cannot be checked (password-protected macros, locked or online-only files, …)
  is reported as "not checkable", and the result records what the scan could and could not see.
- **Secrets stay secret.** Evidence contains only the affected lines, with passwords and connection strings masked.

## Building from source

Requirements: Rust (the version pinned in `rust-toolchain.toml`), Node.js 22.12+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) (WebView2 on Windows).

```sh
npm ci
npm run check:all                          # configuration, translations, offline and read-only policy, types
cargo test --workspace
cargo build --release -p vbs-collector     # the collector: target/release/vbs-collector(.exe)
npx tauri build --no-bundle                # the evaluation app: target/release/vbs-app(.exe)
```

## License

VBScout is **source-available** under the Functional Source License 1.1 with Apache 2.0 future license
(FSL-1.1-ALv2). It is not open-source software; the license text will be added to `LICENSE`. The collector is free
for everyone; the full evaluation requires an organization or MSP license.

Translations are welcome – see [`i18n/README.md`](i18n/README.md).
