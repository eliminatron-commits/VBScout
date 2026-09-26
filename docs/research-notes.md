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

## Office / VBA (input for phase 3)

| Fact | Source | Checked |
|---|---|---|
| VBA projects are affected when they use VBScript, e.g. `VBScript.RegExp` (CreateObject or reference to "Microsoft VBScript Regular Expressions"), call .vbs files, or run VBScript through ScriptControl. | Microsoft 365 Developer Blog – [Prepare your VBA projects for VBScript deprecation in Windows](https://devblogs.microsoft.com/microsoft365dev/how-to-prepare-vba-projects-for-vbscript-deprecation/) (2025-09-10) | 2026-09-26 |
| Office Version 2508 (Build 19127.20154) and later include `RegExp`, `Match`, `MatchCollection`, `SubMatches` natively in VBA – no reference to vbscript.dll needed. | same | 2026-09-26 |
