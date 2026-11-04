#!/usr/bin/env bash
# Optional command coordination; see CONTRIBUTING.md, Cargo resources.
# Caller CARGO_BUILD_JOBS and Cargo configuration are preserved.
# Default: linked worktrees share a lock in their Git common directory.
# NOCTERM_CARGO_LOCK selects another path, or "off" disables the lock.
# Usage: .claude/hooks/cargo-serial.sh cargo test -p nocterm-ssh
# Only the wrapper holds the lock; children/background jobs do not inherit it.
set -uo pipefail
if [ "$#" -eq 0 ]; then
  echo "usage: $0 cargo <arguments>" >&2
  exit 64
fi
lock="${NOCTERM_CARGO_LOCK:-}"
if [ "$lock" = off ]; then
  exec "$@"
fi
if ! command -v flock >/dev/null 2>&1; then
  echo "cargo-serial: locking requires flock; install it, run Cargo directly, or set NOCTERM_CARGO_LOCK=off" >&2
  exit 69
fi
if [ -z "$lock" ]; then
  common=$(git rev-parse --git-common-dir) || {
    echo "cargo-serial: outside a Git repository; set NOCTERM_CARGO_LOCK to a path or off" >&2
    exit 64
  }
  common=$(CDPATH= cd -- "$common" && pwd -P) || exit 75
  lock="$common/nocterm-cargo.lock"
fi
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
exec 9>>"$lock" || exit 75
flock -n 9 || { waiting; flock 9 || exit 75; }
run "$@"
