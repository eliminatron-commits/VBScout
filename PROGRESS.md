# PROGRESS – VBScout

| Phase | Status | Modell |
|---|---|---|
| 1/6 Fundament | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 2/6 Sammler: Systemebene | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 3/6 Sammler: Office-Makros | ✅ abgeschlossen 2026-09-26 | Opus 5.5 (hoch) |
| 4/6 Auswertung und Berichte | ✅ abgeschlossen 2026-09-27 | Opus 5.5 (mittel) |
| 5/6 Lizenzen, Übersetzungen, Pakete, Abnahme | ⏭ als Nächstes | Sonnet 5 (mittel) |
| 6/6 Vermarktung | offen | Opus 5.5 (mittel) |

**Nachweis Phase 1:** CI-Lauf 36253499363 grün. QK1: statische Nur-lesend-Prüfung mit Selbsttest, Snapshot-Test,
strace-/ETW-Kernel-Trace mit Positivkontrolle – 0 Änderungen. QK2: Offline-Prüfung inkl. Abhängigkeitsgraph,
Netzwerk-Blockadetest Sammler und App (inkl. WebView2) – 0 Verbindungen. Leere Ergebnisdatei schema-gültig.

**Nachweis Phase 2:** CI-Lauf 36267131748 auf Linux, windows-2025 und windows-2022 vollständig grün; Lauf 36270432437
bestätigt den letzten Fix (MSI) – Linux und windows-2025 vollständig, windows-2022 bis einschließlich Systemtest; Lauf
36271987377 des Abschluss-Commits auf allen Runnern vollständig grün (inkl. Vollscan windows-2022). 11 Module, 30 Regeln
(28 neu) mit 17 datierten Quellen, Texte in 8 Sprachen. Testsammlung positiv (64 erwartete Befunde) / negativ (0) inkl.
System-Fixtures; Binärdateien von unabhängigen Werkzeugen (msitools, pylnk3, hivex). QK1: ETW-/strace-Trace inkl. WMI
und Ereignisprotokoll – 0 Änderungen (Handle-Tags aus Windows' COM/WMI-Code separat ausgewiesen; eigener Code ohne
WOW64-Flags). QK3: Sammler 1,89 MB, importiert nur Windows-Systembibliotheken (keine C-Laufzeit, kein Netz), läuft ohne
Installation auf Server 2025/2022; ohne Adminrechte „eingeschränkt“ mit Hinweis. QK5 (Dateiteil): 100.000 Dateien
in < 1 s; Vollscan der Runner 1,44 Mio. (2025) / 1,82 Mio. Einträge (2022) ohne interne Fehler. Systemtest mit von
Windows erzeugten Artefakten (Aufgabe, Run-Wert, Dienst-Wrapper, WMI-Abo, Verknüpfung, msi.dll-Paket, Script Encoder,
Profil eines nicht angemeldeten Benutzers) grün. Real gefundener Fehler behoben: lange MSI-Zeichenketten (SQL Server 2016).

