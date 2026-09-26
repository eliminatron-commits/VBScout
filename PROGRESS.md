# PROGRESS – VBScout

| Phase | Status | Modell |
|---|---|---|
| 1/6 Fundament | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 2/6 Sammler: Systemebene | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 3/6 Sammler: Office-Makros | ⏭ als Nächstes | Opus 5.5 (hoch) |
| 4/6 Auswertung und Berichte | offen | Opus 5.5 (mittel) |
| 5/6 Lizenzen, Übersetzungen, Pakete, Abnahme | offen | Sonnet 5 (mittel) |
| 6/6 Vermarktung | offen | Opus 5.5 (mittel) |

**Nachweis Phase 1:** CI-Lauf 36253499363 grün. QK1: statische Nur-lesend-Prüfung mit Selbsttest, Snapshot-Test,
strace-/ETW-Kernel-Trace mit Positivkontrolle – 0 Änderungen. QK2: Offline-Prüfung inkl. Abhängigkeitsgraph,
Netzwerk-Blockadetest Sammler und App (inkl. WebView2) – 0 Verbindungen. Leere Ergebnisdatei schema-gültig.

**Nachweis Phase 2:** CI-Lauf 36267131748 auf Linux, windows-2025 und windows-2022 vollständig grün; Lauf 36270432437
bestätigt den letzten Fix (MSI) – Linux und windows-2025 vollständig, windows-2022 bis einschließlich Systemtest (den
Vollscan wiederholt der Lauf des Abschluss-Commits). 11 Module, 30 Regeln (28 neu) mit 17 datierten Quellen, Texte in
8 Sprachen. Testsammlung positiv (64 erwartete Befunde) / negativ (0) inkl. System-Fixtures; Binärdateien von
unabhängigen Werkzeugen (msitools, pylnk3, hivex). QK1: ETW-/strace-Trace inkl. WMI und Ereignisprotokoll – 0
Änderungen (Handle-Tags aus Windows' COM/WMI-Code separat ausgewiesen; eigener Code ohne WOW64-Flags). QK3: Sammler
1,89 MB, importiert nur Windows-Systembibliotheken (keine C-Laufzeit, kein Netz), läuft ohne Installation auf Server
2025/2022; ohne Adminrechte „eingeschränkt“ mit Hinweis. QK5 (Dateiteil): 100.000 Dateien in < 1 s; Vollscan der
Runner 1,44 Mio. (2025) / 1,82 Mio. Einträge (2022) ohne interne Fehler. Systemtest mit von Windows erzeugten
Artefakten (Aufgabe, Run-Wert, Dienst-Wrapper, WMI-Abo, Verknüpfung, msi.dll-Paket, Script Encoder, Profil eines nicht
angemeldeten Benutzers) grün. Real gefundener Fehler behoben: lange MSI-Zeichenketten (SQL Server 2016).

**Offene Punkte / Abweichungen**
- Stepwright hat selbst erst Phase 1: kein Ed25519-Format, kein Worker → vor Phase 5 entscheiden.
- Microsoft-Quellen nur per Suchauszug geprüft (learn/techcommunity/devblogs gesperrt) → vor Release direkt gegenlesen.
- Platzhalter in `product.json` (Domain, Support-Mail, Identifier), Markenprüfung „VBScout“, Lizenzgeber für LICENSE offen.
- QK3: Windows 10/11 und Server 2016/2019 nicht in CI (Rust-Ziel belegt „Windows 10+/Server 2016+“) → Lauf auf echten
  Maschinen bei der Abnahme (Phase 5).
- Ereignis 4096 wurde auf keinem Runner protokolliert → Datenformat auf einem System verifizieren, das es schreibt.
- Phase 4: Windows-eigene Skripte (System32, WinSxS, Docker-Image-Schichten) als „Windows-Bestandteil“ einordnen und
  Duplikate zusammenfassen; Platzhalter in `docker\windowsfilter` erscheinen als `cloudPlaceholder` (eigener Grund?).
- Optional: Walk unter Windows beschleunigen (Vollscan 6–12 min); PowerShell-Dateien > 16 MB sind `tooLarge`.
