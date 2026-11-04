"""Shared input/output helpers for the repository's Claude Code hooks.

Hooks read one JSON event on stdin and answer with JSON on stdout. They are
deterministic guards for rules in AGENTS.md; CI remains the authoritative gate.
"""
import json
import sys
from pathlib import Path


def event():
    return json.load(sys.stdin)


def absolute(payload, path):
    path = Path(path)
    if not path.is_absolute():
        path = Path(payload.get("cwd") or ".") / path
    return path.resolve()


def relative(payload, path):
    """Path relative to the checkout (repository or worktree) that contains it."""
    if not path:
        return None
    path = absolute(payload, path)
    top = git_top(path.parent)
    return path.relative_to(top).as_posix() if top else None


def git_top(directory):
    current = Path(directory)
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    return None


def resulting_text(payload):
    """Text a file will have after an Edit/Write/MultiEdit, or None if unknown."""
    tool = payload.get("tool_name")
    data = payload.get("tool_input") or {}
    if tool == "Write":
        return data.get("content")
    path = data.get("file_path")
    try:
        text = absolute(payload, path).read_text(encoding="utf-8") if path else ""
    except FileNotFoundError:
        text = ""
    edits = data.get("edits") if tool == "MultiEdit" else [data] if tool == "Edit" else None
    if edits is None:
        return None
    for edit in edits:
        old, new = edit.get("old_string", ""), edit.get("new_string", "")
        if old not in text:
            return None
        text = text.replace(old, new) if edit.get("replace_all") else text.replace(old, new, 1)
    return text


def applied_edits(payload):
    """Exact fragments of a successful Edit/MultiEdit; Write has no old text."""
    data = payload.get("tool_input") or {}
    if payload.get("tool_name") == "Edit":
        return [data]
    if payload.get("tool_name") == "MultiEdit":
        return data.get("edits")
    return None


def previous_text(payload, text):
    """Undo only unambiguous replacements to recover a post-edit predecessor.

    Repeated replacement text, deletions and replace_all are ambiguous without
    a pre-edit snapshot. Leave them unknown rather than guessing the edit site.
    """
    edits = applied_edits(payload)
    if not edits:
        return None
    for edit in reversed(edits):
        old, new = edit.get("old_string"), edit.get("new_string")
        if old is None or not new or edit.get("replace_all") or text.count(new) != 1:
            return None
        text = text.replace(new, old, 1)
    return text


def deny(event_name, reason):
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }))
    sys.exit(0)


def ask(event_name, reason):
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "permissionDecision": "ask",
            "permissionDecisionReason": reason,
        }
    }))
    sys.exit(0)


def context(event_name, text):
    print(json.dumps({
        "hookSpecificOutput": {"hookEventName": event_name, "additionalContext": text}
    }))
    sys.exit(0)
