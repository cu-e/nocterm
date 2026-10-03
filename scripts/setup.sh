#!/usr/bin/env bash
# One-time setup after cloning: enables the versioned git hooks.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
git config core.hooksPath .githooks
echo "git hooks enabled (.githooks)"
