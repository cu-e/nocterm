#!/usr/bin/env python3
"""Checks repository agent instructions and the configuration in .claude/.

Skills and agents name each other, cite repository paths and are loaded by
frontmatter. A renamed skill, a moved file or a malformed header fails silently
at runtime, so this check fails instead:

- every skill has `name` equal to its directory and a description within the
  1536-character limit; procedure skills (no `argument-hint`) are hidden from
  the menu with `user-invocable: false`; workflow skills take `$ARGUMENTS`;
- every skill an agent preloads with `skills:` exists;
- relative Markdown links, local heading anchors and cited repository paths resolve,
  including in AGENTS.md, CLAUDE.md and CONTRIBUTING.md;
- CLAUDE.md's file imports resolve;
- every hook command in .claude/settings.json names an existing script.
"""
import html
import json
import re
import sys
import unicodedata
from pathlib import Path
from urllib.parse import unquote, urlsplit

DESCRIPTION_LIMIT = 1536
PATH_ROOTS = ("crates/", "src/", "docs/", "xtask/", "scripts/", ".claude/", ".github/")
# Ignored working directories that a fresh checkout does not have.
EPHEMERAL = (".claude/plans/", ".claude/worktrees/")
CITED_PATH = re.compile(r"`([^`\s]+)`")
LINK = re.compile(r"\]\(([^)\s]+)\)")
HOOK_SCRIPT = re.compile(r"\$CLAUDE_PROJECT_DIR/([^\"'\s]+)")
INLINE_CODE = re.compile(r"(`+)(.*?)\1", re.DOTALL)
ROOT_DOCUMENTS = ("AGENTS.md", "CLAUDE.md", "CONTRIBUTING.md")


def outside_fences(text):
    """Keep prose outside Markdown backtick/tilde fences, including unclosed ones."""
    lines, fence = [], None
    for line in text.splitlines():
        marker = re.match(r" {0,3}(`{3,}|~{3,})(.*)$", line)
        if fence:
            if marker and marker[1][0] == fence[0] and len(marker[1]) >= len(fence) and not marker[2].strip():
                fence = None
        elif marker:
            fence = marker[1]
        else:
            lines.append(line)
    return "\n".join(lines)


def markdown_anchors(text):
    """GitHub-style anchors for ATX headings and explicit HTML anchors in prose."""
    text = outside_fences(text)
    anchors, headings = set(), set()
    for tag in re.findall(r"<[^>]+>", INLINE_CODE.sub("", text)):
        for match in re.finditer(r"\b(id|name)\s*=\s*([\"'])(.*?)\2", tag, re.IGNORECASE):
            if match[1].lower() == "id" or re.match(r"<a\s", tag, re.IGNORECASE):
                anchors.add(html.unescape(match[3]))
    for line in text.splitlines():
        heading = re.match(r" {0,3}#{1,6}(?:[ \t]+(.*?)|[ \t]*)$", line)
        if not heading:
            continue
        title = re.sub(r"[ \t]+#+[ \t]*$", "", heading[1] or "").strip()
        title = re.sub(r"!?\[([^\]]+)\]\([^)]*\)", r"\1", title)
        title = INLINE_CODE.sub(lambda match: html.escape(match[2]).replace("_", "&#95;"), title)
        title = re.sub(r"(?<!\w)_{1,2}(.+?)_{1,2}(?!\w)", r"\1", title)
        title = html.unescape(re.sub(r"<[^>]+>", "", title)).lower()
        slug = "".join(character for character in title
                       if character in "-_ " or unicodedata.category(character)[0] in "LNM")
        slug = slug.replace(" ", "-")
        candidate, suffix = slug, 0
        while candidate in headings:
            suffix += 1
            candidate = f"{slug}-{suffix}"
        headings.add(candidate)
    return anchors | headings


def frontmatter(text):
    """Returns the header's top-level keys; list values become Python lists."""
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end < 0:
        return None
    fields, key = {}, None
    for line in text[4:end].splitlines():
        item = re.match(r"\s+-\s+(.*)", line)
        if item and key:
            fields.setdefault(key, [])
            fields[key].append(item.group(1).strip())
            continue
        pair = re.match(r"([\w-]+):\s*(.*)", line)
        if pair:
            key, value = pair.group(1), pair.group(2).strip()
            fields[key] = value.strip("\"'") if value else []
    return fields


def names(value):
    """Skill names from a YAML list or a comma-separated string."""
    if isinstance(value, list):
        return [name.strip("\"'") for name in value]
    return [name.strip() for name in value.split(",") if name.strip()]


