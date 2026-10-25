#!/usr/bin/env python3
"""Fail when the user plugin API surface breaks the latest release.

Compares `crates/plugin-api/api-surface.json` in the working tree with the same file at the
highest `v*` release tag (or `--base REF`). Rules:

* a removed or narrowed contract entry fails, whatever the version: a removed capability, key,
  value, field or effect request field, a changed type, vocabulary or family, a narrower integer
  range, a field of an effect request made required or added as required, a stricter
  rate or cap, an upper limit lowered or a `min-*` limit raised;
* a new limit needs a higher major version, because it cannot be told from a narrowing;
* any other change except in the text fields needs a higher `api_version`;
* entries the base marked deprecated may be removed when the major version is higher;
* the ABI golden file (`crates/plugin-abi/abi-golden.json`) must keep every export and sample of
  the base unchanged unless `abi_version` is higher, and `abi_version` never goes down;
* a base that resolves but has no such file passes with a notice. A base that cannot be read
  (a mistyped ref, a shallow clone without tags) is an error, never a pass.

Exit status: 0 compatible, 1 a break, 2 the comparison could not be made.
"""
import argparse
import json
import re
import subprocess
import sys

SURFACE = "crates/plugin-api/api-surface.json"
GOLDEN = "crates/plugin-abi/abi-golden.json"
TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
# Wording for people: it never changes what a package can do.
TEXT_FIELDS = ("consent", "doc", "excluded")
# Limits named `min-*` are lower bounds: raising one narrows the contract. Every other limit is
# an upper bound: lowering it does.
LOWER_BOUND = "min-"


def version(text):
    return tuple(int(part) for part in text.split("."))


def rate_value(rate):
    """The uses per millisecond a rate allows; `None` means no limit."""
    return None if rate is None else rate["uses"] / rate["per_ms"]


def capability_breaks(base, head, major_bump):
    problems, changed = [], False
    heads = {entry["id"]: entry for entry in head["capabilities"]}
    for old in base["capabilities"]:
        name = old["id"]
        new = heads.get(name)
        if new is None:
            if old.get("deprecated") and major_bump:
                continue
            problems.append(f"capability {name} was removed")
            continue
        for key in ("family", "target", "since"):
            if old[key] != new[key]:
                problems.append(f"capability {name}: {key} changed from {old[key]} to {new[key]}")
        for field, kind in (old["fields"] or {}).items():
            if field not in (new["fields"] or {}):
                problems.append(f"capability {name}: result field {field} was removed")
            elif new["fields"][field] != kind:
                problems.append(f"capability {name}: result field {field} changed type")
        if not old["gesture"] and new["gesture"]:
            problems.append(f"capability {name} now needs a user gesture")
        old_rate, new_rate = rate_value(old["rate"]), rate_value(new["rate"])
        if new_rate is not None and (old_rate is None or new_rate < old_rate):
            problems.append(f"capability {name}: the rate limit was tightened")
        if new["live_cap"] is not None and (
            old["live_cap"] is None or new["live_cap"] < old["live_cap"]
        ):
            problems.append(f"capability {name}: the live cap was lowered")
    return problems


# What a key entry (of a manifest table or an effect request) may differ in without a rule of its
# own.
KEY_RULES = ("required", "min", "max")


def key_breaks(where, old, new):
    problems = []
    if old["type"] != new["type"]:
        problems.append(f"{where} changed type")
    for field in sorted((set(old) | set(new)) - set(KEY_RULES) - {"type"}):
        if old.get(field) != new.get(field):
            problems.append(f"{where}: {field} changed from {old.get(field)} to {new.get(field)}")
    if "min" in old and "min" in new and (new["min"] > old["min"] or new["max"] < old["max"]):
        problems.append(
            f"{where}: the accepted range narrowed from {old['min']}..{old['max']}"
            f" to {new['min']}..{new['max']}"
        )
    if not old["required"] and new["required"]:
        problems.append(f"{where} became required")
    return problems


def manifest_breaks(base, head):
    problems = []
    for table, keys in base["manifest"].items():
        new_keys = head["manifest"].get(table)
        if new_keys is None:
            problems.append(f"manifest table {table} was removed")
            continue
        for key, old in keys.items():
            new = new_keys.get(key)
            if new is None:
                problems.append(f"manifest key {table}.{key} was removed")
                continue
            problems += key_breaks(f"manifest key {table}.{key}", old, new)
        for key, new in new_keys.items():
            if key not in keys and new["required"]:
                problems.append(f"manifest key {table}.{key} is new and required")
    return problems


def list_breaks(base, head):
    problems = []
    for name, values in base.items():
        if name == "capabilities" or not isinstance(values, list):
            continue
        for value in set(values) - set(head.get(name, [])):
            problems.append(f"{name}: value {value} was removed")
    return problems


def effect_breaks(base, head):
    problems = []
    for name, fields in base["effect_requests"].items():
        new = head["effect_requests"].get(name)
        if new is None:
            problems.append(f"effect request {name} was removed")
            continue
        for field, old in fields.items():
            if field not in new:
                problems.append(f"effect request {name}: field {field} was removed")
            else:
                problems += key_breaks(f"effect request {name}: field {field}", old, new[field])
        for field, entry in new.items():
            if field not in fields and entry["required"]:
                problems.append(f"effect request {name}: field {field} is new and required")
    return problems