**Nachweis Phase 3:** CI-Läufe 36279662753 und 36280760857 (Abschluss-Commit 7fbdb88) auf Linux, windows-2025 und
windows-2022 vollständig grün (inkl. Systemtest mit Office-Teil, Vollscan der Runner und Prüfung auf Windows-eigene
Daten). Modul `office-macro` mit eigenen Lesern ohne Office: Verbunddateien (.xls/.xla/.xlt/.doc/.dot, eingebettete Objekte),
Open-XML-Pakete (.xlsm/.xlsb/.xlam/.xltm/.docm/.dotm/.pptm/.potm/.ppsm/.ppam, eingebettete Pakete), Access 2000–2016
(.mdb/.accdb über MSysAccessStorage bzw. MSysAccessObjects, auch RC4-kodierte Jet-4-Datenbanken); MS-OVBA-Dekompression,
dir/PROJECT-Stream, Schutzstatus. 7 neue Regeln (600, 601, 611, 612, 621, 622, 632), 5 neue datierte Quellen, Texte in 8
Sprachen; VBScript über Script Control und `execScript` (Weg um das fehlende ScriptControl in 64-Bit-Office). QK4:
Positivsammlung 103 erwartete Befunde (39 Office/Access) zu 100 % erkannt, Negativsammlung 0 Befunde; geschützte Makros
(Kennwort zum Öffnen – echte Dateien von oletools –, Rechteverwaltung, nur kompilierter Code, Excel 5/95, ActiveMime)
als „not checkable“. Echte Office-Dateien (Apache POI, oletools, Jackcess; `tests/corpus/THIRD-PARTY.md`) werden so
gelesen, wie Apache POIs eigene Tests es erwarten (29 Module, Mac-Codepage, falscher Modul-Offset – an dem oletools
scheitert). Erzeugte Fälle: Container von Apache POI (POIFS) und Jackcess, jedes Projekt mit oletools und Apache POI
gegengelesen. QK5: 100.000 Dateien inkl. 1.000 Office-/Access-Dateien in 0,2 s (lokal). QK1/QK2: strace-Trace und
Netztest über die Sammlung inkl. Office-Dateien grün (0 Änderungen, 0 Verbindungen). Robustheit: 100.000 gezielt
beschädigte Varianten der Office-/Access-Dateien und Zufallseingaben für alle Decoder ohne Absturz. Windows-Systemtest
ergänzt: Paket von Microsofts `System.IO.Packaging` mit VBA-Projekt (VBS-601) und Datenbank der Jet-Engine von Windows
(kein Befund). Vollscan der Runner: keine Fehlerkennung durch die Makro-Regeln; gefunden und behoben – Windows-eigene
Daten unter Kandidatennamen erschienen als VBS-101/200/600: Differenziale und komprimierte Nutzdaten des
Komponentenspeichers `WinSxS` (auch Skripte nicht installierter Features wie IIS-Legacy-Skripte und WSUS, auch in
Docker-Schichten) und ESE-Datenbanken der Benutzerzugriffsprotokollierung. Jetzt am Inhalt erkannt
(`analysis/servicing.rs`, ESE-Signatur): 16 (windows-2025) bzw. 35 (windows-2022) Fehlbefunde weniger, kein neuer;
Negativsammlung um solche Dateien ergänzt, der CI-Vollscan schlägt fehl, falls sie wieder als Befund auftauchen.
Einziger Office-Befund der Runner außerhalb der Testsammlung: die gesperrte `Current.mdb` (siehe offene Punkte).

**Nachweis Phase 4:** CI-Lauf 36303201857 (9d5e537) auf Linux, windows-2025 und windows-2022 vollständig grün (inkl.
Merge-Leistung, Berichte in 8 Sprachen gegengelesen, App-Smoke-Test mit allen Berichten, Berichte der Runner-Vollscans).
Auswertung in `vbs-evaluation`: neuester Scan je Rechner, Entdoppeln (gleiche Netzwerkdatei von
mehreren Rechnern, gleicher Inhalt → Aufwand einmal), Verknüpfung Eintrag → gestartetes Skript, Risiko nach Vorgabe
(automatisch > laut Protokoll > Office-Makro > ruhend; `breaks` vor `review`), Migrationshinweis und Faustwert je Regel
im Katalog (`hint`, `effort` mit Basis fest/Skriptgröße/Eintrag). Windows-Bestandteile (WinSxS, Wartung, Docker-Schichten,
`LogFiles\Sum\*.mdb` = gesperrte `Current.mdb`, gleicher Hash wie im Komponentenspeicher) getrennt als „Info“ ohne
Aufwand – offener Punkt aus Phase 3 erledigt. Management-PDF (krilla, eingebettete Liberation Sans, „Seite X von Y“,
Lesezeichen) und Excel-Liste (bis zu 7 Blätter) in 8 Sprachen; App mit Übersicht, Befundliste, Detail, Rechnern, Berichten.
Abnahme: 1.000 Ergebnisdateien (210.410 Funde → 100.183 Einträge) in 3,1 s gelesen und zusammengeführt (Grenze 60 s,
CI-Schritt Linux/Windows); Berichte in allen Sprachen vollständig – Rust-Tests und unabhängig per pdftotext/zipfile
(`scripts/reports/check.py`, CI); jeder Aufwand als „Faustwert“ gekennzeichnet (PDF, Excel, App). Gratis-Edition in Rust
durchgesetzt (≤ 25 Rechner bei Import und Auswertung, PDF verweigert, keine Hinweise/Aufwände in Ansichten und Excel);
Organisationsname und MSP-Logo/Firmenname vorbereitet. Release-App: Linux-Smoke-Test inkl. Berichten grün. Vollscan
windows-2025 als Organisation ausgewertet: 195 eigene Einträge (hoch 6, mittel 29, niedrig 160; 17 nicht prüfbar) und
16 Windows-Bestandteile (86 Fundorte); Einstufung durchgesehen – hoch sind nur Fixtures der Testsammlung unter
Autostart-/Anmeldeskript-Pfaden, deaktivierte Aufgaben (Server Manager `CleanupOldPerfLogs`) ruhend, keine Fehleinstufung.

