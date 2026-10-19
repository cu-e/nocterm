#!/usr/bin/env python3
"""Check every incoming commit, including the first inventory bootstrap PR."""
import argparse
import ast
import subprocess
import tomllib
from pathlib import Path

# Existing mixed commits are debt fixed at the audited dev snapshot, never a moving target.
LEGACY_VENDOR_SNAPSHOT = "084626ce53248982a5c086a9b854f89d8ff7da2b"

METRICS = {
    "struct_fields", "pub_fields", "impl_spread", "globals", "global_reads",
    "quit_handlers", "string_errors", "domain_io", "domain_dependencies", "complexity_expects",
}


def git(*args, check=True):
    return subprocess.run(["git", *args], check=check, capture_output=True, text=True)


def inventory(revision):
    exists = git("ls-tree", "--name-only", revision, "xtask/baseline.toml").stdout.strip()
    if not exists:
        return None
    value = tomllib.loads(git("show", f"{revision}:xtask/baseline.toml").stdout)
    if not set(value).issubset(METRICS):
        raise ValueError(f"{revision}: unknown inventory metrics")
    for metric, entries in value.items():
        if not isinstance(entries, dict):
            raise ValueError(f"{revision}: invalid {metric} inventory")
        if any(not name.strip() or type(count) is not int or count <= 0 for name, count in entries.items()):
            raise ValueError(f"{revision}: invalid {metric} entry")
    return value


def no_growth(previous, current, revision):
    if previous is None:
        return
    if current is None:
        raise ValueError(f"{revision}: cannot remove architectural inventory")
    for metric, entries in current.items():
        for name, count in entries.items():
            old = previous.get(metric, {}).get(name, 0)
            if count > old:
                raise ValueError(f"{revision}: baseline increase {metric} {name}: {old} -> {count}")


def frozen_anchor(revision):
    path = "scripts/check-architecture-history.py"
    if not git("ls-tree", "--name-only", revision, path).stdout.strip():
        return None
    tree = ast.parse(git("show", f"{revision}:{path}").stdout)
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == "LEGACY_VENDOR_SNAPSHOT" for target in node.targets):
            value = ast.literal_eval(node.value)
            if isinstance(value, str) and len(value) == 40:
                return value
    raise ValueError(f"{revision}: missing or invalid frozen vendor snapshot")


def check_history(base, head):
    anchor = git("rev-parse", "--verify", f"{LEGACY_VENDOR_SNAPSHOT}^{{commit}}", check=False)
    if anchor.returncode:
        raise ValueError("audited vendor snapshot is unavailable; fetch complete repository history")
    legacy = set(git("rev-list", LEGACY_VENDOR_SNAPSHOT).stdout.splitlines())
    ancestor = git("merge-base", base, head).stdout.strip()
    bootstrap = inventory(ancestor)
    commits = git("rev-list", "--reverse", "--topo-order", f"{ancestor}..{head}").stdout.splitlines()
    for commit in commits:
        paths = git("diff-tree", "--root", "--first-parent", "-m", "--no-commit-id", "--name-only", "-z", "-r", "--diff-filter=ACDMRTUXB", commit).stdout.split("\0")
        if commit not in legacy and any(path.startswith("vendor/") for path in paths) and any(path.startswith(("crates/", "src/")) for path in paths):
            raise ValueError(f"{commit}: vendor changes must be separate from application changes")
        current = inventory(commit)
        parents = git("rev-list", "--parents", "-n", "1", commit).stdout.split()[1:]
        current_anchor = frozen_anchor(commit)
        parent_anchors = [frozen_anchor(parent) for parent in parents]
        if current_anchor is not None and current_anchor != LEGACY_VENDOR_SNAPSHOT:
            raise ValueError(f"{commit}: cannot change frozen vendor snapshot")
        if any(previous is not None and previous != current_anchor for previous in parent_anchors):
            raise ValueError(f"{commit}: cannot change or remove frozen vendor snapshot")
        parent_inventories = [inventory(parent) for parent in parents]
        for previous in parent_inventories:
            no_growth(previous, current, commit)
        if current is not None and not any(previous is not None for previous in parent_inventories):
            if bootstrap is not None:
                no_growth(bootstrap, current, commit)
            else:
                bootstrap = current
    head_anchor = frozen_anchor(head)
    if head_anchor is not None and head_anchor != LEGACY_VENDOR_SNAPSHOT:
        raise ValueError("working tree: cannot change frozen vendor snapshot")
    previous = inventory(head)
    working = Path("xtask/baseline.toml")
    if working.exists():
        no_growth(previous, tomllib.loads(working.read_text()), "working tree")
    elif previous is not None:
        raise ValueError("working tree: cannot remove architectural inventory")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    try:
        check_history(args.base, args.head)
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"architecture history: {error}\n")


if __name__ == "__main__":
    main()
