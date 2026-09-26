#!/usr/bin/env node
// Static half of the read-only guarantee (Definition of Done #1). The dynamic half:
// crates/vbs-collector/tests/read_only.rs (snapshot before/after) and scripts/readonly/
// (system-call trace on Linux, kernel ETW trace on Windows).
//
// The collector may open files, registry keys, logs, tasks and WMI for reading only, and writes
// exactly one file – its result – in crates/vbs-collector/src/output.rs. This check scans the
// collector and the shared core (`#[cfg(test)] mod … { … }` blocks excluded – tests may create fixtures):
//
//  1. APIs that change a system are forbidden everywhere – registry writes and hive loading,
//     event log clearing/exporting/writing, task scheduler/WMI/service changes, privilege
//     adjustment, process start, network drive mapping, msi.dll, overwrite/truncate flags.
//  2. File writes, moves, deletions, attribute or time changes are allowed only in output.rs.
//  3. OpenOptions only in read_only.rs (reading) and output.rs (the result file); output.rs must
//     create its file with create_new(true), so nothing – not even an old result – is overwritten.
//  4. unsafe code only in platform/windows/ (the audited read-only FFI).
//  5. No write-helper crates among the collector's dependencies.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const SCOPE = ['crates/vbs-collector/src', 'crates/vbs-core/src'];
const WRITER = 'crates/vbs-collector/src/output.rs';
const READER = 'crates/vbs-collector/src/read_only.rs';
const UNSAFE_DIR = 'crates/vbs-collector/src/platform/windows/';
const errors = [];
const rel = (path) => relative(root, path).replaceAll('\\', '/');

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* walk(path);
    else yield path;
  }
}

/** Removes comments and the contents of string literals, keeping line structure. */
function stripRust(source) {
  let out = '';
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    const next = source[i + 1];
    if (c === '/' && next === '/') {
      while (i < source.length && source[i] !== '\n') i++;
    } else if (c === '/' && next === '*') {
      let depth = 1;
      i += 2;
      while (i < source.length && depth > 0) {
        if (source[i] === '/' && source[i + 1] === '*') (depth++, (i += 2));
        else if (source[i] === '*' && source[i + 1] === '/') (depth--, (i += 2));
        else {
          if (source[i] === '\n') out += '\n';
          i++;
        }
      }
    } else if (c === 'r' && /^r#*"/.test(source.slice(i, i + 10)) && !/[A-Za-z0-9_]/.test(source[i - 1] ?? '')) {
      const hashes = source.slice(i + 1).match(/^#*/)[0].length;
      const end = source.indexOf(`"${'#'.repeat(hashes)}`, i + 2 + hashes);
      const literal = source.slice(i, end === -1 ? source.length : end + 1 + hashes);
      out += `""${'\n'.repeat((literal.match(/\n/g) ?? []).length)}`;
      i += literal.length;
    } else if (c === '"') {
      i++;
      let newlines = 0;
      while (i < source.length && source[i] !== '"') {
        if (source[i] === '\\') i++;
        else if (source[i] === '\n') newlines++;
        i++;
      }
      i++;
      out += `""${'\n'.repeat(newlines)}`;
    } else {
      out += c;
      i++;
    }
  }
  return out;
}

/** Blanks `#[cfg(test)] mod … { … }` blocks (keeping line numbers); code after them is still checked. */
function withoutTestModules(code) {
  const marker = /#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{/g;
  let result = '';
  let cursor = 0;
  for (let match = marker.exec(code); match; match = marker.exec(code)) {
    if (match.index < cursor) continue;
    let depth = 1;
    let i = match.index + match[0].length;
    while (i < code.length && depth > 0) {
      if (code[i] === '{') depth++;
      else if (code[i] === '}') depth--;
      i++;
    }
    result += code.slice(cursor, match.index) + code.slice(match.index, i).replace(/[^\n]/g, '');
    cursor = i;
    marker.lastIndex = i;
  }
  return result + code.slice(cursor);
}

