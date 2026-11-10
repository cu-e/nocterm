#!/usr/bin/env python3
"""PostToolUse signals after an edit, before a limit or review rejects it.

Adds context for the agent; it never blocks. The signals mirror the
early warnings of `cargo xtask architecture` and the design-review skill:
- crossing 640 lines or another 40-line band invites responsibility inspection;
- a new Rust file holding only `impl` blocks moves methods without an owner;
- a catch-all module name hides a responsibility;
- a new crate manifest starts the crate-new workflow;
- a plugin declaration, the plugin model or the plugin catalog follows the plugin-api skill.
"""
import difflib
import re
import sys
from pathlib import Path

import hooklib

EVENT = "PostToolUse"
WARN_LINES = 640
LIMIT = 800
WARN_STEP = 40
ROOTS = ("src/", "crates/", "xtask/")
GENERIC = {"utils", "util", "helpers", "helper", "misc", "common", "stuff", "extra", "ext"}
DEFINITION = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+\w", re.MULTILINE)
PLUGIN_CONTRACT = re.compile(r"(?:^|/)plugin\.rs$|^crates/plugin-(?:model|api|abi|ports)/")
PLUGIN_REGISTRATION = re.compile(
    r"\b(?:PLUGIN|PLUGINS|CORE|PluginCatalog|SystemPlugin|SystemContribution|"
    r"register_panel|register_widget|nocterm_plugin_api)\b|\bplugin::catalog\b"
)
IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b", re.MULTILINE)


def line_count(text):
    return text.count("\n") + (0 if text.endswith("\n") or not text else 1)


def size_warning(text, payload):
    lines = line_count(text)
    if lines > LIMIT:
        return True
    if lines < WARN_LINES:
        return False
    before = hooklib.previous_text(payload, text)
    if before is not None:
        return lines // WARN_STEP > line_count(before) // WARN_STEP
    edits = hooklib.applied_edits(payload)
    if edits and all(not edit.get("replace_all") for edit in edits):
        # A deletion or ambiguous replacement can still prove there was no
        # line growth. At most one trailing unterminated line can be added.
        delta = sum(
            edit.get("new_string", "").count("\n") - edit.get("old_string", "").count("\n")
            for edit in edits
        )
        same_endings = all(
            edit.get("old_string") and edit.get("new_string")
            and edit["old_string"].endswith("\n") == edit["new_string"].endswith("\n")
            for edit in edits
        )
        if delta < 0 or (delta == 0 and same_endings):
            return False
    # Write supplies no predecessor. Unknown edits remain conservative; no
    # persistent session cache is needed to suppress known unchanged/shrinking edits.
    return True


def changed_code(payload, text):
    edits = hooklib.applied_edits(payload)
    lines = []
    if edits is None:
        lines = text.splitlines()
    else:
        for edit in edits:
            before = edit.get("old_string", "").splitlines()
            after = edit.get("new_string", "").splitlines()
            for kind, start, end, new_start, new_end in difflib.SequenceMatcher(None, before, after).get_opcodes():
                if kind != "equal":
                    lines.extend(before[start:end])
                    lines.extend(after[new_start:new_end])
    # This is a routing hint, not a Rust parser or a replacement for review.
    return "\n".join(line for line in lines if line.strip() and not line.lstrip().startswith("//"))


def signals(path, text, created, payload):
    found = []
    if path.endswith(".rs") and path.startswith(ROOTS):
        lines = line_count(text)
        if size_warning(text, payload):
            status = "Hard limit exceeded" if lines > LIMIT else "Advisory size warning"
            instruction = (
                f"Reduce the file to at most {LIMIT} lines; split production code by responsibility. "
                if lines > LIMIT else
                "Inspect its responsibility and remaining room; use responsibility-map if separation is unclear. "
                "A warning alone does not require extraction. "
            )
            found.append(
                f"{status}: {path} has {lines} lines (advisory at {WARN_LINES}, hard limit {LIMIT}). "
                f"{instruction}If a split is needed, extract an owner, "
                "rather than moving methods of the same type to another file."
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
        changed = changed_code(payload, text)
        if changed and (PLUGIN_CONTRACT.search(path) or PLUGIN_REGISTRATION.search(changed)):
            found.append(
                f"{path} declares, models or composes plugins. Follow the plugin-api skill: system plugins are "
                "`pub static PLUGIN`, core owners declare `CORE`, registration goes through the workspace "
                "contract, and nothing is exposed to user plugins by a getter or a `pub` item."
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
    found = signals(path, text, created, payload)
    if found:
        hooklib.context(EVENT, "\n".join(found))


if __name__ == "__main__":
    main()
    sys.exit(0)
