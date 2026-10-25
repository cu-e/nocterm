#!/usr/bin/env python3
"""One case per rule of the plugin API compatibility check."""
import copy
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "compat", Path(__file__).with_name("check-plugin-api-compat.py")
)
compat = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compat)

ROOT = Path(__file__).resolve().parent.parent
BASE = json.loads((ROOT / compat.SURFACE).read_text(encoding="utf-8"))


def entry(surface, name):
    return next(item for item in surface["capabilities"] if item["id"] == name)


def bumped(surface, version="0.2"):
    surface["api_version"] = version
    return surface


class Rules(unittest.TestCase):
    def head(self):
        return copy.deepcopy(BASE)

    def assertBreaks(self, head, fragment):
        problems = compat.check(BASE, head)
        self.assertTrue(any(fragment in problem for problem in problems), problems)

    def test_the_unchanged_surface_passes(self):
        self.assertEqual(compat.check(BASE, self.head()), [])

    def test_a_removed_capability_fails_even_with_a_bump(self):
        head = bumped(self.head())
        head["capabilities"] = [c for c in head["capabilities"] if c["id"] != "clock.now"]
        self.assertBreaks(head, "clock.now was removed")

    def test_a_changed_family_target_or_since_fails(self):
        for key, value in (("family", "observe"), ("target", "own-data"), ("since", "0.0")):
            head = bumped(self.head())
            entry(head, "notifications.post")[key] = value
            self.assertBreaks(head, f"{key} changed")

    def test_a_removed_or_retyped_result_field_fails(self):
        head = bumped(self.head())
        del entry(head, "clock.now")["fields"]["unix_minutes"]
        self.assertBreaks(head, "unix_minutes was removed")
        head = bumped(self.head())
        entry(head, "clock.now")["fields"]["unix_minutes"] = "text"
        self.assertBreaks(head, "unix_minutes changed type")

    def test_a_gesture_that_becomes_required_fails(self):
        head = bumped(self.head())
        entry(head, "notifications.post")["gesture"] = True
        self.assertBreaks(head, "user gesture")

    def test_a_tighter_or_new_rate_limit_fails_and_a_looser_one_passes(self):
        head = bumped(self.head())
        entry(head, "notifications.post")["rate"] = {"per_ms": 20000, "uses": 1}
        self.assertBreaks(head, "rate limit was tightened")
        head = bumped(self.head())
        entry(head, "storage.set")["rate"] = {"per_ms": 1000, "uses": 1}
        self.assertBreaks(head, "rate limit was tightened")
        head = bumped(self.head())
        entry(head, "notifications.post")["rate"] = {"per_ms": 5000, "uses": 1}
        self.assertEqual(compat.check(BASE, head), [])

    def test_a_lower_or_new_live_cap_fails(self):
        head = bumped(self.head())
        entry(head, "notifications.post")["live_cap"] = 2
        self.assertBreaks(head, "live cap was lowered")
        head = bumped(self.head())
        entry(head, "storage.set")["live_cap"] = 4
        self.assertBreaks(head, "live cap was lowered")

    def test_a_removed_name_list_value_fails(self):
        head = bumped(self.head())
        head["zones"] = head["zones"][1:]
        self.assertBreaks(head, "zones: value")

    def test_manifest_breaks_fail(self):
        head = bumped(self.head())
        del head["manifest"]["contribution"]["title"]
        self.assertBreaks(head, "contribution.title was removed")
        head = bumped(self.head())
        head["manifest"]["contribution"]["order"]["type"] = "text"
        self.assertBreaks(head, "contribution.order changed type")
        head = bumped(self.head())
        head["manifest"]["contribution"]["order"]["required"] = True
        self.assertBreaks(head, "contribution.order became required")
        head = bumped(self.head())
        head["manifest"]["contribution"]["extra"] = {"required": True, "type": "text"}
        self.assertBreaks(head, "contribution.extra is new and required")
        head = bumped(self.head())
        del head["manifest"]["settings"]
        self.assertBreaks(head, "manifest table settings was removed")

    def test_an_optional_new_manifest_key_passes_with_a_bump(self):
        head = bumped(self.head())
        head["manifest"]["contribution"]["extra"] = {"required": False, "type": "text"}
        self.assertEqual(compat.check(BASE, head), [])

    def test_a_removed_effect_request_field_fails(self):
        head = bumped(self.head())
        del head["effect_requests"]["storage.set"]["value"]
        self.assertBreaks(head, "field value was removed")
        head = bumped(self.head())
        del head["effect_requests"]["storage.set"]
        self.assertBreaks(head, "effect request storage.set was removed")

    def test_a_retyped_or_narrowed_effect_request_field_fails(self):
        head = bumped(self.head())
        head["effect_requests"]["storage.set"]["value"]["type"] = "integer"
        self.assertBreaks(head, "effect request storage.set: field value changed type")
        head = bumped(self.head())
        head["effect_requests"]["panels.set-badge"]["badge"]["max"] -= 1
        self.assertBreaks(head, "effect request panels.set-badge: field badge: the accepted range narrowed")
        head = bumped(self.head())
        head["effect_requests"]["notifications.post"]["tone"]["vocabulary"] = "tones"
        self.assertBreaks(head, "effect request notifications.post: field tone: vocabulary changed")

    def test_an_optional_effect_request_field_made_required_fails(self):
        head = bumped(self.head())
        head["effect_requests"]["notifications.post"]["notice_key"]["required"] = True
        self.assertBreaks(head, "effect request notifications.post: field notice_key became required")

    def test_a_new_effect_request_field_must_be_optional(self):
        head = bumped(self.head())
        head["effect_requests"]["storage.remove"]["extra"] = {"required": True, "type": "text"}
        self.assertBreaks(head, "effect request storage.remove: field extra is new and required")
        head = bumped(self.head())
        head["effect_requests"]["storage.remove"]["extra"] = {"required": False, "type": "text"}
        self.assertEqual(compat.check(BASE, head), [])

    def test_limits_break_in_the_direction_of_their_kind(self):
        head = bumped(self.head())
        head["limits"]["text-chars"] -= 1
        self.assertBreaks(head, "limit text-chars was lowered")
        head = bumped(self.head())
        head["limits"]["min-refresh-ms"] += 1
        self.assertBreaks(head, "limit min-refresh-ms was raised")
        head = bumped(self.head())
        del head["limits"]["text-chars"]
        self.assertBreaks(head, "limit text-chars was removed")
        head = bumped(self.head())
        head["limits"]["text-chars"] += 1
        head["limits"]["min-refresh-ms"] -= 1
        self.assertEqual(compat.check(BASE, head), [])

    def test_a_lowered_api_version_fails(self):
        head = self.head()
        head["api_version"] = "0.0"
        self.assertBreaks(head, "api_version was lowered")

    def test_an_additive_change_needs_a_bump(self):
        head = self.head()
        head["zones"].append("middle")
        self.assertBreaks(head, "raise it")
        self.assertEqual(compat.check(BASE, bumped(head)), [])

    def test_a_new_limit_needs_a_higher_major_version(self):
        head = bumped(self.head())
        head["limits"]["zones-per-package"] = 1
        self.assertBreaks(head, "limit zones-per-package is new")
        head["limits"]["min-something"] = 1
        self.assertBreaks(head, "limit min-something is new")
        self.assertEqual(compat.check(BASE, bumped(head, "1.0")), [])

    def test_a_changed_vocabulary_of_a_key_fails_even_with_a_bump(self):
        head = bumped(self.head())
        head["manifest"]["contribution"]["zone"]["vocabulary"] = "icons"
        self.assertBreaks(head, "contribution.zone: vocabulary changed from zones to icons")

    def test_a_narrower_integer_range_fails_and_a_wider_one_passes(self):
        for field, value in (("min", 1), ("max", 100)):
            head = bumped(self.head())
            head["manifest"]["settings.group.field"]["min"][field] = value
            self.assertBreaks(head, "settings.group.field.min: the accepted range narrowed")
        head = bumped(self.head())
        head["manifest"]["contribution"]["order"]["max"] += 1
        self.assertEqual(compat.check(BASE, head), [])

    def test_a_text_only_change_passes_without_a_bump(self):
        head = self.head()
        for name in compat.TEXT_FIELDS:
            entry(head, "clock.now")[name] = "different words"
        head["capabilities"].reverse()
        self.assertEqual(compat.check(BASE, head), [])

    def test_a_deprecated_removal_passes_only_on_a_major_bump(self):
        base = copy.deepcopy(BASE)
        entry(base, "storage.remove")["deprecated"] = "storage.set"
        head = copy.deepcopy(base)
        head["capabilities"] = [c for c in head["capabilities"] if c["id"] != "storage.remove"]
        head["api_version"] = "1.0"
        self.assertEqual(compat.check(base, head), [])
        head["api_version"] = "0.2"
        self.assertTrue(
            any("storage.remove was removed" in p for p in compat.check(base, head))
        )