// 1. Never, anywhere.
const FORBIDDEN_ALWAYS = [
  [/\bReg(?:CreateKey|SetValue|SetKeyValue|DeleteKey|DeleteValue|DeleteTree|LoadKey|UnLoadKey|SaveKey|RestoreKey|ReplaceKey|CopyTree|RenameKey|SetKeySecurity|FlushKey|ConnectRegistry|OverridePredefKey|DisableReflectionKey|EnableReflectionKey|LoadAppKey|LoadMUIString)\w*/, 'registry write or hive loading'],
  [/\bKEY_(?:WRITE|ALL_ACCESS|SET_VALUE|CREATE_SUB_KEY|CREATE_LINK)\b/, 'registry write access'],
  [/\b(?:GENERIC_WRITE|GENERIC_ALL|FILE_WRITE_DATA|FILE_APPEND_DATA|FILE_WRITE_ATTRIBUTES|FILE_WRITE_EA|FILE_GENERIC_WRITE|FILE_ALL_ACCESS|WRITE_DAC|WRITE_OWNER|FILE_FLAG_DELETE_ON_CLOSE|FILE_FLAG_BACKUP_SEMANTICS|CREATE_ALWAYS|OPEN_ALWAYS|TRUNCATE_EXISTING)\b/, 'write access or backup semantics'],
  [/\.(?:create|truncate|append)\(\s*true\s*\)/, 'open-or-create / overwrite / append semantics'],
  [/\b(?:DeleteFile|MoveFile|CopyFile|ReplaceFile|CreateDirectory|RemoveDirectory|CreateHardLink|CreateSymbolicLink|SetFileTime|SetFileAttributes|SetFileInformationByHandle|SetEndOfFile|WriteFile|DeviceIoControl|SetFileShortName|EncryptFile|DecryptFile)\w*/, 'Win32 file modification'],
  [/\b(?:EvtClearLog|EvtExportLog|EvtArchiveExportedLog|EvtSetChannelConfigProperty|EvtSaveChannelConfig|EvtSubscribe|ClearEventLog|BackupEventLog|ReportEvent|RegisterEventSource)\w*/, 'event log change or write'],
  [/\b(?:RegisterTask|RegisterTaskDefinition|DeleteTask|CreateFolder|DeleteFolder|SetSecurityDescriptor|put_Enabled|RunEx)\b/, 'task scheduler change'],
  [/\b(?:ExecMethod|PutInstance|DeleteInstance|PutClass|DeleteClass|ExecNotificationQuery)\w*/, 'WMI change or subscription'],
  [/\b(?:CreateService|ChangeServiceConfig|DeleteService|StartService|ControlService|SetServiceStatus)\w*/, 'service change'],
  [/\b(?:AdjustTokenPrivileges|SetTokenInformation|ImpersonateLoggedOnUser|LogonUser)\w*/, 'privilege or identity change'],
  [/\bCommand::new\b|\bstd::process::Command\b|\b(?:CreateProcess|ShellExecute|WinExec)\w*/, 'starting programs'],
  [/\b(?:WNetAddConnection|WNetUseConnection|WNetCancelConnection|NetUseAdd)\w*/, 'network drive mapping'],
  [/\b(?:CoCreateInstanceEx|EvtOpenSession|RegConnectRegistry|OpenSCManager|OpenService|CoCreateInstanceFromApp)\w*/, 'remote or service-manager connection'],
  [/\bMsi(?:OpenDatabase|Database|Install|Configure|Reinstall|ApplyPatch|ApplyMultiplePatches|SetProperty|SetComponentState|SetFeatureState|SourceList|Advertise|ProvideComponent|ProcessAdvertiseScript|RemovePatches|SetInternalUI|EnableLog|DoAction|Sequence|RecordSet|ViewModify|CreateTransformSummaryInfo)\w*/, 'msi.dll (MSI packages are parsed read-only instead)'],
  [/\b(?:SystemParametersInfo|SetComputerName|SetEnvironmentVariable|set_var|remove_var)\w*/, 'system or environment change'],
  [/\btempfile::/, 'temporary files'],
];

// 2. Only in output.rs.
const FORBIDDEN_OUTSIDE_WRITER = [
  [/\bfs::(?:write|remove_file|remove_dir|remove_dir_all|rename|copy|create_dir|create_dir_all|set_permissions|hard_link|soft_link)\b/, 'file system change'],
  [/\bFile::create(?:_new)?\b/, 'file creation'],
  [/\.(?:write|create_new)\(\s*true\s*\)/, 'write access'],
  [/\b(?:symlink|symlink_file|symlink_dir|set_len|set_modified|set_times|set_readonly|sync_all|sync_data)\b|\bFileTimes\b/, 'file change'],
];

// 3. Only in read_only.rs and output.rs.
const OPEN_OPTIONS = /\bOpenOptions\b|\bFile::options\b/;

/** Checks one source file (path relative to the repository root); returns the violations. */
function checkSource(name, source) {
  const problems = [];
  const lines = withoutTestModules(stripRust(source)).split('\n');
  lines.forEach((line, index) => {
    const where = `${name}:${index + 1}`;
    for (const [pattern, what] of FORBIDDEN_ALWAYS) {
      if (pattern.test(line)) problems.push(`${where}: ${what} is forbidden in the collector (${line.trim()})`);
    }
    if (name !== WRITER) {
      for (const [pattern, what] of FORBIDDEN_OUTSIDE_WRITER) {
        if (pattern.test(line)) problems.push(`${where}: ${what} – only ${WRITER} may write (${line.trim()})`);
      }
    }
    if (OPEN_OPTIONS.test(line) && name !== READER && name !== WRITER) {
      problems.push(`${where}: OpenOptions only in ${READER} (reading) and ${WRITER} (the result)`);
    }
    if (/\bunsafe\b/.test(line) && !name.startsWith(UNSAFE_DIR)) {
      problems.push(`${where}: unsafe code only in ${UNSAFE_DIR}`);
    }
  });
  if (name === WRITER && !/\.create_new\(\s*true\s*\)/.test(stripRust(source))) {
    problems.push(`${WRITER}: the result file must be created with create_new(true) (never overwrite)`);
  }
  return problems;
}

