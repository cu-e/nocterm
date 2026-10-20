#!/usr/bin/env python3
"""PreToolUse guard for shell commands against AGENTS.md hard rules.

- Hooks are never bypassed (`--no-verify` and its abbreviations, `commit -n`,
  a `core.hooksPath` override).
- `master` and `dev` change only through pull requests: no commits, merges,
  cherry-picks, reverts or patches on them, and no pushes to them.
- Tags are created by release automation only.
- Compiling Cargo commands run through `cargo-serial.sh`, which applies
  `CARGO_BUILD_JOBS=2` and one lock shared by every agent of the user.
- Release files and debt inventories are changed with Edit/Write, where
  `guard_files.py` can check them, not with shell redirection.

Commands are read like a shell would: heredoc bodies, comments and quoted
arguments are data, except heredocs fed to a shell and command substitutions,
which are scripts. `cd` and branch switches earlier in the same command are
followed. This is a guardrail, not a sandbox: CI remains the gate.
"""
import re
import shlex
import subprocess
import sys
from pathlib import Path

import hooklib

EVENT = "PreToolUse"
WRAPPER = "cargo-serial.sh"
COMPILING = {
    "build", "b", "check", "c", "clippy", "test", "t", "run", "r", "xtask", "doc", "d",
    "bench", "nextest", "llvm-cov", "install", "rustc", "fix",
}
CARGO_OPTIONS_WITH_VALUE = {"--config", "-Z", "-C", "--color"}
PROTECTED = {
    "CHANGELOG.md", "version.txt", ".release-please-manifest.json",
    "xtask/baseline.toml", "xtask/oversized-files.toml",
}
LONG_LIVED = {"master", "dev"}
SHELLS = {"sh", "bash", "zsh", "dash"}
RESERVED = {"!", "{", "}", "do", "then", "else", "elif", "if", "while", "until"}
PUNCTUATION = ";&|()<>\n"
HEREDOC = re.compile(
    r"""<<-?[ \t]*(?:\\?(?P<bare>[A-Za-z_][^\s;&|<>()'"\\]*)|'(?P<single>[^']*)'|"(?P<double>[^"]*)")"""
)
ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=.*")
FD_REDIRECT = re.compile(r"\d+(?=[<>])")
MAX_DEPTH = 4
UNKNOWN = "(unknown)"
CREATE_BRANCH = {"-c", "-C", "-b", "-B", "--create", "--force-create", "--orphan"}
TAG_QUERIES = ("-l", "--list", "--contains", "--no-contains", "--merged", "--no-merged", "--points-at", "-v", "--verify")


