# Test collections

Definition of Done #4: every finding type is recognised in the **positive** collection (100 %), the
**negative** collection produces **no** finding at all, and protected macros are reported as
"not checkable".

```text
positive/        every case here must produce exactly the findings listed in expected.json
negative/        nothing here may produce a finding – look-alikes, other script languages, prose
expected.json    the expected findings of positive/ (path relative to positive/, rule, status)
```

`crates/vbs-collector/tests/corpus.rs` runs all registered modules over both folders and checks:

* every expected finding is reported (recall 100 %) and nothing else is reported in `positive/`;
* `negative/` yields zero findings;
* every rule a module can report has at least one positive case;
* every finding type has a module – types without one yet are listed in `PENDING_KINDS` in the
  test, which shrinks phase by phase (phase 2: system level, phase 3: Office macros) and must be
  empty at the end.

Each collection is scanned like a machine: the file walk over the whole folder plus every system
module over the collection's `system/` fixtures:

```text
system/registry.json        registry keys and values (HKLM/HKU, native or 32-bit view); "{root}" = this system/ folder
system/events.json          event log records per channel (Application, Sysmon/Operational)
system/wmi.json             WMI instances per namespace and class
system/Windows/…            %SystemRoot%: task definitions (System32/Tasks), cached packages (Installer), scripts
system/ProgramData/…        %ProgramData%: the all-users startup folder
system/Users/<name>/…       profiles listed in registry.json (ProfileList): per-user startup folders;
                            NTUSER.DAT = hive file of a user who is not logged on (carol, dave)
```

Binary and specially encoded files (`.lnk`, `.msi`, `.vbe`, `NTUSER.DAT`, UTF-16 files) are made by
`make-binaries.py` with independent writers (pylnk3, msitools, hivex) – the readers are not tested only
against files they wrote themselves. `installer/long-strings.msi` holds a string of more than
128 KiB; its string pool entry is converted to the layout of Windows Installer (msitools writes it
differently and cannot read it back itself) and checked with msitools' reader. The Windows CI job
adds a system test with artefacts made by Windows itself (`scripts/systemtest/windows.ps1`).

`expected.json` lists every finding of `positive/` (path, item, rule, status, target). To see what
the collection yields after a change, run
`cargo test -p vbs-collector --test corpus -- --ignored --nocapture print_positive_cases`, review
the output and update the file.

Rules for new cases:

* Keep files small and self-explanatory; one aspect per file, named after it.
* Store the bytes exactly as Windows would (CRLF line endings, UTF-16 where relevant) –
  `.gitattributes` keeps this folder byte-exact.
* Never put real credentials, customer data or real hostnames here; secrets in cases are obvious
  fakes (`Sommer2024!`, `hunter2`).
* Every false positive or false negative found in the field becomes a new case.