**Offene Punkte / Abweichungen**
- QK4-Präzisierung: Ein nur „für die Anzeige gesperrtes“ VBA-Projekt wird gelesen (die Sperre verschlüsselt den
  Quelltext nicht) und mit `projectLocked` gemeldet – „not checkable“ nur, wenn der Code wirklich nicht lesbar ist.
- WSH-Objekte werden wie vorgegeben als `review` gemeldet; die Scripting Runtime (FileSystemObject, Dictionary) nicht –
  Microsoft nennt nur vbscript.dll (Quellen und Begründung: `docs/research-notes.md`).
- Nicht gelesen, aber gemeldet: Access 97 mit Code, Excel-5/95-Modulblätter, Web-/XML-Dokumente mit ActiveMime.
  Nicht im Umfang: PowerPoint-/Visio-Binärformate, XLM-Makros, Access-Makroobjekte, kompilierte .mde/.accde.
- Access 97 mit Code nur lokal an Northwind geprüft (Lizenz unklar, nicht im Repo); ACE-Datenbanken mit Kennwort nur
  über das Kopffeld erkannt (kein echtes Beispiel). Dateien mit Excels Standardkennwort gelten als kennwortgeschützt.
- Stepwright hat selbst erst Phase 1: kein Ed25519-Format, kein Worker → vor Phase 5 entscheiden.
- Microsoft-Quellen nur per Suchauszug geprüft (learn/techcommunity/devblogs gesperrt) → vor Release direkt gegenlesen.
- Platzhalter in `product.json` (Domain, Support-Mail, Identifier), Markenprüfung „VBScout“, Lizenzgeber für LICENSE offen.
- QK3: Windows 10/11 und Server 2016/2019 nicht in CI (Rust-Ziel belegt „Windows 10+/Server 2016+“) → Lauf auf echten
  Maschinen bei der Abnahme (Phase 5).
- Ereignis 4096 wurde auf keinem Runner protokolliert → Datenformat auf einem System verifizieren, das es schreibt.
- Phase 4: Die App läuft bis zu den Lizenzschlüsseln (Phase 5) immer als Gratis-Edition; Organisation/MSP nur in Tests
  und `examples/report.rs --edition`. Aufwandswerte sind eigene Faustwerte ohne externe Quelle (so gekennzeichnet) →
  mit Praxiswerten nachjustieren. Drittanbieter-Hinweise (Liberation Sans/OFL, Crates) für das Release zusammenstellen.
  PDF/Excel des Runner-Vollscans (Ordner `reports-out/` im CI-Artefakt `traces-windows-2025`) einmal ansehen – die
  Einstufung ist geprüft, Layout mit echten Daten noch nicht. fr/es/it/nl/pl/pt-BR maschinell
  unterstützt (Korrekturhinweis in der App). „Manage the Component Store“ (Microsoft Learn) vor Release gegenlesen.
- Optional: Walk unter Windows beschleunigen (Vollscan 6–12 min); PowerShell-Dateien > 16 MB sind `tooLarge`.
