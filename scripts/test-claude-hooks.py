#!/usr/bin/env python3
"""Behaviour of the Claude Code hooks in .claude/hooks against real hook events."""
import json
import os
import signal
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
        payload = {
            "hook_event_name": "PreToolUse", "tool_name": "Edit",
            "tool_input": {"file_path": str(self.checkout.root / path), "old_string": old, "new_string": new},
        }
        if agent:
            payload["agent_type"] = agent
        return self.decision("guard_files.py", payload)

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
        self.assertEqual(self.edit(policy, '[dto]\n', '[dto]\n"a::Cell" = "cell"\n'), "ask")

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

    def test_tester_stage_asks_before_touching_production_code(self):
        self.checkout.write("crates/a/src/lib.rs", "fn a() {}\n")
        self.checkout.write("crates/a/src/panel/run_tests.rs", "fn t() {}\n")
        self.checkout.write("crates/a/src/testing/fs.rs", "fn f() {}\n")
        self.assertEqual(self.edit("crates/a/src/lib.rs", "a", "b", "tester"), "ask")
        self.assertIsNone(self.edit("crates/a/src/panel/run_tests.rs", "t", "u", "tester"))
        self.assertIsNone(self.edit("crates/a/src/testing/fs.rs", "f", "g", "tester"))
        self.assertIsNone(self.edit("crates/a/src/lib.rs", "a", "b", "implementer"))


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

    def test_compiling_cargo_runs_through_the_shared_lock(self):
        denied = self.command("cd crates && CARGO_BUILD_JOBS=2 cargo test -p nocterm-vt")
        self.assertEqual(denied["permissionDecision"], "deny")
        self.assertIn("cargo-serial.sh cargo test -p nocterm-vt", denied["permissionDecisionReason"])
        wrapper = HOOKS / "cargo-serial.sh"
        for command in [
            f"{wrapper} cargo test -p nocterm-vt",
            f"'{wrapper}' cargo clippy --workspace --all-targets -- -D warnings",
            "cargo fmt --all --check",
            "cargo metadata --no-deps",
            "grep -rn cargo build.rs",
        ]:
            self.assertIsNone(self.command(command), command)

    def test_suggested_command_keeps_environment_and_drops_redirections(self):
        reason = self.command("cargo test --workspace 2>&1 | tail -40")["permissionDecisionReason"]
        self.assertTrue(reason.endswith("cargo-serial.sh cargo test --workspace"), reason)
        reason = self.command("NOCTERM_REQUIRE_SSHD=1 cargo test -p nocterm-ssh --test sshd")["permissionDecisionReason"]
        self.assertIn("NOCTERM_REQUIRE_SSHD=1 ", reason)
        self.assertTrue(reason.endswith("cargo test -p nocterm-ssh --test sshd"), reason)

    def test_cargo_hidden_behind_prefixes_and_shells_is_found(self):
        for command in [
            "time cargo build --workspace",
            "env CARGO_BUILD_JOBS=2 nice -n 5 cargo +nightly clippy",
            "bash -c 'cd crates && cargo test'",
            "cargo --locked test",
            "echo start\ncargo check",
            "for i in 1; do cargo test; done",
            "if true; then cargo build; fi",
            "{ cargo test; }",
            "! cargo build",
            'echo "$(cargo build)"',
            "echo `cargo build`",
            "bash <<'EOF'\ncargo build\nEOF",
            "echo x | xargs cargo test",
            "sudo cargo build",
            'echo "see <<EOF"\ncargo build',
            "eval 'cargo build'",
            "git log --format=%h#%s && cargo build",
            "cat <<''\nx\n\ncargo build",
        ]:
            self.assertTrue(self.denied(command), command)

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
    def run_locked(self, directory, script, **env):
        env = {**os.environ, "NOCTERM_CARGO_LOCK": str(Path(directory) / "lock"), **env}
        return subprocess.Popen(
            [str(HOOKS / "cargo-serial.sh"), "sh", "-c", script], env=env, stderr=subprocess.DEVNULL, start_new_session=True,
        )

    def test_commands_run_one_at_a_time_with_two_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "log"
            script = f'echo "start $CARGO_BUILD_JOBS" >> {log}; sleep 0.3; echo end >> {log}'
            runs = [self.run_locked(directory, script, CARGO_BUILD_JOBS="16") for _ in range(2)]
            self.assertEqual([run.wait() for run in runs], [0, 0])
            self.assertEqual(log.read_text().split("\n")[:4], ["start 2", "end", "start 2", "end"])

    def test_exit_status_passes_through_and_background_children_release_the_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.run_locked(directory, "exit 5").wait(), 5)
            self.assertEqual(self.run_locked(directory, "sleep 3 >/dev/null 2>&1 &").wait(), 0)
            started = time.monotonic()
            self.assertEqual(self.run_locked(directory, "true").wait(), 0)
            self.assertLess(time.monotonic() - started, 2)

    def test_stopping_the_wrapper_stops_its_command(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "finished"
            run = self.run_locked(directory, f"sleep 2; touch {marker}")
            time.sleep(0.5)
            run.terminate()
            self.assertNotEqual(run.wait(), 0)
            time.sleep(2)
            self.assertFalse(marker.exists())

    def test_interrupting_the_terminal_stops_every_process_the_command_started(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "finished"
            run = self.run_locked(directory, f"sh -c 'sleep 2; touch {marker}'; touch {marker}")
            time.sleep(0.5)
            os.killpg(run.pid, signal.SIGINT)
            self.assertNotEqual(run.wait(), 0)
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

    def test_files_near_the_limit_ask_for_a_responsibility_split(self):
        self.assertIn("responsibility-map", self.written("crates/a/src/big.rs", "fn a() {}\n" * 640))
        self.assertEqual(self.written("crates/a/src/small.rs", "fn a() {}\n" * 639), "")

    def test_new_files_with_only_foreign_impls_are_file_slicing(self):
        sliced = "use super::Thread;\nimpl Thread {\n    fn open(&self) {}\n}\n"
        self.assertIn("owner-extract", self.written("crates/a/src/thread/open.rs", sliced, "create"))
        owner = "pub(super) struct Queue { items: Vec<u8> }\nimpl Queue {}\n"
        self.assertEqual(self.written("crates/a/src/thread/queue.rs", owner, "create"), "")

    def test_catch_all_modules_and_new_crates_are_named(self):
        self.assertIn("catch-all", self.written("crates/a/src/utils.rs", "pub fn a() {}\n", "create"))
        self.assertIn("crate-new", self.written("crates/b/Cargo.toml", "[package]\n", "create"))

    def test_plugin_declarations_and_the_catalog_follow_the_plugin_api_skill(self):
        for path in (
            "crates/files/src/plugin.rs",
            "crates/plugin-model/src/catalog.rs",
            "src/shell_setup.rs",
            "src/shell_setup/reference.rs",
        ):
            self.assertIn("plugin-api", self.written(path, "pub fn a() {}\n"), path)

    def test_files_that_only_mention_plugins_are_left_alone(self):
        self.assertEqual(self.written("crates/a/src/lib.rs", "// A plugin host.\npub fn plugin() {}\n"), "")
        self.assertEqual(self.written("src/shell_setup_notes.rs", "pub fn a() {}\n"), "")


if __name__ == "__main__":
    unittest.main()
