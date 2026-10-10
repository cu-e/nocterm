#!/usr/bin/env python3
"""Behavior tests for archive authentication and exact vendor reconstruction."""

import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("vendor_replay", Path(__file__).with_name("check-vendor-patches.py"))
replay = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(replay)


def write_archive(path, members):
    with tarfile.open(path, "w:gz") as archive:
        for name, content, kind in members:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            entry.size = len(content) if kind == tarfile.REGTYPE else 0
            entry.mode = 0o644
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                entry.linkname = "../../outside"
            archive.addfile(entry, io.BytesIO(content) if kind == tarfile.REGTYPE else None)


class VendorReplayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        self.archives = Path(self.temp.name) / "archives"
        self.archives.mkdir()
        self.archive = self.archives / "fixture-1.0.0.crate"
        vcs = json.dumps({"git": {"sha1": "a" * 40}, "path_in_vcs": "crates/fixture"}).encode()
        self.members = [
            ("fixture-1.0.0/src/lib.rs", b"pub fn original() {}\n", tarfile.REGTYPE),
            ("fixture-1.0.0/.cargo_vcs_info.json", vcs, tarfile.REGTYPE),
            ("fixture-1.0.0/omitted.txt", b"published sample\n", tarfile.REGTYPE),
        ]
        write_archive(self.archive, self.members)
        self.record = {
            "name": "fixture", "version": "1.0.0",
            "archive_url": "https://static.crates.io/crates/fixture/fixture-1.0.0.crate",
            "sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "upstream": {"repository": "https://github.com/example/fixture", "revision": "a" * 40,
                         "path": "crates/fixture", "evidence": "cargo-vcs"},
            "omitted": ["omitted.txt"], "patches": ["vendor/patches/fixture/0001-nocterm.patch"],
        }
        self.base = replay.extract_archive(self.archive, Path(self.temp.name) / "generation", "fixture-1.0.0")
        replay.prune(self.base, self.record["omitted"])
        replay.git(self.base, "init", "-q")
        replay.git(self.base, "add", "--all", "--force")
        replay.git(self.base, "-c", "user.name=Test", "-c", "user.email=test@invalid", "commit", "-qm", "base")
        (self.base / "src/lib.rs").write_bytes(b"pub fn local_patch() {}\n")
        (self.base / "binary.bin").write_bytes(b"\x00\x01\x02\xff")
        (self.base / "executable").write_bytes(b"#!/bin/sh\nexit 0\n")
        (self.base / "executable").chmod(0o755)
        replay.git(self.base, "add", "--all", "--force")
        self.patch = self.root / self.record["patches"][0]
        self.patch.parent.mkdir(parents=True)
        self.patch.write_bytes(replay.git(self.base, "diff", "--cached", "--binary", "--full-index"))
        self.vendor = self.root / "vendor/fixture"
        shutil.copytree(self.base, self.vendor, ignore=shutil.ignore_patterns(".git"))
        (self.root / "Cargo.toml").write_text('[patch.crates-io]\nfixture = { path = "vendor/fixture" }\n')
        (self.root / ".gitignore").write_text("target/\n*.tmp\n")
        self.manifest = self.root / "vendor/patches/sources.json"
        self.save_manifest([self.record])
        replay.git(self.root, "init", "-q")
        replay.git(self.root, "add", "--all")

    def save_manifest(self, records):
        self.manifest.write_text(json.dumps({"format": 1, "packages": records}))

    def run_replay(self, record=None):
        with tempfile.TemporaryDirectory(dir=self.temp.name) as directory:
            return replay.replay(self.root, record or self.record, self.archive, Path(directory))

    def test_published_archive_binary_patch_and_executable_match(self):
        self.assertEqual(self.run_replay(), 4)
        self.assertEqual(len(replay.load_manifest(self.root, self.manifest)), 1)

    def test_all_binary_stream_preserves_final_delta_and_detects_truncation(self):
        (self.base / ".git/info/attributes").write_text("* -diff\n")
        content = replay.git(self.base, "diff", "--cached", "--binary", "--full-index")
        self.patch.write_bytes(content + b"# End of generated Nocterm binary patch.\n")
        self.assertEqual(self.run_replay(), 4)
        # Git can accept a binary stream missing its final blank delimiter while
        # skipping the final delta. The complete byte comparison must catch it.
        self.patch.write_bytes(content.rstrip(b"\n") + b"\n")
        with self.assertRaises(replay.ReplayError):
            self.run_replay()

    def test_wrong_checksum_rejected_before_extracting(self):
        with self.assertRaisesRegex(replay.ReplayError, "checksum mismatch"):
            replay.verify_digest(self.archive, "0" * 64)

    def test_unsafe_archive_paths_links_and_duplicate_members_rejected(self):
        bad = ["/absolute", "fixture-1.0.0/../outside", "fixture-1.0.0/./x",
               "fixture-1.0.0/.git/config", "wrong-root/file", "fixture-1.0.0/back\\slash"]
        for name in bad:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                archive = Path(directory) / "bad.crate"
                write_archive(archive, [(name, b"bad", tarfile.REGTYPE)])
                with self.assertRaises(replay.ReplayError):
                    replay.extract_archive(archive, Path(directory) / "out", "fixture-1.0.0")
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                archive = Path(directory) / "bad.crate"
                write_archive(archive, [("fixture-1.0.0/link", b"", kind)])
                with self.assertRaises(replay.ReplayError):
                    replay.extract_archive(archive, Path(directory) / "out", "fixture-1.0.0")
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "duplicate.crate"
            write_archive(archive, self.members + [self.members[0]])
            with self.assertRaisesRegex(replay.ReplayError, "duplicate"):
                replay.extract_archive(archive, Path(directory) / "out", "fixture-1.0.0")

    def test_archive_extraction_is_bounded(self):
        with patch.object(replay, "MAX_EXTRACTED", 1), tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(replay.ReplayError, "extracted-size limit"):
                replay.extract_archive(self.archive, Path(directory), "fixture-1.0.0")

    def test_archive_member_count_is_bounded(self):
        with patch.object(replay, "MAX_MEMBERS", 1), tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(replay.ReplayError, "member limit"):
                replay.extract_archive(self.archive, Path(directory), "fixture-1.0.0")

    def test_missing_omission_rejected(self):
        record = copy.deepcopy(self.record)
        record["omitted"].append("absent.txt")
        with self.assertRaisesRegex(replay.ReplayError, "omitted file missing"):
            self.run_replay(record)

    def test_duplicate_or_escaping_omissions_rejected(self):
        for paths in (["omitted.txt", "omitted.txt"], ["../outside"]):
            record = copy.deepcopy(self.record)
            record["omitted"] = paths
            self.save_manifest([record])
            with self.assertRaises(replay.ReplayError):
                replay.load_manifest(self.root, self.manifest)

    def test_wrong_provenance_rejected_even_when_bytes_are_valid(self):
        record = copy.deepcopy(self.record)
        record["upstream"]["revision"] = "b" * 40
        with self.assertRaisesRegex(replay.ReplayError, "provenance differs"):
            self.run_replay(record)

    def test_publisher_revision_is_checked_against_authenticated_archive_metadata(self):
        metadata = ('[package.metadata.gpui-pre]\nzed-rev = "' + "a" * 40 + '"\n').encode()
        write_archive(self.archive, self.members + [
            ("fixture-1.0.0/Cargo.toml", metadata, tarfile.REGTYPE),
        ])
        (self.vendor / "Cargo.toml").write_bytes(metadata)
        record = copy.deepcopy(self.record)
        record["upstream"]["evidence"] = "publisher-zed-rev"
        self.assertEqual(self.run_replay(record), 5)
        record["upstream"]["revision"] = "b" * 40
        with self.assertRaisesRegex(replay.ReplayError, "provenance differs"):
            self.run_replay(record)

    def test_broken_patch_rejected(self):
        self.patch.write_bytes(self.patch.read_bytes().replace(b"pub fn original()", b"pub fn nonexistent()"))
        with self.assertRaisesRegex(replay.ReplayError, "patch failed|does not apply"):
            self.run_replay()

    def test_patches_apply_in_declared_order(self):
        replay.git(self.base, "-c", "user.name=Test", "-c", "user.email=test@invalid", "commit", "-qm", "first patch")
        (self.base / "src/lib.rs").write_bytes(b"pub fn next_patch() {}\n")
        replay.git(self.base, "add", "--all")
        second = self.patch.with_name("0002-next.patch")
        second.write_bytes(replay.git(self.base, "diff", "--cached", "--binary", "--full-index"))
        record = copy.deepcopy(self.record)
        record["patches"].append(second.relative_to(self.root).as_posix())
        (self.vendor / "src/lib.rs").write_bytes(b"pub fn next_patch() {}\n")
        self.assertEqual(self.run_replay(record), 4)
        record["patches"].reverse()
        with self.assertRaises(replay.ReplayError):
            self.run_replay(record)

    def test_patch_cannot_create_symlink_or_escape(self):
        for content in (b"diff --git a/../escape b/../escape\n",
                        b"diff --git a/link b/link\nnew file mode 120000\n"):
            self.patch.write_bytes(content)
            with self.assertRaises(replay.ReplayError):
                self.run_replay()

    def test_patch_headers_cannot_write_outside_fresh_source_tree(self):
        sentinel = Path(self.temp.name) / "sentinel"
        sentinel.write_bytes(b"untouched")
        forbidden = [
            b"diff --git a/../sentinel b/../sentinel\n",
            b"diff --git a/.git/config b/.git/config\n",
            b'diff --git "a/escaped\\tname" "b/escaped\\tname"\n',
            b"diff --git a/safe b/safe\n--- a/safe\n+++ b/../sentinel\n",
            b"diff --git a/safe b/safe\nrename from safe\nrename to ../sentinel\n",
            b"diff --git a/safe b/safe\ncopy from safe\ncopy to /absolute\n",
            b"diff --git a/submodule b/submodule\nnew file mode 160000\n",
            b"diff --git a/safe b/safe\nindex 123..456 120000\n",
        ]
        for content in forbidden:
            with self.subTest(content=content):
                self.patch.write_bytes(content)
                with self.assertRaises(replay.ReplayError):
                    self.run_replay()
                self.assertEqual(sentinel.read_bytes(), b"untouched")

    def test_whitespace_drift_is_not_repaired_to_make_a_patch_apply(self):
        changed = self.archive.with_name("drift.crate")
        members = [(name, data.replace(b"original() {}", b"original() { }"), kind)
                   for name, data, kind in self.members]
        write_archive(changed, members)
        with tempfile.TemporaryDirectory(dir=self.temp.name) as directory:
            with self.assertRaisesRegex(replay.ReplayError, "does not apply|patch failed"):
                replay.replay(self.root, self.record, changed, Path(directory))

    def test_changed_missing_and_added_source_reported(self):
        source = self.vendor / "src/lib.rs"
        source.write_bytes(b"unexpected edit\n")
        with self.assertRaisesRegex(replay.ReplayError, "changed=\\['src/lib.rs'\\]"):
            self.run_replay()
        source.unlink()
        with self.assertRaisesRegex(replay.ReplayError, "missing=\\['src/lib.rs'\\]"):
            self.run_replay()
        source.write_bytes(b"pub fn local_patch() {}\n")
        (self.vendor / "untracked.rs").write_text("new source")
        with self.assertRaisesRegex(replay.ReplayError, "added=\\['untracked.rs'\\]"):
            self.run_replay()

    def test_executable_mode_drift_reported(self):
        (self.vendor / "executable").chmod(0o644)
        with self.assertRaisesRegex(replay.ReplayError, "executable-mode=\\['executable'\\]"):
            self.run_replay()

    def test_only_ignored_build_target_is_excluded(self):
        target = self.vendor / "target"
        target.mkdir()
        (target / "output").write_text("compiler output")
        self.assertEqual(self.run_replay(), 4)
        (self.vendor / "ignored.tmp").write_text("ignored source still matters")
        with self.assertRaisesRegex(replay.ReplayError, "ignored.tmp"):
            self.run_replay()

    def test_symlink_build_target_is_not_silently_excluded(self):
        (self.vendor / "target").symlink_to(self.archives, target_is_directory=True)
        with self.assertRaisesRegex(replay.ReplayError, "symlink"):
            self.run_replay()

    def test_tracked_files_inside_ignored_target_remain_source_inputs(self):
        target = self.vendor / "target"
        target.mkdir()
        (target / "tracked.rs").write_text("tracked source in a build-named directory")
        replay.git(self.root, "add", "--force", "vendor/fixture/target/tracked.rs")
        with self.assertRaisesRegex(replay.ReplayError, "target/tracked.rs"):
            self.run_replay()

    def test_vendor_file_and_directory_symlinks_are_rejected(self):
        external = self.archives / "external"
        external.write_text("outside source")
        for name, destination in (("link.rs", external), ("linked-dir", self.archives)):
            link = self.vendor / name
            with self.subTest(name=name):
                link.symlink_to(destination, target_is_directory=destination.is_dir())
                with self.assertRaisesRegex(replay.ReplayError, "symlink|not regular"):
                    self.run_replay()
                self.assertEqual(external.read_text(), "outside source")
                link.unlink()

    def test_additional_source_digest_checks_locally_added_license(self):
        record = copy.deepcopy(self.record)
        record["additional_sources"] = [{"path": "binary.bin", "sha256": "0" * 64}]
        with self.assertRaisesRegex(replay.ReplayError, "checksum mismatch.*binary.bin"):
            self.run_replay(record)

    def test_omission_must_name_a_regular_file_not_a_directory(self):
        record = copy.deepcopy(self.record)
        record["omitted"] = ["src"]
        with self.assertRaisesRegex(replay.ReplayError, "omitted file missing or not regular"):
            self.run_replay(record)

    def test_missing_extra_or_duplicate_records_rejected(self):
        for records in ([], [self.record, self.record], [dict(self.record, name="extra")]):
            self.save_manifest(records)
            with self.assertRaisesRegex(replay.ReplayError, "records|duplicate"):
                replay.load_manifest(self.root, self.manifest)

    def test_manifest_rejects_unpinned_urls_digests_and_escaped_patch_paths(self):
        mutations = [
            ("archive_url", "http://static.crates.io/fixture.crate"),
            ("sha256", "not a digest"),
            ("version", "../escape"),
            ("patches", []),
            ("patches", [self.record["patches"][0]] * 2),
            ("patches", ["vendor/patches/fixture/../../../sentinel"]),
            ("patches", ["vendor/patches/other/0001.patch"]),
        ]
        for key, value in mutations:
            with self.subTest(key=key, value=value):
                record = dict(self.record, **{key: value})
                self.save_manifest([record])
                with self.assertRaises(replay.ReplayError):
                    replay.load_manifest(self.root, self.manifest)

    def test_manifest_rejects_patch_file_and_parent_symlinks(self):
        actual = self.archives / "actual.patch"
        shutil.copyfile(self.patch, actual)
        self.patch.unlink()
        self.patch.symlink_to(actual)
        with self.assertRaisesRegex(replay.ReplayError, "not regular"):
            replay.load_manifest(self.root, self.manifest)
        self.patch.unlink()
        self.patch.parent.rmdir()
        self.patch.parent.symlink_to(self.archives, target_is_directory=True)
        shutil.copyfile(actual, self.archives / self.patch.name)
        with self.assertRaisesRegex(replay.ReplayError, "traverses a symlink"):
            replay.load_manifest(self.root, self.manifest)

    def test_offline_mode_succeeds_and_never_downloads_missing_archive(self):
        with patch.object(replay, "download") as download:
            self.assertEqual(replay.archive_for(self.record, self.root / "cache", self.archives), self.archive)
            self.archive.unlink()
            with self.assertRaisesRegex(replay.ReplayError, "archive missing"):
                replay.archive_for(self.record, self.root / "cache", self.archives)
            download.assert_not_called()

    def test_corrupt_cache_rejected_without_network_repair(self):
        cache = self.root / "cache"
        cache.mkdir()
        (cache / self.archive.name).write_bytes(b"corrupt")
        with patch.object(replay, "download") as download:
            with self.assertRaisesRegex(replay.ReplayError, "checksum mismatch"):
                replay.archive_for(self.record, cache, None)
            download.assert_not_called()

    def test_offline_archive_symlink_is_not_trusted(self):
        actual = self.archives / "actual.crate"
        self.archive.rename(actual)
        self.archive.symlink_to(actual)
        with self.assertRaisesRegex(replay.ReplayError, "not regular"):
            replay.archive_for(self.record, self.root / "cache", self.archives)

    def test_download_cache_is_authenticated_and_rechecked(self):
        cache = self.root / "cache"
        def transfer(url, destination):
            shutil.copyfile(self.archive, destination)
        with patch.object(replay, "download", side_effect=transfer) as download:
            path = replay.archive_for(self.record, cache, None)
            self.assertEqual(path.read_bytes(), self.archive.read_bytes())
            self.assertEqual(replay.archive_for(self.record, cache, None), path)
            self.assertEqual(download.call_count, 1)

    def test_invalid_download_is_never_published(self):
        cache = self.root / "cache"
        with patch.object(replay, "download", side_effect=lambda url, path: path.write_bytes(b"bad")):
            with self.assertRaisesRegex(replay.ReplayError, "checksum mismatch"):
                replay.archive_for(self.record, cache, None)
        self.assertFalse((cache / self.archive.name).exists())

    def test_download_enforces_transport_size_limit(self):
        class Response(io.BytesIO):
            def geturl(self):
                return "https://static.crates.io/fixture.crate"
        with patch.object(replay.urllib.request, "urlopen", return_value=Response(b"excessive")), \
                patch.object(replay, "MAX_DOWNLOAD", 2):
            with self.assertRaisesRegex(replay.ReplayError, "download limit"):
                replay.download("https://static.crates.io/fixture.crate", self.root / "download")

    def test_download_rejects_insecure_redirect(self):
        class Response(io.BytesIO):
            def geturl(self):
                return "http://unsafe/fixture.crate"
        with patch.object(replay.urllib.request, "urlopen", return_value=Response(b"bytes")):
            with self.assertRaisesRegex(replay.ReplayError, "non-HTTPS"):
                replay.download("https://static.crates.io/fixture.crate", self.root / "download")

    def test_cli_offline_replay_and_failure_exit_status(self):
        command = ["python3", str(Path(replay.__file__)), "--root", str(self.root), "--archives-dir", str(self.archives)]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("4 files match", result.stdout)
        self.archive.unlink()
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("archive missing", result.stderr)

    def test_cli_reports_wrong_archive_digest_without_replay_success(self):
        self.archive.write_bytes(b"corrupted offline archive")
        result = subprocess.run(
            ["python3", str(Path(replay.__file__)), "--root", str(self.root),
             "--archives-dir", str(self.archives)], capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertNotIn("files match", result.stdout)

    def test_cli_reports_mutated_source_and_nonignored_untracked_addition(self):
        command = ["python3", str(Path(replay.__file__)), "--root", str(self.root),
                   "--archives-dir", str(self.archives)]
        source = self.vendor / "src/lib.rs"
        original = source.read_bytes()
        source.write_bytes(b"changed local source")
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("changed=['src/lib.rs']", result.stderr)
        source.write_bytes(original)
        (self.vendor / "new.rs").write_text("untracked nonignored source")
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("added=['new.rs']", result.stderr)


if __name__ == "__main__":
    unittest.main()
