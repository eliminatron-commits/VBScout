# Handoff — VBScout — 2026-09-27

## Stand
Phasen 1–4 abgeschlossen (Branch `claude/blissful-feynman-7hsvb9`, Commit „Phase 4: Auswertung und Berichte“).
Auswertung, Management-PDF und Excel-Liste (8 Sprachen) und die App-Oberfläche sind fertig; lokal grün: `check:all`,
fmt, clippy (inkl. Windows-Ziel), alle Tests, Release-Smoke-Test der App. CI-Lauf und Nachweise: siehe PROGRESS.md.

## Offene Punkte (priorisiert)
1. Phase 5/6 „Lizenzen, Übersetzungen, Pakete, Abnahme“ (angekündigt, wartet auf „weiter“): Organisations- und
   MSP-Lizenz (Ed25519, nur offline) samt Worker und Paddle-Sandbox, 8 Sprachen, Build-Pipeline, Signatur-Vorbereitung,
   winget, README, LICENSE, Datenschutzhinweis; Abnahme: DoD 1–7 in PROGRESS.md belegt.
2. Vorab entscheiden: Stepwright hat noch kein Lizenzformat und keinen Worker (PROGRESS.md, offene Punkte).
3. Übrige offene Punkte/Abweichungen: PROGRESS.md.

## Wichtige Entscheidungen + Begründung
- Editionen nur in Rust (`crates/vbs-evaluation/src/edition.rs`, `Edition::from_license`); die App ist bis Phase 5
  immer Gratis-Edition – Phase 5 lädt die Lizenz und setzt sie in `AppState` (`src-tauri/src/state.rs`).
- Protokoll je Phase unverändert: Ankündigung, auf „weiter“ warten, Abnahme prüfen, ein Commit „Phase X: Name“
  (Amend + `--force-with-lease`), PROGRESS.md, 2–3 Sätze, nächste Phase ankündigen; Deutsch.

## Relevante Pfade
- PROGRESS.md — Stand, Nachweise, offene Punkte
- crates/vbs-license/ — Lizenzschnittstelle (Phase 5: Format und Prüfung)
- crates/vbs-evaluation/src/edition.rs, src-tauri/src/state.rs, src-tauri/src/commands.rs — Edition in der App
- product.json — Preise, Gratis-Grenze, Platzhalter
- i18n/languages.json — Prüfstatus der Sprachen

## Empfehlung für Fortsetzung
- Modell: Sonnet 5
- Effort: mittel – laut Phasenplan; Lizenzformat vorher entscheiden (Punkt 2)
- Erster Schritt: Nach „weiter“ Phase 5 beginnen: Stand von Stepwrights Lizenzsystem prüfen und das Format festlegen.
