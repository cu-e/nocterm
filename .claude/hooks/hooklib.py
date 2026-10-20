"""Shared input/output helpers for the repository's Claude Code hooks.

Hooks read one JSON event on stdin and answer with JSON on stdout. They are
deterministic guards for rules in AGENTS.md; CI remains the authoritative gate.
"""
import json
import os
import sys
from pathlib import Path


def event():
    return json.load(sys.stdin)


def project_dir(payload):
    return Path(os.environ.get("CLAUDE_PROJECT_DIR") or payload.get("cwd") or ".").resolve()


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
