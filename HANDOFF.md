# Handoff — VBScout — 2026-09-26

## Stand
Phase 1 (Fundament) ist abgeschlossen und als ein Commit `Phase 1: Fundament` (5e9a656) auf `claude/blissful-feynman-7hsvb9`
gepusht. CI-Lauf 36253499363 war auf Linux und Windows vollständig grün (QK1 nur-lesend per Snapshot/strace/ETW, QK2 kein
Netz für Sammler und App); Details und offene Punkte siehe PROGRESS.md. Der Kontrolllauf für 5e9a656 (Unterschied nur
PROGRESS.md) lief bei Übergabe noch – kurz prüfen.

## Offene Punkte (priorisiert)
1. Phase 2/6 „Sammler: Systemebene“ beginnen – laut Protokoll zuerst Phase-Ankündigung + Modellzeile ausgeben und auf „weiter“ warten.
2. Phase-2-Inhalt: Module in `crates/vbs-collector/src/modules/` (Skriptdateien, Aufrufe/LNK, Tasks, Richtlinien-/Anmeldeskripte/SYSVOL, Autostart, Dienste, WMI, MSI-CAs per CFB-Parser, Event-Log 4096 `VBScriptDeprecationAlert` + Sysmon 1/7), Zugangsdaten-Befund (VBS-9xx) aus `vbs_core::secrets`, Regeln + Quellen in `rules/catalog.json`, Texte in 8 Sprachen, Korpusfälle + `PENDING_KINDS` in `tests/corpus.rs` kürzen.
3. Performance: Walk unter Windows mit FindFirstFileExW (LARGE_FETCH, Attribute aus Find-Daten) – Vollscan Runner 1,43 Mio. Einträge in 310 s.
4. CI: zusätzlicher Windows-2022-Lauf, Server-2016-Kompatibilität dokumentieren (QK3).

## Wichtige Entscheidungen + Begründung
- Rust-seitige Datei-Dialoge via `rfd` statt Tauri-dialog-Plugin → keine Frontend-Berechtigungen/fs-Plugin.
- Registry nativ ohne `KEY_WOW64_64KEY` (64-bit): das Flag erzeugt `SetInformationKey`-Ereignisse, die der ETW-Test als Änderung wertet.
- Sammler legt nie Ordner an, überschreibt nie, schreibt ohne `--out` nie unter `%SystemRoot%`.
- Microsoft-Quellen vorerst per Suchauszug (Domains gesperrt); Direktprüfung vor Phase 5 – Nutzer hat zugestimmt.
- Lizenz: Stepwright hat noch kein Ed25519/Worker → Entscheidung vor Phase 5.

## Bekannte Probleme / Blocker
- learn/techcommunity/devblogs.microsoft.com in der Umgebung gesperrt (nicht blockierend).
- PowerShell: Variablennamen case-insensitiv (Kollision `$webView`/`$WebView` war Fehlerursache); `pwsh` ist lokal installiert zum Testen.

## Relevante Pfade
- `PROGRESS.md`, `CLAUDE.md` — Stand, Konventionen, verbotene Ansätze
- `crates/vbs-core/src/module.rs`, `views.rs` — Modul-Schnittstelle, Read-only-Views
- `crates/vbs-collector/src/{modules/mod.rs,walk.rs,platform/}` — Modulregistrierung, Walk, OS-Zugriffe
- `rules/catalog.json`, `docs/research-notes.md` — Regeln/Quellen
- `tests/corpus/`, `crates/vbs-collector/tests/corpus.rs` — Testsammlungen
- `scripts/check-readonly.mjs`, `scripts/readonly/windows.ps1` — Nur-lesend-Nachweise

## Empfehlung für Fortsetzung
- Modell: Opus 5.5
- Effort: hoch – viele Windows-Spezialformate (Task-XML, LNK, MSI/CFB, EvtQuery, WMI) strikt nur lesend und belegte Einstufungen
- Erster Schritt: CI-Status von 5e9a656 prüfen, dann Phase-2-Ankündigung ausgeben und auf „weiter“ warten
