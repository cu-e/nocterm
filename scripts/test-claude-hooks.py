#!/usr/bin/env python3
"""Behaviour of the Claude Code hooks in .claude/hooks against real hook events."""
import json
import os
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

HOOKS = Path(__file__).resolve().parent.parent / ".claude" / "hooks"


class Checkout:
    """A throwaway repository the hook events refer to."""

    def __init__(self, branch="feat/example"):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.git("init", "-q", "-b", branch)
        self.write("xtask/baseline.toml", '[pub_fields]\n"a::State" = 3\n\n[globals]\n')
        self.write("xtask/oversized-files.toml", "[files]\n")
        self.write("xtask/architecture-policy.toml", '[dto]\n"a::Row" = "plain row"\n')

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def write(self, path, text):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def run(self, hook, payload):
        payload = {"cwd": str(self.root), **payload}
        result = subprocess.run(
            [sys.executable, "-B", str(HOOKS / hook)],
            input=json.dumps(payload), capture_output=True, text=True, check=True,
            env={**os.environ, "CLAUDE_PROJECT_DIR": str(self.root)},
        )
        return json.loads(result.stdout)["hookSpecificOutput"] if result.stdout.strip() else None

    def close(self):
        self.directory.cleanup()


class HookCase(unittest.TestCase):
    def setUp(self):
        self.checkout = Checkout()

    def tearDown(self):
        self.checkout.close()

    def decision(self, hook, payload):
        output = self.checkout.run(hook, payload)
        return output and output.get("permissionDecision")