class State:
    """Working directory and branch as earlier parts of one command left them."""

    def __init__(self, payload):
        self.payload = payload
        self.cwd = Path(payload.get("cwd") or ".")
        self.branch = None
        self.previous = None

    def enter(self, target):
        """`cd target`; an expanded target keeps the directory, and the branch is re-read.

        A directory that does not exist yet (made earlier in the same command,
        for example by `git worktree add`) has an unknown branch.
        """
        self.branch = self.previous = None
        if "$" in target or "`" in target:
            return
        directory = expanded(self.cwd, target)
        if directory is None:
            return
        self.cwd = directory
        if not directory.is_dir():
            self.branch = UNKNOWN

    def switch(self, target, created=False, paths=False):
        """`git switch`/`checkout target`; checking out a file (`paths`) keeps the branch."""
        if target in {"-", "@{-1}"}:
            before = self.current_branch()
            self.branch, self.previous = self.previous or UNKNOWN, before
            return
        path = expanded(self.cwd, target)
        if paths and not created and path is not None and path.exists() and not self.has_branch(target):
            return
        self.branch, self.previous = target, self.current_branch()

    def has_branch(self, name):
        try:
            return subprocess.run(
                ["git", "show-ref", "--verify", "--quiet", f"refs/heads/{name}"],
                cwd=self.cwd, capture_output=True, timeout=5, check=False,
            ).returncode == 0
        except (OSError, subprocess.SubprocessError):
            return False

    def current_branch(self, directory=None):
        if self.branch and directory is None:
            return self.branch
        cwd = self.cwd / directory if directory else self.cwd
        try:
            return subprocess.run(
                ["git", "branch", "--show-current"],
                cwd=cwd, capture_output=True, text=True, timeout=5, check=False,
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            return ""

    def protected(self, argument):
        if not argument or argument.startswith("-"):
            return False
        path = expanded(self.cwd, argument)
        return path is not None and hooklib.relative({"cwd": str(self.cwd)}, str(path)) in PROTECTED


def expanded(cwd, target):
    """`target` resolved against `cwd` with `~` expanded, or None for `~-` or an unknown `~user`."""
    try:
        return cwd / Path(target).expanduser()
    except RuntimeError:
        return None


def quoting(text):
    """(index, character, open quote or None, escaped by a backslash) per character."""
    quote, escaped = None, False
    for index, char in enumerate(text):
        yield index, char, quote, escaped
        if escaped:
            escaped = False
        elif char == "\\" and quote != "'":
            escaped = True
        elif quote is None and char in "'\"":
            quote = char
        elif char == quote:
            quote = None


def starts_comment(text, index, quote, escaped):
    return quote is None and not escaped and text[index] == "#" and (index == 0 or text[index - 1] in " \t\n;&|(")


def quote_after(quote, line):
    """Quote state at the end of `line` when it starts with `quote` open."""
    state = None
    for _, _, state, _ in quoting((quote or "") + line + "\n"):
        pass
    return state


def heredoc_markers(line, quote):
    """Delimiters of heredocs opened outside quotes and comments on `line`."""
    prefix, markers = quote or "", []
    text = prefix + line
    for index, char, state, escaped in quoting(text):
        position = index - len(prefix)
        if position < 0:
            continue
        if starts_comment(text, index, state, escaped):
            break
        if state is not None or escaped or char != "<":
            continue
        if line.startswith("<<<", position) or (position > 0 and line[position - 1] == "<"):
            continue
        found = HEREDOC.match(line, position)
        if found:
            markers.append(next(group for group in found.groups() if group is not None))
    return markers


def split_heredocs(command):
    """(command without heredoc bodies, bodies fed to a shell)."""
    kept, scripts, pending, body, quote = [], [], [], [], None
    for line in command.split("\n"):
        if pending:
            delimiter, script = pending[0]
            if line.strip() == delimiter:
                if script:
                    scripts.append("\n".join(body))
                body = []
                pending.pop(0)
            else:
                body.append(line)
            continue
        kept.append(line)
        markers = heredoc_markers(line, quote)
        quote = quote_after(quote, line)
        if markers:
            fed = feeds_shell(HEREDOC.sub(" ", line))
            pending = [(marker, fed) for marker in markers]
    return "\n".join(kept), scripts


def feeds_shell(line):
    """Whether a line passes its heredoc to a shell reading a script from stdin."""
    for words, _ in segments(line):
        _, words = program(words)
        if words and Path(words[0]).name in SHELLS and shell_script(words) is None:
            return True
    return False


def closing_paren(text):
    depth = 1
    for index, char, quote, escaped in quoting(text):
        if quote is None and not escaped and char in "()":
            depth += 1 if char == "(" else -1
            if depth == 0:
                return index
    return len(text)


def closing_backtick(text, start):
    index = start
    while True:
        index = text.find("`", index)
        if index < 0 or text[index - 1] != "\\":
            return index
        index += 1


def substitutions(text):
    """Commands inside unescaped `$(…)` and backticks that are not in single quotes."""
    found, index, marks = [], 0, list(quoting(text))
    while index < len(text):
        _, char, quote, escaped = marks[index]
        if quote == "'" or escaped:
            index += 1
            continue
        if char == "`":
            end = closing_backtick(text, index + 1)
            if end > index:
                found.append(text[index + 1:end])
                index = end + 1
                continue
        if text.startswith("$(", index) and not text.startswith("$((", index):
            inner = text[index + 2:]
            close = closing_paren(inner)
            if "<<" in inner[:close]:
                stripped, _ = split_heredocs(inner)
                close = closing_paren(stripped)
                found.append(stripped[:close])
                tail = stripped[close + 1:]
                if close >= len(stripped) or not inner.endswith(tail):
                    break
                index = len(text) - len(tail)
                continue
            found.append(inner[:close])
            index += close + 3
            continue
        index += 1
    return found


def normalized(text):
    """Text without unquoted comments, and with `2>&1` read as `>&1`."""
    out, skip_until = [], -1
    for index, char, quote, escaped in quoting(text):
        if index < skip_until:
            continue
        if starts_comment(text, index, quote, escaped):
            newline = text.find("\n", index)
            skip_until = len(text) if newline < 0 else newline
            continue
        if quote is None and char.isdigit() and (index == 0 or text[index - 1] in " \t\n;&|("):
            match = FD_REDIRECT.match(text, index)
            if match:
                skip_until = match.end()
                continue
        out.append(char)
    return "".join(out)


def tokens(command):
    lexer = shlex.shlex(command.replace("\\\n", " "), posix=True, punctuation_chars=PUNCTUATION)
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    lexer.commenters = ""
    try:
        return list(lexer)
    except ValueError:
        return command.split()


def segments(command):
    """(words, output redirection targets) of each simple command."""
    words, targets, redirect = [], [], None
    for token in tokens(normalized(command)):
        if token and all(char in PUNCTUATION for char in token):
            if "<" in token or ">" in token:
                redirect = token
                continue
            if words:
                yield words, targets
            words, targets = [], []
            continue
        if redirect:
            if ">" in redirect:
                targets.append(token)
            redirect = None
            continue
        words.append(token)
    if words:
        yield words, targets


def drop_options(words, with_value=()):
    while words and words[0].startswith("-") and words[0] != "--":
        words = words[2:] if words[0] in with_value else words[1:]
    return words


def program(words):
    """(environment assignments, words from the real program on)."""
    assignments = []
    while words:
        while words and (ASSIGNMENT.fullmatch(words[0]) or words[0] in RESERVED):
            if ASSIGNMENT.fullmatch(words[0]):
                assignments.append(words[0])
            words = words[1:]
        name = Path(words[0]).name if words else ""
        if name == "env":
            words = drop_options(words[1:], {"-u", "-C", "-S"})
        elif name in {"time", "nohup", "command", "exec", "builtin"}:
            words = drop_options(words[1:])
        elif name == "sudo":
            words = drop_options(words[1:], {"-u", "-g", "-C", "-D", "-h", "-p", "-U"})
        elif name == "xargs":
            words = drop_options(words[1:], {"-a", "-d", "-E", "-I", "-L", "-n", "-P", "-s"})
        elif name == "nice":
            words = drop_options(words[1:], {"-n"})
        elif name == "timeout":
            words = drop_options(words[1:], {"-s", "-k", "--signal", "--kill-after"})[1:]
        else:
            return assignments, words
    return assignments, words


def shell_script(words):
    for index, argument in enumerate(words[1:-1], start=1):
        if re.fullmatch(r"-[a-z]*c[a-z]*", argument):
            return words[index + 1]
    return None


def git_subcommand(words):
    """(subcommand, arguments, -C directory, -c settings) of a git invocation."""
    rest, directory, settings = words[1:], None, []
    while rest and rest[0].startswith("-"):
        if rest[0] in {"-C", "-c"} and len(rest) > 1:
            if rest[0] == "-C":
                directory = rest[1]
            else:
                settings.append(rest[1])
            rest = rest[2:]
        else:
            rest = rest[1:]
    return (rest[0] if rest else None), rest[1:], directory, settings


def skips_hooks(arguments):
    """`--no-verify`, any unique abbreviation of it, or `-n` among short options."""
    expect_value = False
    for argument in arguments:
        if expect_value:
            expect_value = False
            continue
        if len(argument) >= len("--no-ve") and "--no-verify".startswith(argument):
            return True
        if re.fullmatch(r"-[A-Za-z]+", argument):
            for position, letter in enumerate(argument[1:], start=1):
                if letter == "n":
                    return True
                if letter in "mFcCtuS":
                    expect_value = position == len(argument) - 1 and letter in "mFcCt"
                    break
    return False


def push_destinations(arguments, state):
    positional, skip = [], False
    for argument in arguments:
        if skip:
            skip = False
        elif argument in {"-o", "--push-option", "--repo", "--receive-pack", "--exec"}:
            skip = True
        elif not argument.startswith("-"):
            positional.append(argument)
    refspecs = positional[1:]
    if not refspecs:
        return [state.current_branch()]
    destinations = []
    for refspec in refspecs:
        destination = refspec.split(":")[-1].lstrip("+").removeprefix("refs/heads/")
        destinations.append(state.current_branch() if destination in {"HEAD", "@"} else destination)
    return destinations


def created_branch(arguments):
    """Branch named by `-c x`, `-cx`, `--create=x` and their checkout and force forms."""
    for index, argument in enumerate(arguments):
        option, _, value = argument.partition("=")
        if argument in CREATE_BRANCH and index + 1 < len(arguments):
            return arguments[index + 1]
        if option in CREATE_BRANCH and value:
            return value
        if argument[:2] in CREATE_BRANCH and not argument.startswith("--") and len(argument) > 2:
            return argument[2:]
    return None


def check_git(state, assignments, words):
    subcommand, arguments, directory, settings = git_subcommand(words)
    overrides = [value.split("=", 1)[1] for value in assignments if value.startswith("GIT_CONFIG_")]
    if any(setting.lower().startswith("core.hookspath") for setting in settings + overrides):
        hooklib.deny(EVENT, "Never bypass hooks with a core.hooksPath override (AGENTS.md, Hard rules).")
    if subcommand == "config" and any(argument.lower() == "core.hookspath" for argument in arguments):
        if arguments[-1].lower() != "core.hookspath" and arguments[-1] != ".githooks":
            hooklib.deny(EVENT, "core.hooksPath stays .githooks (scripts/setup.sh); never redirect hooks.")
    if subcommand in {"commit", "push", "merge", "rebase", "cherry-pick", "am", "revert", "pull"} and skips_hooks(
        [argument for argument in arguments if argument.startswith("--")] if subcommand != "commit" else arguments
    ):
        hooklib.deny(EVENT, "Never bypass hooks with --no-verify or -n (AGENTS.md, Hard rules). Fix what the hook reports.")
    if subcommand in {"switch", "checkout"}:
        created = created_branch(arguments)
        named = [argument for argument in arguments if argument == "-" or not argument.startswith("-")]
        if created:
            state.switch(created, created=True)
        elif len(named) == 1 and "--" not in arguments:
            state.switch(named[0], paths=subcommand == "checkout")
    if subcommand == "tag" and any(not a.startswith("-") for a in arguments) and not any(a.startswith(TAG_QUERIES) for a in arguments):
        hooklib.deny(EVENT, "Tags are created by release automation only (AGENTS.md, Hard rules; docs/RELEASING.md).")
    changes_branch = subcommand in {"commit", "cherry-pick", "revert", "am"} or (
        subcommand in {"merge", "pull"} and "--ff-only" not in arguments and "--abort" not in arguments
    )
    if changes_branch:
        current = state.current_branch(directory)
        if current == UNKNOWN:
            hooklib.ask(EVENT, f"Cannot tell which branch this {subcommand} lands on; master and dev change only through pull requests.")
        if current in LONG_LIVED:
            hooklib.deny(EVENT, f"Never {subcommand} on {current} directly: work on a <type>/<slug> branch from the recorded target and open a pull request; update a local base with --ff-only (CONTRIBUTING.md, Branching).")
    if subcommand == "push":
        if {"--mirror", "--all"} & set(arguments):
            hooklib.ask(EVENT, "Pushing every branch includes master and dev, which change only through pull requests.")
        reached = LONG_LIVED.intersection(push_destinations(arguments, state))
        if reached:
            hooklib.deny(EVENT, f"{', '.join(sorted(reached))} change only through pull requests; push the feature branch instead (CONTRIBUTING.md, Branching).")


def check_cargo(state, assignments, words):
    if cargo_subcommand(words[1:]) in COMPILING:
        wrapper = hooklib.project_dir(state.payload) / ".claude" / "hooks" / WRAPPER
        suggestion = " ".join([*map(shlex.quote, assignments), shlex.quote(str(wrapper)), shlex.join(words)])
        hooklib.deny(
            EVENT,
            "Compiling Cargo commands run through the shared lock with CARGO_BUILD_JOBS=2 "
            f"(AGENTS.md, Hard rules). Run it as (keeping any pipes and redirections):\n{suggestion}",
        )


def cargo_subcommand(arguments):
    while arguments:
        argument, arguments = arguments[0], arguments[1:]
        if argument in CARGO_OPTIONS_WITH_VALUE:
            arguments = arguments[1:]
        elif not argument.startswith(("-", "+")):
            return argument
    return None


def writes_protected(state, words, targets):
    """Whether a command rewrites a protected file in place."""
    if any(state.protected(target) for target in targets):
        return True
    name, arguments = Path(words[0]).name, words[1:]
    if not any(state.protected(argument) for argument in arguments):
        return False
    if name == "sed":
        return any(re.fullmatch(r"-[nErsuz]*i\S*|--in-place\S*", argument) for argument in arguments)
    if name == "perl":
        return any(re.fullmatch(r"-[0-9xnpaswWXtTUcl]*i\S*", argument) for argument in arguments)
    if name in {"cp", "mv", "install", "ln"}:
        return state.protected(arguments[-1])
    return name in {"tee", "truncate", "rm", "dd"}


def check(state, command, depth=0):
    if depth > MAX_DEPTH:
        return
    text, scripts = split_heredocs(command)
    for script in scripts + substitutions(text):
        check(state, script, depth + 1)
    for words, targets in segments(text):
        assignments, words = program(words)
        if not words:
            continue
        name = Path(words[0]).name
        if writes_protected(state, words, targets):
            hooklib.deny(
                EVENT,
                "Release files and debt inventories are not changed from the shell. Use Edit or Write so the "
                "inventory guard can check that debt only shrinks; release files are never edited.",
            )
        if name in {"cd", "pushd"}:
            targets = [word for word in words[1:] if word == "-" or not word.startswith("-")]
            state.enter(targets[0] if targets else "~")
        elif name == "git":
            check_git(state, assignments, words)
        elif name == "eval":
            check(state, " ".join(words[1:]), depth + 1)
        elif name == "cargo":
            check_cargo(state, assignments, words)
        elif name in SHELLS and shell_script(words):
            check(state, shell_script(words), depth + 1)


def main():
    payload = hooklib.event()
    check(State(payload), (payload.get("tool_input") or {}).get("command") or "")


if __name__ == "__main__":
    main()
    sys.exit(0)
