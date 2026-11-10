#!/usr/bin/env python3
"""PreToolUse guard for file edits that AGENTS.md forbids or ratchets.

- Release files are owned by release automation and are never edited.
- Debt inventories (`xtask/baseline.toml`, `xtask/oversized-files.toml`) may
  only shrink: new entries and raised counts are denied.
- DTO additions and other architecture policy changes need their own design
  review; isolation changes include arrays and removed restrictions.
- Delegated agents keep to their explicit role (AGENTS.md, Workflow): `architect` and
  `reviewer` write only their plan files; `tester` edits tests, and editing
  anything else is handed back to the implementer.
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
        hooklib.deny(EVENT, f"The {agent} stage does not write code; write its output under {PLANS}<slug>/ (AGENTS.md, Workflow; .claude/agents/).")
    if agent == "tester" and path and not path.startswith(PLANS) and not is_test_file(path):
        hooklib.deny(
            EVENT,
            f"The tester stage does not edit production files: {path}. Send the proposed change "
            "to the implementer, including changes confined to an inline test module. "
            "This is a role handoff, not a request for user approval (.claude/agents/tester.md).",
        )


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


def policy_review(before, after):
    """DTO removals shrink exemptions; all other policy changes need review.

    Isolation lists may be allowlists or denylists, so removals cannot safely
    be treated as reductions of authority. Compare parsed values, not TOML text.
    """
    reasons = []
    old_dto, new_dto = before.get("dto", {}), after.get("dto", {})
    if old_dto != new_dto:
        if not isinstance(old_dto, dict) or not isinstance(new_dto, dict):
            reasons.append("Changing the plain-data exemption structure needs design review")
        else:
            changed = [key for key, value in new_dto.items() if old_dto.get(key) != value]
            if changed:
                reasons.append("Adding or changing plain-data exemptions needs design review: " + ", ".join(sorted(changed)))
    sections = sorted(
        key for key in before.keys() | after.keys()
        if key != "dto" and before.get(key) != after.get(key)
    )
    if sections:
        reasons.append(
            "Changing architecture policy sections needs design review, including isolation "
            "permissions and restrictions: " + ", ".join(sections)
        )
    return reasons


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
    if path == POLICY:
        reasons = policy_review(before, after)
        if reasons:
            hooklib.ask(EVENT, "; ".join(reasons) + " (docs/architecture/guardrails.md).")
        return
    grown = growth(before, after)
    if not grown:
        return
    hooklib.deny(
        EVENT,
        f"{path} is an inventory that may only shrink (ADR 0012, AGENTS.md design rule 6). "
        "Remove the new debt instead of recording it: " + "; ".join(grown),
    )


if __name__ == "__main__":
    main()
    sys.exit(0)