def limit_breaks(base, head, major_bump):
    problems = []
    for name, old in base["limits"].items():
        new = head["limits"].get(name)
        if new is None:
            problems.append(f"limit {name} was removed")
        elif name.startswith(LOWER_BOUND) and new > old:
            problems.append(f"limit {name} was raised from {old} to {new}")
        elif not name.startswith(LOWER_BOUND) and new < old:
            problems.append(f"limit {name} was lowered from {old} to {new}")
    if not major_bump:
        for name in sorted(set(head["limits"]) - set(base["limits"])):
            problems.append(f"limit {name} is new; a new limit needs a higher major version")
    return problems


def normal(surface):
    """The surface without wording and without ordering, for a plain comparison."""
    surface = json.loads(json.dumps(surface))
    surface["capabilities"] = sorted(surface["capabilities"], key=lambda entry: entry["id"])
    for entry in surface["capabilities"]:
        for field in TEXT_FIELDS:
            entry.pop(field, None)
    for name, value in surface.items():
        if isinstance(value, list) and value and isinstance(value[0], str):
            surface[name] = sorted(value)
    return surface


def check(base, head):
    """The problems of `head` against `base`."""
    base_version, head_version = version(base["api_version"]), version(head["api_version"])
    major_bump = head_version[0] > base_version[0]
    problems = []
    if head_version < base_version:
        problems.append(f"api_version was lowered from {base['api_version']} to {head['api_version']}")
    problems += capability_breaks(base, head, major_bump)
    problems += manifest_breaks(base, head)
    problems += list_breaks(base, head)
    problems += effect_breaks(base, head)
    problems += limit_breaks(base, head, major_bump)
    if normal(base) != normal(head) and head_version <= base_version:
        problems.append(
            f"the surface changed and api_version is still {head['api_version']}; raise it"
        )
    return problems


def abi_breaks(base, head):
    """The problems of the ABI golden file `head` against `base`.

    `cargo xtask docs` refuses to rewrite a changed sample in the working tree, but it compares
    with the committed file, so a pull request that edits the sample and the file together passes
    it. Only the release can say what the ABI was.
    """
    old_version, new_version = base.get("abi_version"), head.get("abi_version")
    if not isinstance(old_version, int) or not isinstance(new_version, int):
        return [f"{GOLDEN} names no abi_version"]
    if new_version < old_version:
        return [f"abi_version was lowered from {old_version} to {new_version}"]
    if new_version > old_version:
        return []
    problems = []
    for table in ("exports", "samples"):
        for name, value in (base.get(table) or {}).items():
            if (head.get(table) or {}).get(name) != value:
                problems.append(
                    f"ABI {table} `{name}` changed or was removed and abi_version is still {new_version}"
                )
    return problems


class Unreadable(Exception):
    """The comparison could not be made; the check must not pass."""


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True)


def latest_tag():
    tags = []
    for tag in git("tag", "--list", "v*").stdout.split():
        match = TAG.match(tag)
        if match:
            tags.append((tuple(int(part) for part in match.groups()), tag))
    return max(tags)[1] if tags else None


def default_base():
    """The highest release tag, or `None` when the repository has no release yet.

    A shallow clone, or tags that exist but none of them a release, mean the tags were not
    fetched; the check cannot say there is nothing to compare with.
    """
    tag = latest_tag()
    if tag is not None:
        return tag
    if git("rev-parse", "--is-shallow-repository").stdout.strip() == "true":
        raise Unreadable("the clone is shallow and has no v* tag; fetch the history and the tags")
    if git("rev-list", "--tags", "--max-count=1").stdout.strip():
        raise Unreadable("the repository has tags but no v* release tag; were the tags fetched?")
    return None


def surface_at(ref):
    """The surface file at `ref`, or `None` when the ref resolves and has no such file."""
    return json_at(ref, SURFACE)


def json_at(ref, path):
    """The JSON file `path` at `ref`, or `None` when the ref resolves and has no such file."""
    resolved = git("rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}")
    if resolved.returncode != 0:
        raise Unreadable(f"{ref} is not a commit of this repository")
    listed = git("ls-tree", "--name-only", ref, "--", path)
    if listed.returncode != 0:
        raise Unreadable(f"git could not read the tree of {ref}: {listed.stderr.strip()}")
    if not listed.stdout.strip():
        return None
    shown = git("show", f"{ref}:{path}")
    if shown.returncode != 0:
        raise Unreadable(f"git could not read {path} at {ref}: {shown.stderr.strip()}")
    try:
        return json.loads(shown.stdout)
    except ValueError as error:
        raise Unreadable(f"{path} at {ref} is not JSON: {error}") from error


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="the ref to compare with (default: the highest v* tag)")
    arguments = parser.parse_args()
    try:
        base_ref = arguments.base or default_base()
        if base_ref is None:
            print("notice: no release tag yet; nothing to compare with")
            return 0
        base = surface_at(base_ref)
        base_golden = json_at(base_ref, GOLDEN) if base is not None else None
    except Unreadable as error:
        print(f"::error title=Plugin API compatibility::{error}")
        return 2
    if base is None:
        print(f"notice: {base_ref} has no {SURFACE}; the guarantee starts with the first release containing it")
        return 0
    with open(SURFACE, encoding="utf-8") as handle:
        head = json.load(handle)
    problems = check(base, head)
    if base_golden is not None:
        try:
            with open(GOLDEN, encoding="utf-8") as handle:
                problems += abi_breaks(base_golden, json.load(handle))
        except (OSError, ValueError) as error:
            print(f"::error title=Plugin API compatibility::{GOLDEN} cannot be read here: {error}")
            return 2
    for problem in problems:
        print(f"::error title=Plugin API compatibility::{problem} (against {base_ref})")
    if not problems:
        print(f"The plugin API surface is compatible with {base_ref}.")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
