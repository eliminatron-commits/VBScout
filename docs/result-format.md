# The `.vbscout` result file (schema version 1)

One collector run on one machine produces one result file. It is a ZIP archive; extension and media type come from
the central product configuration (`product.json` → `resultFile`), currently `.vbscout` and
`application/vnd.vbscout.result+zip`.

```text
mimetype       first entry, stored (uncompressed): the media type as ASCII, no newline
result.json    deflated: the scan result (UTF-8 JSON), see below and docs/result.schema.json
```

No other entries are read. The collector builds the archive in memory and writes it in one go to a file it creates
itself (`create_new`) – it never overwrites an existing file.

## Content

The JSON Schema [`result.schema.json`](result.schema.json) is normative; [`examples/result.example.json`](examples/result.example.json)
shows a complete example. In short:

| Field | Meaning |
|---|---|
| `format`, `schemaVersion` | media type and schema version (1) |
| `scanId` | UUID of the run – the same run imported twice is recognised |
| `generator` | product, component (`collector`), version, date of the rule catalog (`rulesAsOf`), platform |
| `startedAt`, `finishedAt` | UTC, RFC 3339 |
| `machine` | host name, FQDN, DNS domain (local configuration – no directory query), pseudonymous `machineId` (SHA-256 of the Windows MachineGuid), operating system (name, version, build, feature update, edition, role, architecture) |
| `scope` | what was asked for: all local drives or `--path` folders, `--include-unc` network paths, system sources on/off |
| `coverage` | what could be checked: `full`/`limited`, administrator rights, `limitations` (codes) and one entry per source with status, reason, counters, examples of unreadable items and – for logs – the time span covered |
| `findings` | the dependencies found, see below |

### Findings

| Field | Meaning |
|---|---|
| `id` | `f1`, `f2`, … unique within the file, stable order |
| `rule` | rule ID from `rules/catalog.json`, e.g. `VBS-101` |
| `kind` | finding type: `scriptFile`, `scriptInvocation`, `shortcut`, `scheduledTask`, `autostart`, `service`, `wmiSubscription`, `logonScript`, `msiCustomAction`, `eventLogUsage`, `officeMacro`, `hardcodedCredential` |
| `classification` | `breaks` or `review`, copied from the rule catalog at scan time |
| `status`, `reason` | `detected`, or `notCheckable` with a reason (`passwordProtected`, `accessDenied`, `locked`, `corrupt`, `unsupportedFormat`, `tooLarge`, `cloudPlaceholder`, `encrypted`, `networkLocation`, `internalError`) – nothing is skipped silently |
| `activation` | how it runs – basis of the risk ranking: `automatic`, `logged`, `macro`, `installer`, `manual`, `dormant` |
| `location` | `type` (`file`, `registry`, `scheduledTask`, `service`, `wmi`, `eventLog`, `msiPackage`), `path` and optional `item` (value, action, VBA module, custom action, …) |
| `target` | script or program the finding runs, as written |
| `file` | size, modification time, SHA-256 (when read completely), `network` – for de-duplication across machines |
| `evidence` | at most 5 affected lines, each at most 240 characters, secrets masked (`********`, `masked: true`) |
| `details` | small module-specific scalar values |

### Coverage sources

| Source | What it covers |
|---|---|
| `files.localDrives`, `files.paths`, `files.networkPaths` | the file walk: all fixed drives, `--path` folders, `--include-unc` paths |
| `tasks.scheduled` | task definitions in `%SystemRoot%\System32\Tasks` (needs administrator rights) |
| `autostart.entries` | Run/RunOnce keys (machine, both registry views, every user profile: loaded hives under `HKEY_USERS`, the hives of users who are not logged on read from their `NTUSER.DAT` file – never loaded), Winlogon, Active Setup, command processor AutoRun, startup folders of all users and profiles; `skipped` counts profiles whose hive file could not be read (`partial`, reason `userHivesNotRead`) |
| `services.configuration` | services: image path, failure command, srvany/NSSM `Parameters\Application` |
| `wmi.subscriptions` | permanent WMI consumers (`ActiveScriptEventConsumer`, `CommandLineEventConsumer`) and their bindings |
| `policies.scripts` | Group Policy scripts as applied (registry, including the policy state of users who are not logged on and their hive files), `UserInitMprLogonScript`; `scripts.ini` files are found by the walk |
| `installer.packages` | cached packages of installed products (`%SystemRoot%\Installer`, via the registry) |
| `eventLog.vbscriptDeprecation` | Application log, event 4096 of `VBScriptDeprecationAlert`; `timeRange` = span of records available |
| `eventLog.sysmon` | Sysmon log, events 1 and 7 (`unavailable`/`notInstalled` without Sysmon); `timeRange` as above |

