# PROGRESS – VBScout

| Phase | Status | Modell |
|---|---|---|
| 1/6 Fundament | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 2/6 Sammler: Systemebene | ⏭ als Nächstes | Opus 5.5 (hoch) |
| 3/6 Sammler: Office-Makros | offen | Opus 5.5 (hoch) |
| 4/6 Auswertung und Berichte | offen | Opus 5.5 (mittel) |
| 5/6 Lizenzen, Übersetzungen, Pakete, Abnahme | offen | Sonnet 5 (mittel) |
| 6/6 Vermarktung | offen | Opus 5.5 (mittel) |

**Nachweis Phase 1:** CI-Lauf 36253499363 grün (Stand bis auf diese Datei identisch). QK1: statische Nur-lesend-Prüfung
mit Selbsttest, Snapshot-Test des echten Sammlers (Linux/Windows), strace- und ETW-Kernel-Trace mit Positivkontrolle –
0 Änderungen, einzige neue Datei = Ergebnis (`FILE_CREATE`). QK2: Offline-Prüfung inkl. aufgelöstem Abhängigkeitsgraph,
Netzwerk-Blockadetest Sammler (Linux/Windows) und App samt WebView2 (Windows) – 0 Verbindungen, 0 DNS, Positivkontrolle
erkannt. Sammler 1,05 MB, App 8,9 MB; leere Ergebnisdatei schema-gültig und von der Auswertung gelesen.

**Offene Punkte / Abweichungen**
- Stepwright hat selbst erst Phase 1: kein Ed25519-Format, kein Worker → vor Phase 5 entscheiden (hier bauen und zurückportieren oder Stepwright abwarten).
- Microsoft-Quellen nur per Suchauszug geprüft (learn/techcommunity/devblogs in dieser Umgebung gesperrt) → vor Release direkt gegenlesen (`docs/research-notes.md`).
- Platzhalter in `product.json` (Domain, Support-Mail, Identifier), Markenprüfung „VBScout“, Lizenzgeber für LICENSE offen (Repo ist bereits öffentlich).
- Phase 2: Walk unter Windows beschleunigen (FindFirstFileExW; Vollscan Runner 1,43 Mio. Einträge in 310 s), CI zusätzlich auf windows-2022, Server-2016-Kompatibilität belegen.