// Positive control: the check must catch known violations and accept known-good code.
if (process.argv.includes('--self-test')) {
  const cases = [
    ['crates/vbs-collector/src/engine.rs', 'fn f() { std::fs::write("x", b"").ok(); }', true],
    ['crates/vbs-collector/src/walk.rs', 'fn f() { OpenOptions::new().write(true).open(p); }', true],
    ['crates/vbs-collector/src/platform/windows/registry.rs', 'unsafe { RegSetValueExW(k, n, 0, 1, d, l) };', true],
    ['crates/vbs-collector/src/platform/windows/registry.rs', 'let s = RegOpenKeyExW(root, p, 0, KEY_ALL_ACCESS, &mut k);', true],
    ['crates/vbs-collector/src/modules/tasks.rs', 'service.RegisterTaskDefinition(path, def)?;', true],
    ['crates/vbs-collector/src/modules/logs.rs', 'unsafe { EvtClearLog(0, channel, null(), 0) };', true],
    ['crates/vbs-collector/src/modules/wmi.rs', 'services.ExecMethod(path, name)?;', true],
    ['crates/vbs-collector/src/platform/windows/eventlog.rs', 'let s = unsafe { EvtOpenSession(1, login, 0, 0) };', true],
    ['crates/vbs-collector/src/platform/windows/wmi.rs', 'CoCreateInstanceEx(&clsid, None, ctx, Some(&server), &mut qi)?;', true],
    ['crates/vbs-collector/src/platform/windows/wmi.rs', 'services.ExecQuery(&wql, &query, flags, None)?;', false],
    ['crates/vbs-collector/src/engine.rs', 'fn f() { std::process::Command::new("cmd"); }', true],
    ['crates/vbs-core/src/container.rs', 'fn f() { std::fs::File::create("x"); }', true],
    ['crates/vbs-collector/src/output.rs', 'OpenOptions::new().write(true).create(true).open(p)', true],
    ['crates/vbs-collector/src/engine.rs', '#[cfg(test)]\nmod tests { fn t() {} }\nfn f() { std::fs::remove_file("x").ok(); }', true],
    ['crates/vbs-collector/src/read_only.rs', 'fn open(p: &Path) { OpenOptions::new().read(true).open(p); }', false],
    ['crates/vbs-collector/src/walk.rs', '#[cfg(test)]\nmod tests { fn t() { std::fs::write("fixture", "x").unwrap(); } }', false],
    ['crates/vbs-collector/src/walk.rs', '// never calls RegSetValueExW or fs::write\nlet s = "RegDeleteKeyW";', false],
    ['crates/vbs-collector/src/output.rs', 'OpenOptions::new().write(true).create_new(true).open(p)', false],
  ];
  let failed = 0;
  for (const [name, source, violation] of cases) {
    const found = checkSource(name, source).length > 0;
    if (found !== violation) {
      failed++;
      console.error(`✗ self-test: ${name}: ${JSON.stringify(source)} – expected ${violation ? 'a violation' : 'no violation'}`);
    }
  }
  if (failed) process.exit(1);
  console.log(`✓ read-only check self-test: ${cases.length} cases recognised correctly`);
  process.exit(0);
}

for (const dir of SCOPE) {
  for (const path of walk(join(root, dir))) {
    if (path.endsWith('.rs')) errors.push(...checkSource(rel(path), readFileSync(path, 'utf8')));
  }
}

// 5. Dependencies of the collector.
const FORBIDDEN_DEPENDENCIES = ['tempfile', 'winreg', 'fs_extra', 'remove_dir_all', 'filetime', 'trash'];
let table = '';
for (const line of readFileSync(join(root, 'crates/vbs-collector/Cargo.toml'), 'utf8').split('\n')) {
  const header = line.match(/^\s*\[([^\]]+)\]/);
  if (header) {
    table = header[1];
    continue;
  }
  if (!/dependencies/.test(table) || /dev-dependencies/.test(table)) continue;
  const dependency = line.match(/^\s*([A-Za-z0-9_-]+)\s*=/)?.[1];
  if (dependency && FORBIDDEN_DEPENDENCIES.includes(dependency)) {
    errors.push(`crates/vbs-collector/Cargo.toml: "${dependency}" must not be a dependency of the collector`);
  }
}

if (errors.length) {
  for (const error of errors) console.error(`✗ ${error}`);
  console.error(`\n${errors.length} read-only violation(s).`);
  process.exit(1);
}
console.log(`✓ read-only policy: the collector reads only; ${WRITER} is the single place that writes`);
