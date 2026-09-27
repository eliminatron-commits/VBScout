# VBScout

**Find the VBScript dependencies in a Windows environment before VBScript is switched off.** Microsoft has
deprecated VBScript; it is expected to be disabled by default in a later phase (around 2027 according to Microsoft's
[published timeline](https://techcommunity.microsoft.com/blog/windows-itpro-blog/vbscript-deprecation-timelines-and-next-steps/4148301),
as of 2026-09-26) and to be removed afterwards. VBScout shows what would break – scripts, scheduled tasks, logon
scripts, services, WMI subscriptions, installer actions and Office macros – and helps plan the migration.

## Downloads

Every release (GitHub → Releases) contains, for Windows x64:

| File | What it is |
|---|---|
| `VBScout-Collector-<version>-x64.exe` | the **collector** – one portable file, no installation, free |
| `VBScout-<version>-x64-setup.exe` | the **evaluation app**, installer (per user, no administrator rights needed) |
| `VBScout-<version>-x64-portable.exe` | the evaluation app without installation |
| `VBScout-<version>-SHA256SUMS.txt` | checksums – verify with `Get-FileHash <file>` |
| `VBScout-<version>-THIRD-PARTY-NOTICES.md` | licenses of the components inside the programs |
| `VBScout-<version>-winget-manifests.zip` | manifests for the Windows Package Manager |

Requirements: the collector runs on Windows 10/11 and Windows Server 2016–2025 (x64). The evaluation app needs
Windows 10 (1809) or later with the Microsoft Edge WebView2 runtime, which Windows 10/11 already include – the
installer never downloads anything.

## How it works

1. **Collector** – run it on every computer you want to check: by hand, or through GPO, Intune or your RMM tool
   (as administrator or SYSTEM for a complete scan). It only reads and writes a single result file (`.vbscout`):

   ```bat
   VBScout-Collector-<version>-x64.exe --out \\fileserver\vbscout-results\ --quiet
   ```

   Useful options: `--path <folder>` (scan only these folders), `--include-unc <\\server\share>` (also scan a network
   share – never done otherwise), `--files-only`, `--lang de`. `--help` lists all. Without administrator rights the
   scan is limited and the result says so. Exit code 0 = result written.
2. **Evaluation** – open any number of result files (or a whole folder) in the app. It keeps the newest scan per
   machine, merges identical findings across machines, rates the risk (started automatically > used according to the
   event logs > Office macro > dormant file) and shows Windows' own components separately.
3. **Reports** – a technical **Excel list** and, with a license, a **management PDF** with priorities, migration
   hints and effort estimates (clearly labelled as rules of thumb), in eight languages.

## Editions

| | Free | Organization license (one-time) | MSP license (yearly) |
|---|---|---|---|
| Collector | ✓ | ✓ | ✓ |
| Finding list, Excel list | a limited number of machines | unlimited machines of one organization | unlimited customer environments |
| Management PDF, migration hints, effort estimates | – | ✓ | ✓ |
| Name in the reports | – | organization | your company name and logo |

Prices are on the website. License keys are entered under **License** in the app and verified on the computer –
see [`docs/licensing.md`](docs/licensing.md).

## Safety first

- **Read-only.** The collector never changes anything on the scanned computer and writes exactly one file – its
  result. It never overwrites a file. Automated tests in CI prove this on Linux (system-call trace) and Windows
  (kernel trace of every file and registry change), each with a positive control.
- **Fully offline.** Neither the collector nor the evaluation opens a network connection – no telemetry, no update
  check, no online activation. Network shares are only read when you name them explicitly (`--include-unc`).
- **No silent gaps.** Everything that cannot be checked (password-protected macros, locked or online-only files, …)
  is reported as "not checkable", and the result records what the scan could and could not see.
- **Secrets stay secret.** Evidence contains only the affected lines, with passwords and connection strings masked.
- **Signed releases** (as soon as the code-signing certificate is in place) and published checksums. Scanning many
  files and registry keys can look unusual to security products; the collector's behaviour is documented here and in
  [`docs/result-format.md`](docs/result-format.md).

Privacy: [`PRIVACY.md`](PRIVACY.md).

## Building from source

Requirements: Rust (the version pinned in `rust-toolchain.toml`), Node.js 22.12+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) (WebView2 on Windows).

```sh
npm ci
npm run check:all                          # configuration, translations, offline/read-only policy, key service, types
cargo test --workspace
cargo build --release -p vbs-collector     # the collector: target/release/vbs-collector(.exe)
npx tauri build --no-bundle                # the evaluation app: target/release/vbs-app(.exe)
```

Release files for Windows: `scripts/release/package.ps1` (the same script runs in CI and in the release workflow,
`.github/workflows/release.yml`, started by a `v<version>` tag).

## License

VBScout is **source-available** under the [Functional Source License 1.1, ALv2 Future License](LICENSE)
(FSL-1.1-ALv2): you may use, change and share it for any purpose except a competing product; each version becomes
available under the Apache License 2.0 two years after its release. It is not open-source software. The collector is
free for everyone; the full evaluation requires an organization or MSP license.

Translations are welcome – see [`i18n/README.md`](i18n/README.md).
