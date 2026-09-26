#!/usr/bin/env bash
# Read-only test, Linux (Definition of Done #1) – system-call level.
#
# Runs the collector under strace and records every system call of its whole process tree that can
# change the file system or start a program (open for writing, create, rename, delete, link,
# truncate, chmod/chown, times, xattrs, writes to descriptors, execve). Passes only if the collector
# exits with 0, the only file it opened for writing is its result file, it changed nothing else and
# started no program, the scanned folder is unchanged, and its working and temp folders stay empty.
#
# A positive control (a shell that writes a file and creates a directory, traced the same way) proves
# that the trace and its analysis catch such changes – otherwise "no changes" would be meaningless.
#
# usage: scripts/readonly/linux.sh <collector-binary> <folder-to-scan> [output-dir]
# needs: strace, node
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <collector-binary> <folder-to-scan> [output-dir]" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="$(realpath "$1")"
scan="$(realpath "$2")"
mkdir -p "${3:-readonly-out}"
out="$(realpath "${3:-readonly-out}")"
analyze="$root/scripts/readonly/analyze-strace.mjs"
extension="$(node -p "require('$root/product.json').resultFile.extension")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/cwd" "$work/temp" "$work/results"
result="$work/results/result.$extension"

syscalls="open,openat,creat,mkdir,mkdirat,rmdir,rename,renameat,renameat2,unlink,unlinkat,link,linkat,symlink,symlinkat"
syscalls+=",truncate,ftruncate,chmod,fchmod,fchmodat,chown,fchown,lchown,fchownat,utime,utimes,utimensat,futimesat"
syscalls+=",setxattr,lsetxattr,fsetxattr,removexattr,lremovexattr,fremovexattr,mknod,mknodat,fallocate"
syscalls+=",write,pwrite64,writev,pwritev,pwritev2,sendfile,copy_file_range,splice,execve"
for optional in openat2 fchmodat2; do # newer calls, only if this strace knows them
  if strace -qq -e trace="$optional" -o /dev/null true 2>/dev/null; then syscalls+=",$optional"; fi
done
trace() { strace -f -qq -y -e trace="$syscalls" -e signal=none "$@"; }

snapshot() { # path, type, size, mode, mtime, content hash of every entry – links recorded, not followed
  (cd "$1" && find . -printf '%p\t%y\t%s\t%m\t%T@\t%l\n' | sort
    find . -type f -print0 | sort -z | xargs -0 -r sha256sum)
}

# 1) Positive control.
control_log="$out/strace-control.log"
trace -o "$control_log" sh -c "echo x > '$work/control.txt' && mkdir '$work/control-dir'" || true
set +e
control_output="$(node "$analyze" "$control_log" "$work/nothing-is-allowed")"
set -e
control_seen="$(grep -c '^VIOLATION' <<<"$control_output" || true)"

# 2) The collector: all sources, file scan restricted to the given folder.
before="$(snapshot "$scan")"
set +e
(cd "$work/cwd" && TMPDIR="$work/temp" TEMP="$work/temp" TMP="$work/temp" \
  trace -o "$out/strace-collector.log" "$bin" --path "$scan" --out "$result" --quiet) \
  >"$out/collector-output.log" 2>&1
status=$?
analysis="$(node "$analyze" "$out/strace-collector.log" "$result" --program "$bin")"
analysis_status=$?
set -e
after="$(snapshot "$scan")"

results_listing="$(ls -A "$work/results")"
cwd_listing="$(ls -A "$work/cwd")"
temp_listing="$(ls -A "$work/temp")"
unchanged=$([[ "$before" == "$after" ]] && echo yes || echo NO)

{
  echo "Read-only test (Linux) – $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "collector:              $bin"
  echo "scanned folder:         $scan"
  echo "exit code:              $status"
  echo "trace analysis:         $analysis"
  echo "scanned folder unchanged: $unchanged"
  echo "output folder:          ${results_listing:-<empty>}"
  echo "working folder:         ${cwd_listing:-<empty>}"
  echo "temp folder:            ${temp_listing:-<empty>}"
  echo "positive control seen:  $control_seen change(s)"
  if [[ "$unchanged" != yes ]]; then
    echo
    echo "Differences in the scanned folder:"
    diff <(echo "$before") <(echo "$after") || true
  fi
} | tee "$out/readonly-linux.txt"

fail() { echo "FAIL: $1" >&2; exit 1; }
[[ $control_seen -ge 2 ]] || fail "the positive control was not detected – the monitor does not work"
[[ $status -eq 0 ]] || fail "the collector exited with $status (see $out/collector-output.log)"
[[ $analysis_status -eq 0 ]] || fail "the collector changed the file system or started a program"
[[ "$unchanged" == yes ]] || fail "the scanned folder changed"
[[ "$results_listing" == "$(basename "$result")" ]] || fail "the output folder must contain exactly the result file"
[[ -z "$cwd_listing" && -z "$temp_listing" ]] || fail "the collector left files in its working or temp folder"
echo "PASS: the collector changed nothing and wrote only its result file"
