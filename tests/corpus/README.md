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

System sources (registry, scheduled tasks, services, WMI, event logs) are tested with in-memory
views (`vbs_core::views::MemoryRegistry` and friends) next to the module tests.

Rules for new cases:

* Keep files small and self-explanatory; one aspect per file, named after it.
* Store the bytes exactly as Windows would (CRLF line endings, UTF-16 where relevant) –
  `.gitattributes` keeps this folder byte-exact.
* Never put real credentials, customer data or real hostnames here; secrets in cases are obvious
  fakes (`Sommer2024!`, `hunter2`).
* Every false positive or false negative found in the field becomes a new case.
