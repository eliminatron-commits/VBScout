#!/usr/bin/env node
// Analyses an `strace -f -y` log for operations that change the file system or start programs.
// Used by scripts/readonly/linux.sh (Definition of Done #1).
//
//   node scripts/readonly/analyze-strace.mjs <trace.log> <allowed-result-file> [--program <path>]
//
// Allowed: opening/creating/writing the result file, writing to stdout/stderr, and the single
// execve of the program itself. Everything else – opening any other file for writing, creating
// directories, renaming, deleting, linking, truncating, changing modes/owners/times/xattrs,
// writing to any other descriptor, starting programs – is a violation. Prints a summary and every
// violation; exit code 1 if there is any.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const [logPath, allowedArg, ...rest] = process.argv.slice(2);
if (!logPath || !allowedArg) {
  console.error('usage: analyze-strace.mjs <trace.log> <allowed-result-file> [--program <path>]');
  process.exit(2);
}
const allowed = resolve(allowedArg);
const program = rest[0] === '--program' ? resolve(rest[1]) : null;

const WRITE_FLAGS = /\bO_(WRONLY|RDWR|CREAT|TRUNC|APPEND|TMPFILE)\b/;
const OPEN = new Set(['open', 'openat', 'openat2', 'creat']);
const PATH_CHANGES = new Set([
  'mkdir', 'mkdirat', 'rmdir', 'rename', 'renameat', 'renameat2', 'unlink', 'unlinkat', 'link', 'linkat',
  'symlink', 'symlinkat', 'truncate', 'chmod', 'fchmodat', 'fchmodat2', 'chown', 'lchown', 'fchownat', 'utime',
  'utimes', 'utimensat', 'futimesat', 'setxattr', 'lsetxattr', 'removexattr', 'lremovexattr', 'mknod', 'mknodat',
]);
const FD_CHANGES = new Set(['ftruncate', 'fallocate', 'fchmod', 'fchown', 'fsetxattr', 'fremovexattr']);
const WRITES = new Set(['write', 'pwrite64', 'writev', 'pwritev', 'pwritev2', 'sendfile', 'copy_file_range', 'splice']);

const violations = [];
const counts = { lines: 0, opens: 0, writeOpensOfResult: 0, writesOfResult: 0, consoleWrites: 0, execs: 0 };

/** First quoted string argument (strace escapes quotes inside). */
const firstPath = (args) => args.match(/"((?:[^"\\]|\\.)*)"/)?.[1] ?? null;
/** Path decoration of the n-th descriptor argument, e.g. `3</tmp/x>`. */
const fdPath = (args) => args.match(/^\s*(-?\d+)(?:<([^>]*)>)?/);

for (const line of readFileSync(logPath, 'utf8').split('\n')) {
  const call = line.match(/^(?:\[pid\s+)?(\d+)\]?\s+(\w+)\((.*)$/);
  if (!call) continue;
  counts.lines++;
  const [, , name, args] = call;
  if (/\)\s+=\s+-1\s/.test(line) && !OPEN.has(name) && name !== 'execve') continue; // failed calls change nothing

  if (OPEN.has(name)) {
    counts.opens++;
    const flags = name === 'creat' ? 'O_CREAT|O_WRONLY|O_TRUNC' : args;
    if (!WRITE_FLAGS.test(flags)) continue;
    const path = firstPath(args);
    if (path && resolve(path) === allowed) counts.writeOpensOfResult++;
    else violations.push(`opened for writing: ${line.trim()}`);
  } else if (PATH_CHANGES.has(name)) {
    violations.push(`file system change: ${line.trim()}`);
  } else if (FD_CHANGES.has(name) || WRITES.has(name)) {
    const [, fd, target] = fdPath(args) ?? [];
    if (WRITES.has(name) && (fd === '1' || fd === '2')) counts.consoleWrites++;
    else if (target && resolve(target) === allowed) counts.writesOfResult++;
    else violations.push(`${WRITES.has(name) ? 'write' : 'change'} via descriptor: ${line.trim()}`);
  } else if (name === 'execve') {
    counts.execs++;
    const path = firstPath(args);
    if (counts.execs > 1 || (program && path && resolve(path) !== program)) {
      violations.push(`program started: ${line.trim()}`);
    }
  }
}

console.log(
  `traced calls: ${counts.lines}, opens: ${counts.opens}, result opened for writing: ${counts.writeOpensOfResult}, ` +
    `writes to the result: ${counts.writesOfResult}, console writes: ${counts.consoleWrites}, programs started: ${counts.execs}`,
);
for (const violation of violations) console.log(`VIOLATION ${violation}`);
console.log(`violations: ${violations.length}`);
process.exit(violations.length ? 1 : 0);
