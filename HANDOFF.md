# Handoff — VBScout — 2026-09-27

## Stand
Phasen 1–3 abgeschlossen und gepusht (Branch `claude/blissful-feynman-7hsvb9`, HEAD 7fbdb88 „Phase 3: Sammler:
Office-Makros“). Code-Stand von Phase 3 in CI-Lauf 36279662753 auf Linux, windows-2025 und windows-2022 vollständig grün
(inkl. Systemtest, Vollscan mit Prüfung auf Windows-eigene Daten). Der letzte Amend 7fbdb88 ändert nur PROGRESS.md und
docs/research-notes.md; sein CI-Lauf wurde noch nicht geprüft. Nachweise und offene Abweichungen: siehe PROGRESS.md.

## Offene Punkte (priorisiert)
1. CI-Lauf für 7fbdb88 prüfen (MCP `mcp__github__actions_list` list_workflow_runs, Branch-Filter; owner
   `eliminatron-commits`, repo `VBScout`); Lauf-ID später in PROGRESS.md (Nachweis Phase 3) nachtragen.
2. Phase 4/6 „Auswertung und Berichte“ starten (bereits angekündigt, Nutzer hat noch nicht „weiter“ gesagt):
   Import, Entdoppeln, Risikobewertung, Migrationshinweise, Aufwandsschätzung (Faustwerte sichtbar gekennzeichnet),
   Protokoll-Abdeckung, Management-PDF, Excel-Liste, Gratis-Grenzen, Logo-/Organisationsfelder vorbereitet.
   Abnahme: Zusammenführungs-Leistung (Spez. Punkt 5), Berichte vollständig en+de.
3. In Phase 4 mit erledigen (PROGRESS.md, offene Punkte): Windows-eigene Funde (System32, WinSxS, Docker-Schichten)
   als „Windows-Bestandteil“ einordnen und Duplikate zusammenfassen; gesperrte `Current.mdb` (User Access Logging)
   entsprechend kennzeichnen; `cloudPlaceholder` in `docker\windowsfilter` bewerten.

## Wichtige Entscheidungen + Begründung
- Windows-Wartungsdaten (WinSxS-Differenziale PA30/PA31, komprimierte Nutzdaten DCx v1) und ESE-Datenbanken werden
  am Inhalt erkannt und sind kein Befund; nie Überspringen nach Pfad (Regel „kein stilles Überspringen“) – daher bleibt
  die gesperrte `Current.mdb` „nicht prüfbar“.
- Protokoll je Phase: Ankündigung „Phase X/6 – …“, Modellempfehlung, auf „weiter“ warten; am Ende Abnahme prüfen,
  ein Commit „Phase X: Name“ (Amend + `--force-with-lease` auf eigenem Branch), PROGRESS.md, 2–3 Sätze, nächste Phase
  ankündigen. Kommunikation auf Deutsch. Commit-Nachricht Phase 3: Scratchpad `commit3.txt` (nicht im Repo).

## Relevante Pfade
- PROGRESS.md — Stand, Nachweise, offene Punkte
- crates/vbs-evaluation/ — Auswertungslogik (Phase 4 baut hier)
- src-tauri/, src/ — App-Shell und Frontend (nur Anzeige)
- docs/result-format.md, docs/result.schema.json — Ergebnisformat, das die Auswertung importiert
- rules/catalog.json, i18n/ — Regeln (breaks/review) und Texte für Berichte
- product.json — Gratis-Grenze (`editions.free.maxMachines`), Name, Preise

## Empfehlung für Fortsetzung
- Modell: Opus 5.5
- Effort: mittel – umfangreiche, aber klar spezifizierte Umsetzung auf vorhandenem Ergebnismodell und Import
- Erster Schritt: CI-Lauf für 7fbdb88 prüfen, dann Phase 4 mit „weiter“ beginnen (Spezifikation Phase 4 aus PROGRESS.md/CLAUDE.md, Entscheidung 9 Editionen).
