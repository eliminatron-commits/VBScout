# Handoff — VBScout — 2026-09-26

## Stand
Phase 2 (Sammler: Systemebene) ist abgeschlossen: ein Commit `Phase 2: Sammler: Systemebene` auf
`claude/blissful-feynman-7hsvb9`; CI grün auf Linux, windows-2025 und windows-2022 (Tests, Korpus, ETW-/strace-Trace,
Netztest, Nicht-Admin-Lauf, Systemtest mit echten Windows-Artefakten, Vollscan ohne interne Fehler) – Läufe und offene
Punkte: PROGRESS.md. Beim Wiedereinstieg kurz prüfen, ob der Lauf des Abschluss-Commits grün ist.

## Offene Punkte (priorisiert)
1. Phase 3/6 „Sammler: Office-Makros“ beginnen – laut Protokoll zuerst Ankündigung + Modellzeile, dann auf „weiter“ warten.
2. Phase-3-Inhalt: Modul für Office-Dateien (OLE/CFB `vbaProject.bin`, OOXML-ZIP mit `vbaProject.bin`), VBA-Dekompression,
   Regeln 6xx mit Quellen, Texte in 8 Sprachen, Korpusfälle; `PENDING_KINDS` in `tests/corpus.rs` leeren.
3. Offene Punkte aus PROGRESS.md (4096-Format, Server 2016/Win10-Abnahme, Einordnung Windows-eigener Skripte in Phase 4).

## Wichtige Entscheidungen + Begründung
- Registry nur mit `KEY_READ`, ohne WOW64-Flags (32-Bit-Sicht über `SOFTWARE\WOW6432Node`): Flags setzen Handle-Tags,
  die der ETW-Trace sonst als „Set“ zählt; Handle-Tags aus Windows' COM/WMI-Code weist der Trace separat aus.
- Hives nicht angemeldeter Benutzer werden als Datei gelesen (`analysis/regf.rs`), nie geladen.
- MSI-Zeichenketten ≥ 64 KiB: Layout (0, Referenzen), (Low, High) wie Windows/Wine; msitools schreibt es anders
  (Fixture `long-strings.msi` wird deshalb konvertiert).
- Nicht prüfbare Befunde tragen `readError` (Leserfehler, ≤ 120 Zeichen); gesperrte Dateien = `locked`.

## Bekannte Probleme / Blocker
- learn/techcommunity/devblogs.microsoft.com und CI-Artefakt-Downloads (blob.core.windows.net) sind in der Umgebung
  gesperrt: Quellen per Suchauszug, CI-Diagnose über die Job-Logs (GitHub-Releases und raw.githubusercontent.com gehen).
- PowerShell-Fallen in den Windows-Skripten: Variablennamen case-insensitiv, Cmdlet-Ausgaben sind für COM in PSObject
  verpackt (auspacken), `echo 0> datei` ist in cmd eine Umleitung (`(echo 0)> datei`).

## Relevante Pfade
- `PROGRESS.md`, `CLAUDE.md` — Stand, Konventionen, verbotene Ansätze
- `crates/vbs-core/src/module.rs`, `views.rs` — Modul-Schnittstelle, Views
- `crates/vbs-collector/src/modules/`, `analysis/` — Module und Leser (Vorlage für Office: `msi.rs` mit `cfb`)
- `rules/catalog.json`, `i18n/`, `docs/research-notes.md` — Regeln, Texte, Quellen (Abschnitt „Office / VBA“)
- `tests/corpus/`, `crates/vbs-collector/tests/corpus.rs`, `tests/corpus/make-binaries.py` — Testsammlungen

## Empfehlung für Fortsetzung
- Modell: Opus 5.5
- Effort: hoch – Binärformate (CFB, VBA-Kompression, OOXML) und Passwort-/Verschlüsselungsfälle strikt nur lesend
- Erster Schritt: CI-Lauf des Abschluss-Commits prüfen, dann Phase-3-Ankündigung ausgeben und auf „weiter“ warten