A source is `complete`, `partial` (reason, e.g. `notElevated`, `unreadableItems`, `userHivesNotRead`),
`unavailable` or `failed`. System modules never read network paths: a script on a network path that a task or policy
starts is reported as `notCheckable` with reason `networkLocation`.

### Details (by finding type)

| Type | Detail keys |
|---|---|
| all command-based types | `via` (`wscript`, `cscript`, `mshta`, `rundll32`, `direct`, `inline`, `script`), `calledScriptStarts` (when a called batch/PowerShell file starts VBScript) |
| `scriptFile` | `encoded`, `vbscriptBlocks` |
| `scriptInvocation` | `calls` (lines that start VBScript) |
| `shortcut` | `workingDirectory`, `recentItem` |
| `scheduledTask` | `enabled`, `triggers` |
| `autostart` | `autostartType` (`run`, `runOnce`, `runOnceEx`, `policyRun`, `winlogon`, `windowsLoad`, `activeSetup`, `commandProcessor`, `startupFolder`, …), `offlineHive` (read from the hive file of a user who is not logged on) |
| `service` | `startType`, `displayName` |
| `wmiSubscription` | `consumerClass`, `scriptingEngine`, `bound`, `eventQuery` |
| `logonScript` | `phase`, `policyName`, `offlineHive` |
| `msiCustomAction` | `customActionType`, `scriptSource`, `scheduled`, `continueOnError`, `condition`, `productName`, `productVersion`, `publisher`, `packagePath` |
| `eventLogUsage` | `events`, `firstSeen`, `lastSeen`, `processTree`, `image`, `parentImage` |
| `hardcodedCredential` | `secrets` (count), `secretKinds` (`password`, `connectionString`, `urlCredentials`) |
| `notCheckable` findings of read formats (`shortcut`, `msiCustomAction`) | `readError` – why the reader gave up, at most 120 characters (e.g. `link info`, `no string pool`, `not a regular file`); never file contents |

## Privacy and security invariants (enforced when writing)

* Evidence contains only the affected lines, shortened, with control characters removed.
* Passwords, connection-string passwords and URL credentials are masked before a line is stored – once when a module
  reports it and again when the file is written. Secrets in a script, macro or command that uses VBScript are reported
  as a `hardcodedCredential` finding (`VBS-901`) with the masked lines only, never the value.
* No user names or profile paths beyond what a finding's location requires; the machine ID is pseudonymous.

## Limits (files are untrusted input)

At most 16 archive entries, `mimetype` ≤ 256 bytes, `result.json` ≤ 256 MiB, 500,000 findings, 10,000 coverage
sources. Readers accept up to 50 evidence lines of 4,096 characters per finding; texts ≤ 32,768 characters.

## Versioning

* Additive changes – new optional fields and new values of the open enumerations (`kind`, `activation`, `reason`,
  limitation codes, source IDs, detail keys) – keep `schemaVersion`. Readers keep unknown values verbatim, show them
  generically and treat an unknown classification as `review`.
* Every other change increments `schemaVersion` and ships a migration (`crates/vbs-core/src/migrate.rs`). Readers
  refuse files with a newer `schemaVersion` instead of guessing. The evaluation keeps reading every version ever
  released – collectors stay in the field for a long time.