class FileGuard(HookCase):
    def edit(self, path, old, new, agent=None):
        output = self.edit_result(path, old, new, agent)
        return output and output.get("permissionDecision")

    def edit_result(self, path, old, new, agent=None):
        payload = {
            "hook_event_name": "PreToolUse", "tool_name": "Edit",
            "tool_input": {"file_path": str(self.checkout.root / path), "old_string": old, "new_string": new},
        }
        if agent:
            payload["agent_type"] = agent
        return self.checkout.run("guard_files.py", payload)

    def test_release_files_are_never_edited(self):
        for path in ["CHANGELOG.md", "version.txt", ".release-please-manifest.json"]:
            self.checkout.write(path, "1.0.0\n")
            self.assertEqual(self.edit(path, "1.0.0", "1.1.0"), "deny", path)

    def test_debt_inventory_may_shrink_but_not_grow(self):
        baseline = "xtask/baseline.toml"
        self.assertIsNone(self.edit(baseline, '"a::State" = 3', '"a::State" = 2'))
        self.assertEqual(self.edit(baseline, '"a::State" = 3', '"a::State" = 4'), "deny")
        self.assertEqual(self.edit(baseline, "[globals]\n", '[globals]\n"a::G" = 1\n'), "deny")
        self.assertEqual(self.edit("xtask/oversized-files.toml", "[files]\n", '[files]\n"src/big.rs" = 900\n'), "deny")

    def test_plain_data_exemptions_ask_for_design_review(self):
        policy = "xtask/architecture-policy.toml"
        output = self.edit_result(policy, '[dto]\n', '[dto]\n"a::Cell" = "cell"\n')
        self.assertEqual(output["permissionDecision"], "ask")
        self.assertIn("plain-data exemptions", output["permissionDecisionReason"])
        self.assertEqual(self.edit(policy, '"plain row"', '"new reason"'), "ask")
        self.assertIsNone(self.edit(policy, '"a::Row" = "plain row"\n', ""))

    def test_isolation_policy_changes_are_reviewed_as_policy_not_dto_exemptions(self):
        policy = "xtask/architecture-policy.toml"
        for before, after in (
            ('[isolation.providers]\nallowed = ["a"]\n', '[isolation.providers]\nallowed = ["a", "b"]\n'),
            ('[isolation.providers]\nallowed = ["a"]\n', '[isolation.providers]\nallowed = []\n'),
            ('[isolation]\napp = "a"\n', '[isolation]\napp = "b"\n'),
            ('[isolation]\napp = "a"\n', ''),
            ('', '[isolation.gpui_allowed]\nupdate = "reason"\n'),
            ('[isolation.identifiers.a]\nforbidden = ["settings"]\n', '[isolation.identifiers.a]\nforbidden = []\n'),
        ):
            original = '[dto]\n"a::Row" = "plain row"\n' + before
            replacement = '[dto]\n"a::Row" = "plain row"\n' + after
            self.checkout.write(policy, original)
            output = self.edit_result(policy, original, replacement)
            self.assertEqual(output["permissionDecision"], "ask", (before, after))
            self.assertIn("isolation permissions and restrictions", output["permissionDecisionReason"])
            self.assertNotIn("plain-data exemption", output["permissionDecisionReason"])

    def test_policy_formatting_and_comment_changes_do_not_need_review(self):
        policy = "xtask/architecture-policy.toml"
        self.checkout.write(policy, '[isolation.providers]\nallowed = ["a"]\n')
        self.assertIsNone(self.edit(policy, '["a"]', '[ "a" ] # same permissions'))

    def test_inventories_in_worktrees_are_guarded_too(self):
        self.checkout.write(".claude/worktrees/w/.git", "gitdir: elsewhere\n")
        self.checkout.write(".claude/worktrees/w/xtask/baseline.toml", '[globals]\n"a::G" = 1\n')
        self.assertEqual(self.edit(".claude/worktrees/w/xtask/baseline.toml", '= 1', '= 2'), "deny")

    def test_ordinary_files_pass(self):
        self.checkout.write("src/lib.rs", "fn a() {}\n")
        self.assertIsNone(self.edit("src/lib.rs", "a", "b"))

    def test_design_and_review_stages_write_only_plans(self):
        self.checkout.write("src/lib.rs", "fn a() {}\n")
        self.checkout.write(".claude/plans/x/30-design.md", "draft\n")
        for agent in ["architect", "reviewer"]:
            self.assertEqual(self.edit("src/lib.rs", "a", "b", agent), "deny", agent)
            self.assertIsNone(self.edit(".claude/plans/x/30-design.md", "draft", "plan", agent), agent)

    def test_tester_hands_production_files_back_to_implementer(self):
        self.checkout.write("crates/a/src/lib.rs", "fn a() {}\n")
        self.checkout.write("crates/a/src/panel/run_tests.rs", "fn t() {}\n")
        self.checkout.write("crates/a/src/testing/fs.rs", "fn f() {}\n")
        output = self.edit_result("crates/a/src/lib.rs", "a", "b", "tester")
        self.assertEqual(output["permissionDecision"], "deny")
        self.assertIn("Send the proposed change to the implementer", output["permissionDecisionReason"])
        self.assertIsNone(self.edit("crates/a/src/panel/run_tests.rs", "t", "u", "tester"))
        self.assertIsNone(self.edit("crates/a/src/testing/fs.rs", "f", "g", "tester"))
        self.assertIsNone(self.edit("crates/a/src/lib.rs", "a", "b", "implementer"))

    def test_tester_inline_tests_use_the_same_handoff_without_user_approval(self):
        path = "crates/a/src/state.rs"
        self.checkout.write(path, 'pub fn value() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn value_is_one() { assert_eq!(super::value(), 1); }\n}\n')
        output = self.edit_result(path, 'value_is_one', 'returns_one', 'tester')
        self.assertEqual(output["permissionDecision"], "deny")
        self.assertIn("inline test module", output["permissionDecisionReason"])
        self.assertIn("not a request for user approval", output["permissionDecisionReason"])
        self.assertIsNone(self.edit(path, 'value_is_one', 'returns_one', 'implementer'))


