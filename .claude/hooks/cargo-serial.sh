#!/usr/bin/env bash
# Runs one Cargo command with CARGO_BUILD_JOBS=2 under one lock shared by every
# agent of this user on the machine (AGENTS.md, Hard rules). The lock is held
# by this script only: the command and anything it leaves running in the
# background do not inherit it.
# Usage: .claude/hooks/cargo-serial.sh cargo test -p nocterm-ssh
# NOCTERM_CARGO_LOCK overrides the lock path (tests only).
set -uo pipefail
if [ "$#" -eq 0 ]; then
  echo "usage: $0 cargo <arguments>" >&2
  exit 64
fi
export CARGO_BUILD_JOBS=2
lock="${NOCTERM_CARGO_LOCK:-/tmp/nocterm-cargo-$(id -u).lock}"
waiting() { echo "cargo-serial: waiting for another Cargo command to finish" >&2; }
# Runs the command as a child that a signal to this script reaches too, and
# returns its status. A background job of a non-interactive shell starts with
# SIGINT and SIGQUIT ignored; the subshell restores them so Ctrl-C stops the
# command and everything it started.
run() {
  ( trap - INT QUIT; exec "$@" ) 9>&- <&0 &
  child=$!
  trap 'kill -s TERM "$child" 2>/dev/null' TERM INT HUP
  status=0
  while :; do
    wait "$child" && status=0 || status=$?
    kill -0 "$child" 2>/dev/null || break
  done
  return "$status"
}
if command -v flock >/dev/null 2>&1; then
  exec 9>>"$lock" || exit 75
  flock -n 9 || { waiting; flock 9 || exit 75; }
  run "$@"
  exit $?
fi
# Portable fallback without flock: an atomic directory, removed on exit.
until mkdir "$lock.d" 2>/dev/null; do
  [ -n "${announced:-}" ] || { waiting; announced=1; }
  sleep 2
done
trap 'rmdir "$lock.d"' EXIT
run "$@"
