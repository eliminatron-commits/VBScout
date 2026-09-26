# Research notes (sources for the rule catalog and marketing)

Collected facts with their sources and the date they were checked. Everything that ends up in the rule catalog
(`rules/catalog.json`), reports or marketing must cite a source from here (or a newer one) with its date. Timeline
statements are always "expected"/"approximately" – Microsoft may move them.

> Access note (2026-09-26): the build environment's egress policy blocks learn.microsoft.com,
> techcommunity.microsoft.com and devblogs.microsoft.com. The statements below were checked through search-engine
> excerpts of these pages on 2026-09-26. Re-read the primary pages before a public release (release checklist).

## Microsoft timeline

| Fact | Source | Checked |
|---|---|---|
| VBScript is deprecated; in future Windows releases it is available as a feature on demand (FOD) before its removal. Announced October 2023. | Microsoft Learn – [Deprecated features for Windows client](https://learn.microsoft.com/en-us/windows/whats-new/deprecated-features) | 2026-09-26 |
| Phase 1: VBScript FODs pre-installed and enabled by default in Windows 11, version 24H2. Phase 2: FODs disabled by default, "approximately 2026 or 2027" / "around 2027". Phase 3: removal, date TBD. | Windows IT Pro Blog – [VBScript deprecation: Timelines and next steps](https://techcommunity.microsoft.com/blog/windows-itpro-blog/vbscript-deprecation-timelines-and-next-steps/4148301) (Naveen Shankar, 2024-05-22) | 2026-09-26 |
| Status updates for Windows Insiders (June and September 2026) still describe phase 2 as approximately 2026/2027. | Microsoft Community Hub – [State of vbscript deprecation – September 2026](https://techcommunity.microsoft.com/discussions/windowsserverinsiders/blog-windows-insiders---state-of-vbscript-deprecation---september-2026/4525768) | 2026-09-26 |

## Detection (input for phase 2)

| Fact | Source | Checked |
|---|---|---|
| Event ID 4096, source `VBScriptDeprecationAlert`, Application log: logged when VBScript runs; message names the process, the process tree and a call stack. | Microsoft Community Hub (September 2026 post above); Microsoft Q&A threads | 2026-09-26 |
| Sysmon Event ID 7 (image load of `vbscript.dll`) and Event ID 1 (process creation) identify processes using VBScript; also: scan for .vbs files, check custom MSI packages, AppLocker script/MSI audit events. | Windows IT Pro Blog – [VBScript deprecation: Detection strategies for Windows](https://techcommunity.microsoft.com/blog/windows-itpro-blog/vbscript-deprecation-detection-strategies-for-windows/4414325) (message center MC1075649, 2025) | 2026-09-26 |
| `slmgr.vbs` automation: Microsoft published PowerShell guidance to keep Windows activation automation working. | Windows IT Pro Blog – [Keep Windows activation automation working with PowerShell](https://techcommunity.microsoft.com/blog/windows-itpro-blog/keep-windows-activation-automation-working-with-powershell/4540459) | 2026-09-26 |
| Event 4096 data: third-party reports quote the fields ProcessName, ProcessTree (`cscript.exe;cmd.exe;userinit.exe;winlogon.exe`) and CallStack. Whether they are named `<Data Name="…">` or positional is not documented – the collector reads both and the labelled message text; verify on a system that logs the event (the Windows system test in CI checks it where available). | Search excerpts of forum posts (Windows 11 Forum, Univention, Checkmk) | 2026-09-26 |
| Custom action types: VBScript = base type 6 (`msidbCustomActionTypeVBScript`); source 0x00 Binary table (type 6), 0x10 installed file (22), 0x20 text in `Target` (38), 0x30 property (54); 7 = nested installation of a sub-storage. The Microsoft detection blog names "6, 38 and 50" – 50 is an EXE from a property, so the collector uses the documented type bits instead. | Microsoft Learn – [Summary List of All Custom Action Types](https://learn.microsoft.com/en-us/windows/win32/msi/summary-list-of-all-custom-action-types), [Custom Action Type 6](https://learn.microsoft.com/en-us/windows/win32/msi/custom-action-type-6), [Custom Action Type 22](https://learn.microsoft.com/en-us/windows/win32/msi/custom-action-type-22), [Custom Action Type 54](https://learn.microsoft.com/en-us/windows/win32/msi/custom-action-type-54) | 2026-09-26 |
| Installer string pool: a string of 64 KiB or more takes two entries – (0, reference count), then the length as (low word, high word). Wine's reader and writer agree on this layout; SQL Server 2016 packages on the CI runners (written by Microsoft's tools) could not be read with the reverse layout, which msitools writes. | Wine – [`dlls/msi/string.c`](https://github.com/wine-mirror/wine/blob/master/dlls/msi/string.c) (`msi_load_string_table`, `msi_save_string_table`); CI run of this repository | 2026-09-26 |
| `msidbCustomActionTypeContinue` (0x40): errors of the action are ignored – the collector rates such actions "review". | Microsoft Learn – [Custom Action Return Processing Options](https://learn.microsoft.com/en-us/windows/win32/msi/custom-action-return-processing-options) | 2026-09-26 |
| `ActiveScriptEventConsumer`: `ScriptingEngine` (e.g. "VBScript"), `ScriptText` or `ScriptFileName`; registered in `root\subscription`. | Microsoft Learn – [ActiveScriptEventConsumer class](https://learn.microsoft.com/en-us/windows/win32/wmisdk/activescripteventconsumer) | 2026-09-26 |
| `scripts.ini`: sections Logon/Logoff (user) and Startup/Shutdown (computer) with `<n>CmdLine` and `<n>Parameters`. | Microsoft Learn – [[MS-GPSCR]: Scripts.ini Syntax](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-gpscr/ff1fd13e-1e18-4160-9b50-0263e108e5e1) | 2026-09-26 |
| Task definitions: `Exec` actions with `Command`, `Arguments`, `WorkingDirectory`. | Microsoft Learn – [Exec (actionGroup) Element](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-exec-actiongroup-element) | 2026-09-26 |
| Run/RunOnce keys (machine and user). | Microsoft Learn – [Run and RunOnce Registry Keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys) | 2026-09-26 |
| WSH: `.wsf` files choose the language per `<script>` element; the Script Host option `//E:engine` selects the engine; `.vbe` is VBScript encoded by the Script Encoder (`VBScript.Encode`), a reversible substitution. HTAs run in mshta.exe. | Microsoft Learn – [&lt;script&gt; Element (Windows Script Host)](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/dzfdccyf(v=vs.84)), [wscript](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/wscript), [Script Encoder Syntax](https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/windows-scripting/xw61tsx7(v=vs.84)), [Introduction to HTML Applications](https://learn.microsoft.com/en-us/previous-versions/ms536496(v=vs.85)) | 2026-09-26 |
| Sysmon events 1 (process creation, with command line) and 7 (image loaded) in `Microsoft-Windows-Sysmon/Operational`. | Microsoft Learn – [Sysmon](https://learn.microsoft.com/en-us/sysinternals/downloads/sysmon) | 2026-09-26 |
| Hard-coded credentials as a security weakness. | MITRE – [CWE-798: Use of Hard-coded Credentials](https://cwe.mitre.org/data/definitions/798.html) | 2026-09-26 |

## Supported Windows versions (Definition of Done #3)

| Fact | Source | Checked |
|---|---|---|
| The collector's build target `x86_64-pc-windows-msvc` requires Windows 10 or later for client installs and Windows Server 2016 or later for server installs – the range the collector promises (Windows 10/11, Server 2016–2025). | Rust compiler book – [Platform support](https://doc.rust-lang.org/rustc/platform-support.html) ("64-bit MSVC (Windows 10+, Windows Server 2016+)") and [`*-pc-windows-msvc`](https://doc.rust-lang.org/rustc/platform-support/windows-msvc.html) ("Windows 10 or higher is required for client installs, Windows Server 2016 or higher is required for server installs"), read from the rust-lang/rust repository | 2026-09-26 |
| Checked on every build: the collector imports only reviewed Windows system libraries – no C runtime (linked statically), no network or directory library (`scripts/check-imports.mjs`; the full list of imported functions is kept as a CI artifact). CI runs the collector on Windows Server 2025 and Server 2022; Windows 10/11 clients and Server 2016/2019 are not available as CI machines – run the collector on them before a release (release checklist). | this repository (CI) | 2026-09-26 |

## Office / VBA (input for phase 3)

| Fact | Source | Checked |
|---|---|---|
| VBA projects are affected when they use VBScript, e.g. `VBScript.RegExp` (CreateObject or reference to "Microsoft VBScript Regular Expressions"), call .vbs files, or run VBScript through ScriptControl. | Microsoft 365 Developer Blog – [Prepare your VBA projects for VBScript deprecation in Windows](https://devblogs.microsoft.com/microsoft365dev/how-to-prepare-vba-projects-for-vbscript-deprecation/) (2025-09-10) | 2026-09-26 |
| Office Version 2508 (Build 19127.20154) and later include `RegExp`, `Match`, `MatchCollection`, `SubMatches` natively in VBA – no reference to vbscript.dll needed. | same | 2026-09-26 |