class ShellGuard(HookCase):
    def command(self, command):
        return self.checkout.run("guard_bash.py", {
            "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": command},
        })

    def denied(self, command):
        output = self.command(command)
        return output is not None and output["permissionDecision"] == "deny"

    def test_hooks_are_never_bypassed(self):
        for command in [
            "git commit --no-verify -m x", "git commit -nm x", "git commit --no-verif -m x",
            "git push --no-verify origin feat/x", "git -c core.hooksPath=/dev/null commit -m x",
            "git config core.hooksPath /tmp/none",
            "env EXAMPLE=1 nice -n 5 git commit --no-verify -m x",
            "bash -c 'git commit --no-verify -m x'",
            "bash <<'EOF'\ngit commit --no-verify -m x\nEOF",
            'echo "$(git commit --no-verify -m x)"',
            "echo `git tag v999`",
            "eval 'git tag v999'",
            "if true; then git tag v999; fi",
        ]:
            self.assertTrue(self.denied(command), command)
        self.assertTrue(self.denied(
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null git commit -m x"
        ))
        for command in ["git commit -uno -m 'fix: n'", "git commit -m -n", "git config core.hooksPath .githooks"]:
            self.assertIsNone(self.command(command), command)

    def test_long_lived_branches_change_only_through_pull_requests(self):
        self.checkout.git("checkout", "-q", "-b", "dev")
        for command in [
            'git commit -m "fix: x"', "git cherry-pick abc", "git merge feat/y", "git revert abc",
            "git push --force", "git push origin feat/y:refs/heads/master", "git push -f origin +feat/y:dev",
            "git push origin HEAD", "git push -u origin @", "git pull",
        ]:
            self.assertTrue(self.denied(command), command)
        self.assertIsNone(self.command("git merge --ff-only origin/dev"))
        self.assertIsNone(self.command("git pull --ff-only"))
        self.assertIsNone(self.command('git switch -c feat/new && git commit -m "fix: x"'))
        self.checkout.git("checkout", "-q", "-b", "feat/y")
        self.assertIsNone(self.command('git commit -m "fix: x"'))
        self.assertIsNone(self.command("git push origin feat/y"))
        self.assertTrue(self.denied("git checkout master && git commit -m x"))
        self.assertIsNone(self.command("git checkout dev && git pull --ff-only && git checkout - && git commit -m 'fix: x'"))

    def test_commits_follow_the_directory_the_command_enters(self):
        self.checkout.git("checkout", "-q", "-b", "master")
        worktree = self.checkout.root / "wt"
        worktree.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "feat/wt"], cwd=worktree, check=True)
        self.assertIsNone(self.command(f'cd {worktree} && git commit -m "fix: x"'))
        self.assertTrue(self.denied('git commit -m "fix: x"'))
        self.assertTrue(self.denied('cd "$(git rev-parse --show-toplevel)" && git commit -m x'))
        self.assertTrue(self.denied(f"cd {worktree} && git checkout -b feat/q && cd {self.checkout.root} && git commit -m x"))
        self.assertTrue(self.denied("cd ~no-such-user-nocterm && git commit -m x"))
        fresh = self.command("git worktree add -b feat/n ../fresh && cd ../fresh && git commit -m x")
        self.assertEqual(fresh["permissionDecision"], "ask")

    def test_checking_out_a_file_keeps_the_branch(self):
        self.checkout.git("checkout", "-q", "-b", "master")
        self.checkout.write("src/lib.rs", "")
        self.assertTrue(self.denied("git checkout src/lib.rs && git commit -m x"))
        self.assertIsNone(self.command("git switch --create=feat/q && git commit -m x"))
        self.assertIsNone(self.command("git switch -cfeat/q && git commit -m x"))
        self.checkout.write("dev/notes.md", "")
        self.assertTrue(self.denied("git switch dev && git commit -m x"))
        self.assertTrue(self.denied("cd -P src && git commit -m x"))

    def test_tags_are_left_to_release_automation(self):
        self.assertTrue(self.denied("git tag v1.2.0"))
        self.assertIsNone(self.command("git tag -l 'v*'"))
        self.assertIsNone(self.command("git tag --contains HEAD"))

    def test_cargo_resource_choices_are_not_shell_denials(self):
        wrapper = HOOKS / "cargo-serial.sh"
        for command in [
            "cargo test --workspace 2>&1 | tail -40",
            "cd crates && CARGO_BUILD_JOBS=7 cargo test -p nocterm-vt",
            "NOCTERM_REQUIRE_SSHD=1 cargo test -p nocterm-ssh --test sshd",
            "env CARGO_BUILD_JOBS=9 nice -n 5 cargo +nightly clippy",
            "bash -c 'cargo build'",
            "bash <<'EOF'\ncargo build\nEOF",
            f"{wrapper} cargo test -p nocterm-vt",
            "cargo fmt --all --check",
            "cargo metadata --no-deps",
        ]:
            self.assertIsNone(self.command(command), command)

    def test_cargo_mentioned_as_data_is_not_a_command(self):
        for command in [
            'git commit -m "test: run cargo test before review; then cargo build"',
            "python3 - <<'EOF'\ntext = 'Run `cargo xtask architecture`; cargo test -p x'\nEOF\necho done",
            "echo 'cargo build' # cargo test",
            "grep -n 'cargo test' AGENTS.md | head",
            "git commit -m 'docs: mention `cargo test`'",
            "git commit -m \"$(cat <<'EOF'\nfix: x\n\n1) run `cargo test` (it's slow)\nEOF\n)\"",
            'git commit -m "docs(app): run \\`cargo xtask docs\\` after schema changes"',
            'gh pr create --body "Run \\`cargo xtask architecture\\`"',
            'git commit -m "fix: escape \\$(cargo build)"',
            "echo x # see <<EOF\necho y",
            "cat <<\\EOF\ncargo build\nEOF",
            "cat > notes.md <<'END-OF-NOTES'\nRun cargo build first.\nEND-OF-NOTES",
        ]:
            self.assertIsNone(self.command(command), command)

    def test_inventories_are_not_rewritten_from_the_shell(self):
        for command in [
            "sed -i 's/3/4/' xtask/baseline.toml",
            "echo x >> CHANGELOG.md",
            "cp /tmp/b xtask/oversized-files.toml",
            "cd xtask && sed -i s/3/2/ baseline.toml",
            "rm ./version.txt",
            "sed --in-place=.bak s/3/2/ xtask/baseline.toml",
            "perl -pi -e s/3/2/ xtask/baseline.toml",
            "perl -0777pi -e s/3/2/ xtask/baseline.toml",
            "sed -Ei s/3/2/ xtask/baseline.toml",
        ]:
            self.assertTrue(self.denied(command), command)
        for command in [
            "git diff xtask/baseline.toml 2>&1 | head",
            "cp xtask/baseline.toml /tmp/b",
            "rm vendor/foo/CHANGELOG.md",
            "perl -Mstrict -ne 'print if /3/' xtask/baseline.toml",
        ]:
            self.assertIsNone(self.command(command), command)

    def test_large_commands_are_checked_within_the_hook_timeout(self):
        started = time.monotonic()
        self.assertIsNone(self.command("echo " + " ".join(["$(true)"] * 2500)))
        self.assertLess(time.monotonic() - started, 5)


