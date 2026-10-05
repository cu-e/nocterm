#!/usr/bin/env python3
"""Exercise notice drift and bot preparation in an isolated Git repository."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class ReleasePreparation(unittest.TestCase):
    def test_release_bump_regenerates_and_pushes_only_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            repo = temp / "repo"
            repo.mkdir()
            (repo / "scripts").mkdir()
            (repo / "licensing").mkdir()
            for name in ("prepare-release-pr.sh", "generate-license-notices.py"):
                shutil.copy(ROOT / "scripts" / name, repo / "scripts" / name)
            (repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.0"\n')
            (repo / "licensing/sources.json").write_text(
                '[{"component":"rust-std","version":"1.98.0","packages":[], '
                '"path":"rust-license","sha256":'
                '"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",'
                '"url":"https://example.test/license"}]'
            )
            (repo / "licensing/rust-license").write_text("")
            (repo / "about.toml").write_text('accepted = ["MIT"]\n')
            # Harvesting is stubbed; the real generator computes fingerprints,
            # renders notices, and checks them throughout preparation.
            about = temp / "cargo-about"
            about.write_text(
                '#!/usr/bin/env python3\nimport sys\n'
                'print("cargo-about 0.9.2" if "--version" in sys.argv '
                'else \'{"crates": [], "licenses": []}\')\n'
            )
            about.chmod(0o755)
            env = dict(os.environ, CARGO_ABOUT=str(about), GITHUB_OUTPUT=str(temp / "output"))

            def run(*args, check=True):
                return subprocess.run(args, cwd=repo, env=env, check=check,
                                      capture_output=True, text=True)

            def version(number):
                (repo / "Cargo.toml").write_text(
                    f'[package]\nname = "nocterm"\nversion = "{number}"\n')
                (repo / "Cargo.lock").write_text(
                    f'[[package]]\nname = "nocterm"\nversion = "{number}"\n')

            run("git", "init", "-b", "release-please--branches--master")
            run("git", "config", "user.name", "Test")
            run("git", "config", "user.email", "test@example.test")
            run("git", "init", "--bare", str(temp / "remote"))
            run("git", "remote", "add", "origin", str(temp / "remote"))
            version("1.0.0")
            run("python3", "scripts/generate-license-notices.py")
            run("git", "add", ".")
            run("git", "commit", "-m", "chore: initialize release")
            run("git", "push", "-u", "origin", "HEAD")
            version("1.1.0")
            run("git", "add", "Cargo.toml", "Cargo.lock")
            run("git", "commit", "-m", "chore(master): release 1.1.0")
            run("git", "push", "origin", "HEAD")
            self.assertNotEqual(run("python3", "scripts/generate-license-notices.py",
                                    "--check-inputs", check=False).returncode, 0)
            run("bash", "scripts/prepare-release-pr.sh")
            run("python3", "scripts/generate-license-notices.py", "--check")
            self.assertEqual(run("git", "diff-tree", "--no-commit-id", "--name-only",
                                 "-r", "HEAD").stdout.strip(), "THIRD_PARTY_NOTICES.txt")
            sha = run("git", "rev-parse", "HEAD").stdout.strip()
            self.assertEqual((temp / "output").read_text(), f"sha={sha}\n")
            self.assertEqual(run("git", "ls-remote", "origin", "refs/heads/release-please--branches--master")
                             .stdout.split()[0], sha)
            run("bash", "scripts/prepare-release-pr.sh")
            self.assertEqual(run("git", "rev-parse", "HEAD").stdout.strip(), sha)
            notice = repo / "THIRD_PARTY_NOTICES.txt"
            original_notice = notice.read_text()
            notice.write_text(original_notice + "Unapproved notice edit.\n")
            tampered = run("python3", "scripts/generate-license-notices.py",
                           "--check-inputs", check=False)
            self.assertNotEqual(tampered.returncode, 0)
            self.assertIn("was edited", tampered.stderr)
            notice.write_text(original_notice)
            # New bundled assets and dependency-manifest changes must still
            # invalidate packaging, even when package versions stay unchanged.
            (repo / "assets").mkdir()
            asset = repo / "assets" / "font.ttf"
            asset.write_bytes(b"new bundled asset")
            drift = run("python3", "scripts/generate-license-notices.py",
                        "--check-inputs", check=False)
            self.assertNotEqual(drift.returncode, 0)
            self.assertIn("License inputs changed", drift.stderr)
            asset.unlink()
            manifest = repo / "Cargo.toml"
            original_manifest = manifest.read_text()
            manifest.write_text(original_manifest + '\n[dependencies]\nnew-crate = "1"\n')
            drift = run("python3", "scripts/generate-license-notices.py",
                        "--check-inputs", check=False)
            self.assertNotEqual(drift.returncode, 0)
            self.assertIn("License inputs changed", drift.stderr)
            manifest.write_text(original_manifest)
            run("python3", "scripts/generate-license-notices.py", "--check-inputs")
            (repo / "unrelated.txt").write_text("Do not include this in the notice commit.\n")
            run("git", "add", "unrelated.txt")
            self.assertNotEqual(run("bash", "scripts/prepare-release-pr.sh", check=False).returncode, 0)
            self.assertEqual(run("git", "diff", "--cached", "--name-only").stdout.strip(), "unrelated.txt")
            run("git", "reset", "--hard")
            # Another release update wins remotely while this checkout is
            # preparing notices. A normal push must reject the stale checkout.
            competitor = temp / "competitor"
            branch = "release-please--branches--master"
            run("git", "clone", "--branch", branch, str(temp / "remote"), str(competitor))
            run("git", "-C", str(competitor), "config", "user.name", "Competitor")
            run("git", "-C", str(competitor), "config", "user.email", "other@example.test")
            (competitor / "new-release.txt").write_text("Remote branch advanced.\n")
            run("git", "-C", str(competitor), "add", ".")
            run("git", "-C", str(competitor), "commit", "-m", "chore: advance release")
            run("git", "-C", str(competitor), "push", "origin", "HEAD")
            remote_sha = run("git", "ls-remote", "origin", f"refs/heads/{branch}").stdout.split()[0]
            version("1.2.0")
            run("git", "add", "Cargo.toml", "Cargo.lock")
            run("git", "commit", "-m", "chore(master): release 1.2.0")
            original_output = (temp / "output").read_text()
            stale = run("bash", "scripts/prepare-release-pr.sh", check=False)
            self.assertNotEqual(stale.returncode, 0)
            self.assertIn("[rejected]", stale.stderr)
            self.assertEqual(run("git", "ls-remote", "origin", f"refs/heads/{branch}")
                             .stdout.split()[0], remote_sha)
            self.assertEqual((temp / "output").read_text(), original_output)
            run("git", "checkout", "-b", "feat/unsafe")
            self.assertNotEqual(run("bash", "scripts/prepare-release-pr.sh", check=False).returncode, 0)
            run("git", "checkout", "--detach", sha)
            self.assertNotEqual(run("bash", "scripts/prepare-release-pr.sh", check=False).returncode, 0)


if __name__ == "__main__":
    unittest.main()
