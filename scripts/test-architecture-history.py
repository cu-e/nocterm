#!/usr/bin/env python3
"""Regression fixtures for commit-level debt and vendor guards."""
import importlib.util
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("history", Path(__file__).with_name("check-architecture-history.py"))
history = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(history)


class CommitPolicies(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.original = Path.cwd()
        os.chdir(self.directory.name)
        self.git("init", "-b", "dev")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("README.md", "fixture")
        self.commit()
        self.base = self.git("rev-parse", "HEAD").stdout.strip()
        self.anchor_patch = patch.object(history, "LEGACY_VENDOR_SNAPSHOT", self.base)
        self.anchor_patch.start()

    def tearDown(self):
        self.anchor_patch.stop()
        os.chdir(self.original)
        self.directory.cleanup()

    def git(self, *args):
        return subprocess.run(["git", *args], check=True, capture_output=True, text=True)

    def write(self, path, text):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-m", "test(xtask): prepare fixture")

    def inventory(self, count):
        self.write("xtask/baseline.toml", f'[globals]\n"fixture::Model" = {count}\n')
        self.commit()

    def test_bootstrap_cannot_hide_later_growth_in_the_same_pr(self):
        self.inventory(1)
        self.inventory(2)
        with self.assertRaisesRegex(ValueError, "baseline increase"):
            history.check_history(self.base, "HEAD")

    def test_reductions_pass_but_deleting_inventory_fails(self):
        self.inventory(2)
        self.inventory(1)
        history.check_history(self.base, "HEAD")
        Path("xtask/baseline.toml").unlink()
        self.commit()
        with self.assertRaisesRegex(ValueError, "cannot remove"):
            history.check_history(self.base, "HEAD")

    def test_vendor_and_product_changes_must_have_separate_commits(self):
        self.write("vendor/example/lib.rs", "fixture")
        self.write("crates/example/src/lib.rs", "fixture")
        self.commit()
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")

    def test_separate_vendor_and_product_commits_pass(self):
        self.write("vendor/example/lib.rs", "fixture")
        self.commit()
        self.write("src/main.rs", "fixture")
        self.commit()
        history.check_history(self.base, "HEAD")

    def test_sibling_reductions_merge_without_comparing_unrelated_commits(self):
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=2\n"Beta"=2\n')
        self.commit()
        shared = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("checkout", "-b", "left")
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=1\n"Beta"=2\n')
        self.commit()
        self.git("checkout", "-b", "right", shared)
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=2\n"Beta"=1\n')
        self.commit()
        self.git("checkout", "left")
        subprocess.run(["git", "merge", "--no-commit", "right"], capture_output=True)
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=1\n"Beta"=1\n')
        self.commit()
        history.check_history(self.base, "HEAD")

    def test_merge_cannot_restore_debt_removed_by_either_parent(self):
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=2\n"Beta"=2\n')
        self.commit()
        shared = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("checkout", "-b", "left")
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=1\n"Beta"=2\n')
        self.commit()
        self.git("checkout", "-b", "right", shared)
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=2\n"Beta"=1\n')
        self.commit()
        self.git("checkout", "left")
        subprocess.run(["git", "merge", "--no-commit", "right"], capture_output=True)
        self.write("xtask/baseline.toml", '[globals]\n"Alpha"=1\n"Beta"=2\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, "baseline increase"):
            history.check_history(self.base, "HEAD")

    def test_transient_growth_is_rejected_even_when_head_restores_the_inventory(self):
        self.inventory(1)
        self.inventory(2)
        self.inventory(1)
        with self.assertRaisesRegex(ValueError, "baseline increase"):
            history.check_history(self.base, "HEAD")

    def test_equal_total_cannot_transfer_a_global_allowance_to_another_type(self):
        self.inventory(1)
        self.write("xtask/baseline.toml", '[globals]\n"fixture::Other" = 1\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, "baseline increase.*Other"):
            history.check_history(self.base, "HEAD")

    def test_inventory_deletion_cannot_be_hidden_by_restoring_it_later(self):
        self.inventory(1)
        Path("xtask/baseline.toml").unlink()
        self.commit()
        self.inventory(1)
        with self.assertRaisesRegex(ValueError, "cannot remove"):
            history.check_history(self.base, "HEAD")

    def test_uncommitted_baseline_growth_is_rejected(self):
        self.inventory(1)
        self.write("xtask/baseline.toml", '[globals]\n"fixture::Model" = 2\n')
        with self.assertRaisesRegex(ValueError, "working tree: baseline increase"):
            history.check_history(self.base, "HEAD")

    def test_inventory_typo_cannot_introduce_a_new_allowance(self):
        self.write("xtask/baseline.toml", '[globalz]\n"fixture::Model" = 1\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, "unknown inventory metrics"):
            history.check_history(self.base, "HEAD")

    def test_vendor_deletion_and_product_edit_still_count_as_mixed_changes(self):
        self.write("vendor/example/lib.rs", "old fixture")
        self.commit()
        Path("vendor/example/lib.rs").unlink()
        self.write("src/main.rs", "replacement fixture")
        self.commit()
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")

    def test_unicode_vendor_paths_cannot_escape_mixed_commit_policy(self):
        self.git("config", "core.quotePath", "true")
        self.write("vendor/тест.rs", "fixture")
        self.write("crates/example/src/lib.rs", "fixture")
        self.commit()
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")

    def test_unicode_product_paths_cannot_escape_mixed_commit_policy(self):
        self.git("config", "core.quotePath", "true")
        self.write("vendor/example/lib.rs", "fixture")
        self.write("crates/тест/src/lib.rs", "fixture")
        self.commit()
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")

    def test_control_characters_in_vendor_paths_cannot_escape_commit_policy(self):
        self.write("vendor/file\twith\ncontrols.rs", "fixture")
        self.write("src/main.rs", "fixture")
        self.commit()
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")

    def test_legacy_mixed_ancestry_passes_but_new_mixed_commits_fail(self):
        self.write("vendor/old.rs", "old")
        self.write("src/old.rs", "old")
        self.commit()
        legacy = self.git("rev-parse", "HEAD").stdout.strip()
        with patch.object(history, "LEGACY_VENDOR_SNAPSHOT", legacy):
            history.check_history(self.base, "HEAD")
            self.write("vendor/new.rs", "new")
            self.write("src/new.rs", "new")
            self.commit()
            with self.assertRaisesRegex(ValueError, "vendor changes"):
                history.check_history(self.base, "HEAD")

    def test_missing_legacy_anchor_fails_closed(self):
        with patch.object(history, "LEGACY_VENDOR_SNAPSHOT", "0" * 40):
            with self.assertRaisesRegex(ValueError, "fetch complete"):
                history.check_history(self.base, "HEAD")

    def test_frozen_snapshot_cannot_be_changed_or_removed(self):
        self.write("scripts/check-architecture-history.py", f'LEGACY_VENDOR_SNAPSHOT = "{self.base}"\n')
        self.commit()
        self.write("scripts/check-architecture-history.py", 'LEGACY_VENDOR_SNAPSHOT = "' + "0"*40 + '"\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, "cannot change"):
            history.check_history(self.base, "HEAD")

    def test_new_merge_commit_cannot_combine_separate_vendor_and_product_commits(self):
        self.git("checkout", "-b", "feature")
        self.write("vendor/new.rs", "vendor")
        self.commit()
        self.write("src/new.rs", "product")
        self.commit()
        self.git("checkout", "dev")
        self.git("merge", "--no-ff", "feature", "-m", "test(xtask): merge fixture")
        with self.assertRaisesRegex(ValueError, "vendor changes"):
            history.check_history(self.base, "HEAD")


if __name__ == "__main__":
    unittest.main()