class CargoLock(unittest.TestCase):
    def run_locked(self, directory, script, cwd=None, **overrides):
        env = {**os.environ, "NOCTERM_CARGO_LOCK": str(Path(directory) / "lock"), **overrides}
        env = {key: value for key, value in env.items() if value is not None}
        return subprocess.Popen(
            [str(HOOKS / "cargo-serial.sh"), "sh", "-c", script], env=env, cwd=cwd, stderr=subprocess.DEVNULL, start_new_session=True,
        )

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_shared_lock_serializes_commands_and_preserves_caller_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "log"
            script = f'echo "start $CARGO_BUILD_JOBS" >> {log}; sleep 0.3; echo end >> {log}'
            runs = [self.run_locked(directory, script, CARGO_BUILD_JOBS="16") for _ in range(2)]
            self.assertEqual([run.wait(timeout=5) for run in runs], [0, 0])
            self.assertEqual(log.read_text().split("\n")[:4], ["start 16", "end", "start 16", "end"])

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_unset_jobs_stay_unset_for_cargo_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.run_locked(directory, 'test "${CARGO_BUILD_JOBS+x}" != x', CARGO_BUILD_JOBS=None).wait(timeout=5), 0)

    def test_off_mode_needs_neither_git_nor_flock(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [shutil.which("bash"), str(HOOKS / "cargo-serial.sh"), shutil.which("sh"), "-c", "exit 5"],
                cwd=directory, env={**os.environ, "PATH": directory, "NOCTERM_CARGO_LOCK": "off"}, timeout=2,
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 5, result.stderr)

    def test_missing_flock_fails_promptly_with_recovery_choices(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [shutil.which("bash"), str(HOOKS / "cargo-serial.sh"), "true"],
                env={**os.environ, "PATH": directory, "NOCTERM_CARGO_LOCK": str(Path(directory) / "lock")},
                capture_output=True, text=True, timeout=2,
            )
            self.assertEqual(result.returncode, 69)
            self.assertIn("NOCTERM_CARGO_LOCK=off", result.stderr)
            self.assertFalse((Path(directory) / "lock.d").exists())

    def git(self, cwd, *arguments):
        subprocess.run(["git", *arguments], cwd=cwd, check=True, capture_output=True)

    def wait_until_ready(self, process, marker):
        deadline = time.monotonic() + 3
        while not marker.exists() and process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(marker.exists(), "lock holder did not start its command")

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_default_lock_resolution_ignores_cdpath_output(self):
        with tempfile.TemporaryDirectory() as directory:
            self.git(directory, "init", "-q", "-b", "feat/example")
            command = self.run_locked(directory, "exit 0", cwd=directory,
                                      NOCTERM_CARGO_LOCK=None, CDPATH=".")
            self.assertEqual(command.wait(timeout=5), 0)

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_default_lock_is_shared_by_worktrees_and_invocation_subdirectories(self):
        with tempfile.TemporaryDirectory(prefix="cargo locks ") as directory:
            repo, worktree = Path(directory) / "repo", Path(directory) / "worktree"
            repo.mkdir()
            self.git(repo, "init", "-q", "-b", "feat/example")
            self.git(repo, "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-q", "--allow-empty", "-m", "test: fixture")
            self.git(repo, "worktree", "add", "-q", "--detach", str(worktree))
            nested = worktree / "nested"
            nested.mkdir()
            ready = Path(directory) / "ready"
            held = self.run_locked(directory, 'printf ready > "$NOCTERM_TEST_READY"; exec sleep 30',
                                   cwd=repo, NOCTERM_CARGO_LOCK=None, NOCTERM_TEST_READY=str(ready))
            try:
                self.wait_until_ready(held, ready)
                second = self.run_locked(directory, "exit 0", cwd=nested, NOCTERM_CARGO_LOCK=None)
                try:
                    with self.assertRaises(subprocess.TimeoutExpired):
                        second.wait(timeout=0.2)
                    held.terminate()
                    held.wait(timeout=3)
                    self.assertEqual(second.wait(timeout=3), 0)
                finally:
                    if second.poll() is None:
                        second.terminate()
                        second.wait(timeout=3)
            finally:
                if held.poll() is None:
                    held.terminate()
                    held.wait(timeout=3)

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_distinct_default_repository_locks_do_not_block_each_other(self):
        with tempfile.TemporaryDirectory() as directory:
            repos = [Path(directory) / name for name in ("a", "b")]
            for repo in repos:
                repo.mkdir()
                self.git(repo, "init", "-q", "-b", "feat/example")
            ready = Path(directory) / "ready"
            first = self.run_locked(directory, 'printf ready > "$NOCTERM_TEST_READY"; exec sleep 30',
                                    cwd=repos[0], NOCTERM_CARGO_LOCK=None, NOCTERM_TEST_READY=str(ready))
            try:
                self.wait_until_ready(first, ready)
                second = self.run_locked(directory, "exit 0", cwd=repos[1], NOCTERM_CARGO_LOCK=None)
                self.assertEqual(second.wait(timeout=3), 0)
            finally:
                first.terminate()
                first.wait(timeout=3)

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_explicit_path_shares_a_lock_between_independent_repositories(self):
        with tempfile.TemporaryDirectory() as directory:
            repos = [Path(directory) / name for name in ("a", "b")]
            for repo in repos:
                repo.mkdir()
                self.git(repo, "init", "-q", "-b", "feat/example")
            ready = Path(directory) / "ready"
            first = self.run_locked(directory, 'printf ready > "$NOCTERM_TEST_READY"; exec sleep 30',
                                    cwd=repos[0], NOCTERM_TEST_READY=str(ready))
            try:
                self.wait_until_ready(first, ready)
                second = self.run_locked(directory, "exit 0", cwd=repos[1])
                try:
                    with self.assertRaises(subprocess.TimeoutExpired):
                        second.wait(timeout=0.2)
                    first.terminate()
                    first.wait(timeout=3)
                    self.assertEqual(second.wait(timeout=3), 0)
                finally:
                    if second.poll() is None:
                        second.terminate()
                        second.wait(timeout=3)
            finally:
                if first.poll() is None:
                    first.terminate()
                    first.wait(timeout=3)

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_exit_status_passes_through_and_background_children_release_the_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.run_locked(directory, "exit 5").wait(timeout=5), 5)
            self.assertEqual(self.run_locked(directory, "sleep 3 >/dev/null 2>&1 &").wait(timeout=5), 0)
            started = time.monotonic()
            self.assertEqual(self.run_locked(directory, "true").wait(timeout=5), 0)
            self.assertLess(time.monotonic() - started, 2)

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_stopping_the_wrapper_stops_its_command(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "finished"
            run = self.run_locked(directory, f"sleep 2; touch {marker}")
            time.sleep(0.5)
            run.terminate()
            self.assertNotEqual(run.wait(timeout=5), 0)
            time.sleep(2)
            self.assertFalse(marker.exists())

    @unittest.skipUnless(shutil.which("flock"), "optional helper locking requires flock")
    def test_interrupting_the_terminal_stops_every_process_the_command_started(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "finished"
            run = self.run_locked(directory, f"sh -c 'sleep 2; touch {marker}'; touch {marker}")
            time.sleep(0.5)
            os.killpg(run.pid, signal.SIGINT)
            self.assertNotEqual(run.wait(timeout=5), 0)
            time.sleep(2)
            self.assertFalse(marker.exists())


class EditSignals(HookCase):
    def written(self, path, text, kind="update"):
        file = self.checkout.write(path, text)
        output = self.checkout.run("post_edit.py", {
            "hook_event_name": "PostToolUse", "tool_name": "Write",
            "tool_input": {"file_path": str(file), "content": text},
            "tool_response": {"type": kind, "filePath": str(file)},
        })
        return output["additionalContext"] if output else ""

    def edited(self, path, text, edits):
        for edit in edits:
            text = text.replace(edit["old_string"], edit["new_string"], -1 if edit.get("replace_all") else 1)
        file = self.checkout.write(path, text)
        tool = "Edit" if len(edits) == 1 else "MultiEdit"
        data = edits[0] if tool == "Edit" else {"edits": edits}
        output = self.checkout.run("post_edit.py", {
            "hook_event_name": "PostToolUse", "tool_name": tool,
            "tool_input": {"file_path": str(file), **data},
            "tool_response": {"type": "update", "filePath": str(file)},
        })
        return output["additionalContext"] if output else ""

    def test_files_near_the_limit_receive_an_advisory(self):
        warning = self.written("crates/a/src/big.rs", "fn a() {}\n" * 640)
        self.assertIn("responsibility-map", warning)
        self.assertIn("advisory", warning)
        self.assertEqual(self.written("crates/a/src/small.rs", "fn a() {}\n" * 639), "")

    def test_known_edits_warn_when_crossing_size_bands(self):
        for before_lines, added, expected in (
            (638, 1, False), (639, 1, True), (650, 1, False),
            (679, 1, True), (650, 40, True),
        ):
            text = "// unchanged\n" * (before_lines - 1) + "fn original() {}\n"
            warning = self.edited("crates/a/src/state.rs", text, [{
                "old_string": "fn original() {}\n",
                "new_string": "// added\n" * added + "fn changed() {}\n",
            }])
            self.assertEqual("size warning" in warning, expected, (before_lines, added))

    def test_known_unchanged_or_shrinking_files_do_not_repeat_size_advisories(self):
        path = "crates/a/src/state.rs"
        text = "// unchanged\n" * 699 + "fn original() {}\n"
        for old, new in (
            ("original", "changed"),
            ("// unchanged\nfn original() {}\n", "fn changed() {}\n"),
            ("// unchanged\n", ""),  # Deletion has no unique inverse edit site.
            ("// unchanged", "// unchanged"),  # Repeated replacement text.
        ):
            self.assertEqual(self.edited(path, text, [{"old_string": old, "new_string": new}]), "")

    def test_multiple_edits_use_their_combined_size_change(self):
        text = "// unchanged\n" * 638 + "fn first() {}\nfn second() {}\n"
        edits = [
            {"old_string": "fn first() {}", "new_string": "fn renamed() {}"},
            {"old_string": "fn second() {}", "new_string": "fn another() {}"},
        ]
        self.assertEqual(self.edited("crates/a/src/state.rs", text, edits), "")
        edits[1]["new_string"] = "// added\n" * 40 + "fn another() {}"
        self.assertIn("size warning", self.edited("crates/a/src/state.rs", text, edits))

    def test_over_limit_files_keep_warning_even_while_shrinking(self):
        text = "// unchanged\n" * 802 + "fn original() {}\n"
        warning = self.edited("crates/a/src/state.rs", text, [{
            "old_string": "// unchanged\nfn original() {}\n", "new_string": "fn changed() {}\n",
        }])
        self.assertIn("Hard limit exceeded", warning)

    def test_unknown_replace_all_history_keeps_size_warning(self):
        warning = self.edited("crates/a/src/state.rs", "// before\n" * 640, [{
            "old_string": "before", "new_string": "after", "replace_all": True,
        }])
        self.assertIn("size warning", warning)

    def test_new_files_with_only_foreign_impls_are_file_slicing(self):
        sliced = "use super::Thread;\nimpl Thread {\n    fn open(&self) {}\n}\n"
        self.assertIn("owner-extract", self.written("crates/a/src/thread/open.rs", sliced, "create"))
        owner = "pub(super) struct Queue { items: Vec<u8> }\nimpl Queue {}\n"
        self.assertEqual(self.written("crates/a/src/thread/queue.rs", owner, "create"), "")

    def test_catch_all_modules_and_new_crates_are_named(self):
        self.assertIn("catch-all", self.written("crates/a/src/utils.rs", "pub fn a() {}\n", "create"))
        self.assertIn("crate-new", self.written("crates/b/Cargo.toml", "[package]\n", "create"))

    def test_plugin_declarations_and_the_catalog_follow_the_plugin_api_skill(self):
        for path, code in (
            ("crates/files/src/plugin.rs", "pub static PLUGIN: SystemPlugin = value();\n"),
            ("crates/plugin-model/src/catalog.rs", "pub struct Catalog { id: u64 }\n"),
            ("crates/plugin-api/src/lib.rs", "pub struct Capability { id: u64 }\n"),
            ("src/shell_setup.rs", "let plugins = &[&feature::PLUGIN];\n"),
            ("src/shell_setup/user_plugin_panel/sync.rs", "workspace.register_panel(panel, cx);\n"),
        ):
            self.assertIn("plugin-api", self.written(path, code), path)

    def test_existing_plugin_declarations_and_registrations_are_checked_when_changed(self):
        for path, old, new in (
            ("crates/files/src/plugin.rs", "pub static PLUGIN: SystemPlugin = a();", "pub static PLUGIN: SystemPlugin = b();"),
            ("src/shell_setup.rs", "&first::PLUGIN,", "&second::PLUGIN,"),
            ("crates/plugin-model/src/catalog.rs", "pub value: u32,", "pub value: u64,"),
        ):
            warning = self.edited(path, old + "\n", [{"old_string": old, "new_string": new}])
            self.assertIn("plugin-api", warning, path)

    def test_shell_edits_without_contract_changes_do_not_repeat_plugin_routing(self):
        text = "use nocterm_plugin_model::PluginCatalog;\nfn render(catalog: &PluginCatalog) {\n    let title = \"Old\";\n}\n"
        self.assertEqual(self.edited("src/shell_setup/reference.rs", text, [{
            "old_string": '"Old"', "new_string": '"New"',
        }]), "")
        self.assertEqual(self.written("src/shell_setup/settings.rs", "fn settings() {}\n", "create"), "")
        self.assertEqual(self.edited("crates/files/src/plugin.rs", "// PLUGIN description\n", [{
            "old_string": "// PLUGIN description", "new_string": "// PLUGIN clarification",
        }]), "")

    def test_files_that_only_mention_plugins_are_left_alone(self):
        self.assertEqual(self.written("crates/a/src/lib.rs", "// A plugin host.\npub fn plugin() {}\n"), "")
        self.assertEqual(self.written("src/shell_setup_notes.rs", "pub fn a() {}\n"), "")


if __name__ == "__main__":
    unittest.main()