GOLDEN = {
    "abi_version": 1,
    "exports": {"nocterm_call": "(i32, i32) -> i64"},
    "samples": {"a": "00", "b": "01"},
}


class AbiGolden(unittest.TestCase):
    """The golden file may only grow while `abi_version` stays, whatever the working tree says."""

    def head(self, **changes):
        head = copy.deepcopy(GOLDEN)
        head.update(changes)
        return head

    def test_an_unchanged_or_appended_golden_file_passes(self):
        self.assertEqual(compat.abi_breaks(GOLDEN, self.head()), [])
        samples = {**GOLDEN["samples"], "c": "02"}
        self.assertEqual(compat.abi_breaks(GOLDEN, self.head(samples=samples)), [])

    def test_a_changed_or_removed_sample_needs_a_higher_version(self):
        changed = self.head(samples={**GOLDEN["samples"], "b": "ff"})
        self.assertTrue(any("`b` changed" in p for p in compat.abi_breaks(GOLDEN, changed)))
        removed = self.head(samples={"a": "00"})
        self.assertTrue(any("`b`" in p for p in compat.abi_breaks(GOLDEN, removed)))
        exports = self.head(exports={"nocterm_call": "(i32) -> i64"})
        self.assertTrue(any("exports" in p for p in compat.abi_breaks(GOLDEN, exports)))
        self.assertEqual(compat.abi_breaks(GOLDEN, self.head(abi_version=2, samples={})), [])

    def test_a_lower_or_missing_version_fails(self):
        base = {**GOLDEN, "abi_version": 2}
        self.assertTrue(compat.abi_breaks(base, self.head()))
        self.assertTrue(compat.abi_breaks(GOLDEN, {"samples": {}}))


