#!/usr/bin/env python3
"""PostToolUse signals after an edit, before a limit or review rejects it.

Adds context for the agent; it never blocks. The signals mirror the
early warnings of `cargo xtask architecture` and the design-review skill:
- a Rust file at or above 640 lines needs a split designed by responsibility;
- a new Rust file holding only `impl` blocks moves methods without an owner;
- a catch-all module name hides a responsibility;
- a new crate manifest starts the crate-new workflow.
"""
import re
import sys
from pathlib import Path

import hooklib

EVENT = "PostToolUse"
WARN_LINES = 640
LIMIT = 800
ROOTS = ("src/", "crates/", "xtask/")
GENERIC = {"utils", "util", "helpers", "helper", "misc", "common", "stuff", "extra", "ext"}
DEFINITION = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+\w", re.MULTILINE)
IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b", re.MULTILINE)


def signals(path, text, created):
    found = []
    if path.endswith(".rs") and path.startswith(ROOTS):
        lines = text.count("\n") + (0 if text.endswith("\n") or not text else 1)
        if lines >= WARN_LINES:
            found.append(
                f"{path} has {lines} lines (warn at {WARN_LINES}, limit {LIMIT}). Before it grows, run the "
                "responsibility-map skill on its main type and extract an owner with private state; "
                "do not cut the file at the limit or move methods of the same type to another file."
            )
        stem = Path(path).stem
        if created and stem in GENERIC:
            found.append(
                f"{path} is a catch-all module name. Name the module after the responsibility its type owns "
                "(design-review skill, anti-patterns)."
            )
        if created and IMPL.search(text) and not DEFINITION.search(text) and not path.endswith(("tests.rs", "/mod.rs")) and "/tests/" not in path:
            found.append(
                f"{path} only implements types declared elsewhere. Moving methods of a parent type to a new "
                "file is file slicing, not an owner extraction: use the owner-extract skill (AGENTS.md design rules 1 and 7)."
            )
    if created and re.fullmatch(r"crates/[^/]+/Cargo\.toml", path):
        found.append(
            f"{path} starts a new crate. Follow the crate-new skill: placement, ownership card, layer metadata, "
            "`[lints] workspace = true`, typed errors and `cargo xtask docs`."
        )
    return found


def main():
    payload = hooklib.event()
    data = payload.get("tool_input") or {}
    path = hooklib.relative(payload, data.get("file_path"))
    if path is None:
        return
    response = payload.get("tool_response") or {}
    created = isinstance(response, dict) and response.get("type") == "create"
    try:
        text = hooklib.absolute(payload, data.get("file_path")).read_text(encoding="utf-8")
    except (FileNotFoundError, UnicodeDecodeError):
        return
    found = signals(path, text, created)
    if found:
        hooklib.context(EVENT, "\n".join(found))


if __name__ == "__main__":
    main()
    sys.exit(0)