def cited_paths(text):
    for match in CITED_PATH.finditer(text):
        path = match.group(1).rstrip(".,:;")
        path = path.split(":")[0] if re.search(r":\d+$", path) else path
        if path.startswith(PATH_ROOTS) and not path.startswith(EPHEMERAL) and not re.search(r"[<>*{}$|]", path):
            yield path


def check_document(root, path, text):
    errors = []
    prose = outside_fences(text)
    for target in LINK.findall(INLINE_CODE.sub("", prose)):
        if re.match(r"[a-zA-Z][\w+.-]*:", target) or target.startswith("//"):
            continue
        link = urlsplit(target)
        destination = path.parent / unquote(link.path) if link.path else path
        if not destination.exists():
            errors.append(f"{path.relative_to(root)}: broken link {target}")
        elif link.fragment and destination.suffix.lower() == ".md" and destination.is_file():
            content = text if destination == path else destination.read_text()
            if unquote(link.fragment) not in markdown_anchors(content):
                errors.append(f"{path.relative_to(root)}: broken anchor {target}")
    for cited in cited_paths(prose):
        if not (root / cited).exists():
            errors.append(f"{path.relative_to(root)}: cited path {cited} does not exist")
    return errors


def check_imports(root, path, text):
    """Check CLAUDE.md's standalone local @file imports."""
    errors = []
    for target in re.findall(r"^\s*@([^\s]+)\s*$", outside_fences(text), re.MULTILINE):
        if not (path.parent / target).is_file():
            errors.append(f"{path.relative_to(root)}: imported file {target} does not exist")
    return errors


def check_skill(root, directory):
    path = directory / "SKILL.md"
    if not path.is_file():
        return [f"{directory.relative_to(root)}: missing SKILL.md"]
    text = path.read_text()
    fields = frontmatter(text)
    where = path.relative_to(root)
    if fields is None:
        return [f"{where}: missing frontmatter"]
    errors = []
    if fields.get("name") != directory.name:
        errors.append(f"{where}: name {fields.get('name')!r} differs from directory")
    description = fields.get("description") or ""
    if not description:
        errors.append(f"{where}: missing description")
    if len(description) > DESCRIPTION_LIMIT:
        errors.append(f"{where}: description longer than {DESCRIPTION_LIMIT} characters")
    workflow = "argument-hint" in fields
    if workflow and "$ARGUMENTS" not in text:
        errors.append(f"{where}: argument-hint without $ARGUMENTS")
    if not workflow and directory.name != "feature" and fields.get("user-invocable") != "false":
        errors.append(f"{where}: procedure skill must set user-invocable: false")
    return errors + check_document(root, path, text)


def check_agent(root, path, skills):
    text = path.read_text()
    fields = frontmatter(text)
    where = path.relative_to(root)
    if fields is None:
        return [f"{where}: missing frontmatter"]
    errors = [f"{where}: name differs from file"] if fields.get("name") != path.stem else []
    for name in names(fields.get("skills", [])):
        if name not in skills:
            errors.append(f"{where}: preloads unknown skill {name}")
    return errors + check_document(root, path, text)


def check_hooks(root):
    settings = root / ".claude" / "settings.json"
    if not settings.exists():
        return []
    errors = []
    for groups in json.loads(settings.read_text()).get("hooks", {}).values():
        for group in groups:
            for hook in group.get("hooks", []):
                for script in HOOK_SCRIPT.findall(hook.get("command", "")):
                    if not (root / script).is_file():
                        errors.append(f".claude/settings.json: hook script {script} does not exist")
    return errors


def check(root):
    skills_dir = root / ".claude" / "skills"
    directories = sorted(path for path in skills_dir.iterdir() if path.is_dir()) if skills_dir.exists() else []
    skills = {directory.name for directory in directories}
    errors = []
    for name in ROOT_DOCUMENTS:
        path = root / name
        if path.is_file():
            text = path.read_text()
            errors += check_document(root, path, text)
            if name == "CLAUDE.md":
                errors += check_imports(root, path, text)
    for directory in directories:
        errors += check_skill(root, directory)
        for extra in sorted(directory.glob("*.md")):
            if extra.name != "SKILL.md":
                errors += check_document(root, extra, extra.read_text())
    for agent in sorted((root / ".claude" / "agents").glob("*.md")):
        errors += check_agent(root, agent, skills)
    return errors + check_hooks(root)


def main():
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    errors = check(root)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