def script(*arguments):
    return subprocess.run(
        ["python3", str(ROOT / "scripts" / "check-plugin-api-compat.py"), *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


class Releases(unittest.TestCase):
    """The tag choice and the missing-file notice, on a repository built for the case."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.original = Path.cwd()
        os.chdir(self.directory.name)
        self.git("init", "-b", "master")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")

    def tearDown(self):
        os.chdir(self.original)
        self.directory.cleanup()

    def git(self, *args):
        return subprocess.run(["git", *args], check=True, capture_output=True, text=True)

    def tag(self, name, files):
        for path, text in files.items():
            Path(path).parent.mkdir(parents=True, exist_ok=True)
            Path(path).write_text(text, encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-m", name, "--allow-empty")
        self.git("tag", name)

    def test_the_highest_version_wins_over_the_newest_tag(self):
        self.tag("v3.10.0", {"a": "1"})
        self.tag("v3.9.0", {"a": "2"})
        self.tag("v4.0.0-rc.1", {"a": "3"})
        self.assertEqual(compat.latest_tag(), "v3.10.0")

    def test_no_tag_means_nothing_to_compare(self):
        self.assertIsNone(compat.latest_tag())

    def surface(self, text="{}"):
        return {"crates/plugin-api/api-surface.json": text}

    def test_a_base_without_the_file_passes_with_a_notice(self):
        self.tag("v1.0.0", {"README.md": "x"})
        done = script()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("notice", done.stdout)

    def test_a_base_that_does_not_resolve_is_an_error_and_not_a_pass(self):
        self.tag("v1.0.0", {"README.md": "x"})
        done = script("--base", "nonexistent-ref")
        self.assertEqual(done.returncode, 2, done.stdout + done.stderr)
        self.assertIn("not a commit", done.stdout)
        self.assertNotIn("notice", done.stdout)

    def test_a_base_with_the_file_is_compared(self):
        text = json.dumps(BASE)
        self.tag("v1.0.0", self.surface(text))
        done = script("--base", "v1.0.0")
        # The checkout here is the fixture, so the working-tree file is the same one.
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn("compatible with v1.0.0", done.stdout)

    def test_an_edited_sample_is_caught_against_the_release_even_when_the_file_was_edited_too(self):
        golden = "crates/plugin-abi/abi-golden.json"
        self.tag("v1.0.0", {**self.surface(json.dumps(BASE)), golden: json.dumps(GOLDEN)})
        edited = {**GOLDEN, "samples": {**GOLDEN["samples"], "b": "ff"}}
        Path(golden).write_text(json.dumps(edited), encoding="utf-8")
        done = script("--base", "v1.0.0")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("samples `b` changed", done.stdout)

        Path(golden).write_text(json.dumps(GOLDEN), encoding="utf-8")
        done = script("--base", "v1.0.0")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

        Path(golden).unlink()
        done = script("--base", "v1.0.0")
        self.assertEqual(done.returncode, 2, done.stdout + done.stderr)

    def test_a_base_with_a_broken_file_is_an_error(self):
        self.tag("v1.0.0", self.surface("not json"))
        done = script("--base", "v1.0.0")
        self.assertEqual(done.returncode, 2, done.stdout + done.stderr)
        self.assertIn("is not JSON", done.stdout)

    def test_tags_that_are_not_releases_are_an_error_and_not_a_pass(self):
        self.tag("nightly", {"README.md": "x"})
        done = script()
        self.assertEqual(done.returncode, 2, done.stdout + done.stderr)
        self.assertIn("no v* release tag", done.stdout)

    def test_a_shallow_clone_without_the_release_tags_is_an_error(self):
        self.tag("v1.0.0", {"README.md": "1"})
        self.tag("later", {"README.md": "2"})
        self.git("tag", "-d", "later")
        source = self.directory.name
        with tempfile.TemporaryDirectory() as parent:
            clone = Path(parent) / "clone"
            subprocess.run(
                ["git", "clone", "--depth", "1", "--no-tags", f"file://{source}", str(clone)],
                check=True,
                capture_output=True,
            )
            os.chdir(clone)
            done = script()
            os.chdir(source)
        self.assertEqual(done.returncode, 2, done.stdout + done.stderr)
        self.assertIn("shallow", done.stdout)

    def test_a_repository_without_any_tag_has_nothing_to_compare_with(self):
        self.tag("first", {"README.md": "x"})
        self.git("tag", "-d", "first")
        done = script()
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn("no release tag yet", done.stdout)


if __name__ == "__main__":
    unittest.main()
