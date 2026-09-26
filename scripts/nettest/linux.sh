#!/usr/bin/env bash
# Network block test, Linux (Definition of Done #2). Taken over from Stepwright and generalised,
# so it covers both the collector and the evaluation app.
#
# Runs a program inside an isolated network namespace (only a downed loopback interface, so every
# connection attempt is blocked) under strace, which logs connect/sendto/sendmsg of the whole process
# tree. Passes only if the program exits with 0 AND made zero attempts to reach an IPv4/IPv6 address.
# Unix-domain sockets (X11, D-Bus, WebKit IPC) are local IPC: logged, but not counted.
#
# A positive control (a deliberate connection attempt traced the same way) proves the logging works –
# otherwise "0" would be meaningless.
#
# usage: scripts/nettest/linux.sh [--gui] <output-dir> -- <program> [arguments…]
#   --gui  run under xvfb-run + dbus-run-session (the evaluation app's --smoke-test)
# needs: sudo (unshare), strace; with --gui also xvfb-run and dbus-run-session
set -euo pipefail

gui=0
if [[ "${1:-}" == "--gui" ]]; then
  gui=1
  shift
fi
if [[ $# -lt 3 || "$2" != "--" ]]; then
  echo "usage: $0 [--gui] <output-dir> -- <program> [arguments…]" >&2
  exit 2
fi
mkdir -p "$1"
out="$(realpath "$1")"
shift 2
bin="$(realpath "$1")"
shift
name="$(basename "$bin")"
log="$out/strace-$name.log"
control_log="$out/strace-control.log"
report="$out/nettest-linux-$name.txt"
user="$(id -un)"
rm -f "$log" "$control_log" "$out/output-$name.log"

isolated() {
  sudo unshare --net -- runuser -u "$user" -- "$@"
}

# 1) Positive control: an IPv4 connection attempt must show up in the log.
isolated strace -f -qq -e trace=connect -e signal=none -o "$control_log" \
  bash -c 'exec 3<>/dev/tcp/192.0.2.1/80' >/dev/null 2>&1 || true
control=$(grep -cE 'sa_family=AF_INET6?,' "$control_log" || true)

# 2) The program itself, blocked and traced. A headless GUI session gets a private session bus
#    (dbus-run-session) like a real desktop; bus-activated helpers run outside the traced tree.
traced=(strace -f -qq -e trace=connect,sendto,sendmsg,sendmmsg -e signal=none -o "$log" "$bin" "$@")
set +e
if [[ $gui -eq 1 ]]; then
  isolated env NO_AT_BRIDGE=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 \
    xvfb-run --auto-servernum --server-args="-screen 0 1280x800x24 -nolisten tcp" \
    dbus-run-session -- timeout --kill-after=15 600 "${traced[@]}" >"$out/output-$name.log" 2>&1
else
  isolated timeout --kill-after=15 1800 "${traced[@]}" >"$out/output-$name.log" 2>&1
fi
status=$?
set -e

touch "$log"
inet=$(grep -cE 'sa_family=AF_INET6?,' "$log" || true)
local_ipc=$(grep -c 'sa_family=AF_UNIX' "$log" || true)
syscalls=$(wc -l <"$log")

{
  echo "Network block test (Linux) – $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "program:                $bin $*"
  echo "exit code:              $status"
  echo "traced socket calls:    $syscalls"
  echo "  local IPC (AF_UNIX):  $local_ipc (not network)"
  echo "  IPv4/IPv6 attempts:   $inet"
  echo "positive control seen:  $control"
  if [[ $inet -gt 0 ]]; then
    echo
    echo "Network attempts:"
    grep -E 'sa_family=AF_INET6?,' "$log"
  fi
} | tee "$report"

if [[ $control -lt 1 ]]; then
  echo "FAIL: the positive control was not logged – the monitor does not work" >&2
  exit 1
fi
if [[ $status -ne 0 ]]; then
  echo "FAIL: $name exited with $status (output: $out/output-$name.log)" >&2
  exit 1
fi
if [[ $inet -gt 0 ]]; then
  echo "FAIL: $name attempted $inet network connection(s)" >&2
  exit 1
fi
echo "PASS: $name made zero network connections"
