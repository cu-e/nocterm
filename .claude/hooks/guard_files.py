#!/usr/bin/env python3
"""PreToolUse guard for file edits that AGENTS.md forbids or ratchets.

- Release files are owned by release automation and are never edited.
- Debt inventories (`xtask/baseline.toml`, `xtask/oversized-files.toml`) may
  only shrink: new entries and raised counts are denied.
- DTO exemptions in `xtask/architecture-policy.toml` need design review, so
  adding one asks the user.
- Pipeline stages keep to their role (AGENTS.md, Pipeline): `architect` and
  `reviewer` write only their plan files; `tester` edits tests, and editing
  anything else asks the user.
"""
import sys
import tomllib

import hooklib

RELEASE_FILES = {"CHANGELOG.md", "version.txt", ".release-please-manifest.json"}
RATCHETS = {"xtask/baseline.toml", "xtask/oversized-files.toml"}
POLICY = "xtask/architecture-policy.toml"
EVENT = "PreToolUse"
PLANS = ".claude/plans/"
PLAN_ONLY = {"architect", "reviewer"}
TEST_DIRECTORIES = {"tests", "testing", "test_support", "gui_tests"}


def is_test_file(path):
    parts = path.split("/")
    name = parts[-1]
    return (
        any(part in TEST_DIRECTORIES for part in parts[:-1])
        or name in {"tests.rs", "test_support.rs"}
        or name.endswith("_tests.rs")
        or name.startswith(("test-", "test_"))
    )


def check_role(agent, path):
    """Stage-specific write limits; `path` is None outside any checkout."""
    if agent in PLAN_ONLY and not (path or "").startswith(PLANS):
        hooklib.deny(EVENT, f"The {agent} stage does not write code; write its output under {PLANS}<slug>/ (AGENTS.md, Pipeline).")
    if agent == "tester" and path and not path.startswith(PLANS) and not is_test_file(path):
        hooklib.ask(EVENT, f"The tester stage changes tests only; {path} looks like production code (AGENTS.md, Pipeline).")


def flatten(document, prefix=""):
    values = {}
    for key, value in document.items():
        name = f"{prefix}{key}"
        if isinstance(value, dict):
            values.update(flatten(value, f"{name}."))
        else:
            values[name] = value
    return values


def growth(before, after):
    """Entries that are new or larger in `after`."""
    old, new = flatten(before), flatten(after)
    grown = []
    for name, value in new.items():
        previous = old.get(name)
        if previous is None:
            grown.append(f"{name} = {value} (new entry)")
        elif isinstance(value, (int, float)) and isinstance(previous, (int, float)) and value > previous:
            grown.append(f"{name}: {previous} -> {value}")
    return grown


def parse(text):
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return None


def main():
    payload = hooklib.event()
    data = payload.get("tool_input") or {}
    path = hooklib.relative(payload, data.get("file_path") or data.get("notebook_path"))
    check_role(payload.get("agent_type"), path)
    if path is None:
        return
    if path in RELEASE_FILES:
        hooklib.deny(EVENT, f"{path} is maintained by release automation; never edit it (AGENTS.md, Hard rules; docs/RELEASING.md).")
    if path not in RATCHETS and path != POLICY:
        return
    after_text = hooklib.resulting_text(payload)
    if after_text is None:
        return
    try:
        before_text = hooklib.absolute(payload, data.get("file_path")).read_text(encoding="utf-8")
    except FileNotFoundError:
        before_text = ""
    before, after = parse(before_text), parse(after_text)
    if after is None:
        hooklib.deny(EVENT, f"{path} would no longer be valid TOML.")
    if before is None:
        return
    grown = growth(before, after)
    if not grown:
        return
    if path == POLICY:
        hooklib.ask(EVENT, "Adding a plain-data exemption needs design review (docs/architecture/guardrails.md): " + "; ".join(grown))
    hooklib.deny(
        EVENT,
        f"{path} is an inventory that may only shrink (ADR 0012, AGENTS.md design rule 6). "
        "Remove the new debt instead of recording it: " + "; ".join(grown),
    )


if __name__ == "__main__":
    main()
    sys.exit(0)
