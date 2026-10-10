#!/usr/bin/env python3
"""Rebuild vendored forks from pinned published archives and ordered patches."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib
import urllib.error
import urllib.request


MAX_DOWNLOAD = 32 * 1024 * 1024
MAX_EXTRACTED = 256 * 1024 * 1024
MAX_MEMBERS = 20000


class ReplayError(Exception):
    """An input cannot safely reproduce the vendored source."""


def git(directory, *args):
    result = subprocess.run(
        ["git", "-c", "core.autocrlf=false", "-c", "core.filemode=true", *args],
        cwd=directory, capture_output=True,
    )
    if result.returncode:
        raise ReplayError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def relative_path(value):
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        raise ReplayError(f"unsafe path: {value!r}")
    parts = value.split("/")
    if any(part in ("", ".", "..", ".git") for part in parts):
        raise ReplayError(f"unsafe path: {value!r}")
    if PurePosixPath(value).is_absolute() or ":" in parts[0]:
        raise ReplayError(f"unsafe path: {value!r}")
    return Path(*parts)


def load_manifest(root, manifest):
    data = json.loads(manifest.read_text())
    if data.get("format") != 1 or not isinstance(data.get("packages"), list):
        raise ReplayError("unsupported vendor manifest")
    packages = data["packages"]
    names = [record["name"] for record in packages]
    if len(set(names)) != len(names):
        raise ReplayError("duplicate vendor package records")
    cargo = tomllib.loads((root / "Cargo.toml").read_text())
    expected = cargo.get("patch", {}).get("crates-io", {})
    if set(names) != set(expected):
        raise ReplayError(
            f"vendor records differ from Cargo patches: missing {sorted(set(expected) - set(names))}, "
            f"extra {sorted(set(names) - set(expected))}"
        )
    for record in packages:
        name, version = record["name"], record["version"]
        if not re.fullmatch(r"[A-Za-z0-9_-]+", name):
            raise ReplayError(f"invalid package name: {name}")
        if not re.fullmatch(r"[A-Za-z0-9_.+-]+", version):
            raise ReplayError(f"invalid package version: {version}")
        if expected[name].get("path") != f"vendor/{name}":
            raise ReplayError(f"unexpected Cargo vendor path for {name}")
        url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
        if record["archive_url"] != url:
            raise ReplayError(f"unexpected published archive URL for {name}")
        if not re.fullmatch(r"[0-9a-f]{64}", record["sha256"]):
            raise ReplayError(f"invalid archive digest for {name}")
        upstream = record["upstream"]
        if (upstream["evidence"] not in ("cargo-vcs", "publisher-zed-rev")
                or not re.fullmatch(r"[0-9a-f]{40}", upstream["revision"])
                or not upstream["repository"].startswith("https://github.com/")):
            raise ReplayError(f"invalid upstream provenance for {name}")
        if upstream["path"]:
            relative_path(upstream["path"])
        omitted = record["omitted"]
        if len(set(omitted)) != len(omitted):
            raise ReplayError(f"duplicate omissions for {name}")
        for path in omitted:
            relative_path(path)
        patches = record["patches"]
        if not patches or len(set(patches)) != len(patches):
            raise ReplayError(f"missing or duplicate patches for {name}")
        for path in patches:
            safe = relative_path(path)
            if safe.parts[:3] != ("vendor", "patches", name):
                raise ReplayError(f"patch outside package patch directory: {path}")
            if not (root / safe).is_file() or (root / safe).is_symlink():
                raise ReplayError(f"patch missing or not regular: {path}")
            if any(parent.is_symlink() for parent in (root / safe).parents if parent != root):
                raise ReplayError(f"patch path traverses a symlink: {path}")
    return packages


def verify_digest(path, digest):
    if path.is_symlink() or not path.is_file():
        raise ReplayError(f"archive missing or not regular: {path}")
    if path.stat().st_size > MAX_DOWNLOAD:
        raise ReplayError(f"archive exceeds download limit: {path}")
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != digest:
        raise ReplayError(f"archive checksum mismatch: {path.name}: {actual}, expected {digest}")


def download(url, destination):
    # Bound both transfer size and wait time, and never publish an incomplete cache.
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "nocterm-vendor-replay/1"})
            with urllib.request.urlopen(request, timeout=30) as response:
                if not response.geturl().startswith("https://"):
                    raise ReplayError("archive redirected to non-HTTPS URL")
                with destination.open("wb") as output:
                    size = 0
                    while chunk := response.read(1024 * 1024):
                        size += len(chunk)
                        if size > MAX_DOWNLOAD:
                            raise ReplayError("archive exceeds download limit")
                        output.write(chunk)
            return
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def archive_for(record, cache, archives_dir):
    filename = f"{record['name']}-{record['version']}.crate"
    if archives_dir is not None:
        path = archives_dir / filename
        # Explicit offline mode never falls back to downloading.
        verify_digest(path, record["sha256"])
        return path
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / filename
    if not path.exists():
        with tempfile.TemporaryDirectory(dir=cache) as staging:
            staged = Path(staging) / filename
            download(record["archive_url"], staged)
            verify_digest(staged, record["sha256"])
            os.replace(staged, path)
    # A corrupt cache is an error, never silently trusted or repaired.
    verify_digest(path, record["sha256"])
    return path


def extract_archive(archive, destination, package_root):
    seen, total = set(), 0
    with tarfile.open(archive, "r:gz") as source:
        for index, member in enumerate(source):
            if index >= MAX_MEMBERS:
                raise ReplayError("archive exceeds member limit")
            name = member.name.rstrip("/") if member.isdir() else member.name
            safe = relative_path(name)
            if safe.parts[0] != package_root:
                raise ReplayError(f"unexpected archive root: {member.name}")
            if name in seen:
                raise ReplayError(f"duplicate archive member: {name}")
            seen.add(name)
            target = destination / safe
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                total += member.size
                if total > MAX_EXTRACTED:
                    raise ReplayError("archive exceeds extracted-size limit")
                target.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(member) as stream, target.open("xb") as output:
                    shutil.copyfileobj(stream, output)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)
            else:
                raise ReplayError(f"archive member is not regular or directory: {name}")
    base = destination / package_root
    if not base.is_dir():
        raise ReplayError("empty archive or missing package root")
    return base


def prune(base, omitted):
    for path in omitted:
        target = base / relative_path(path)
        if target.is_symlink() or not target.is_file():
            raise ReplayError(f"omitted file missing or not regular: {path}")
        target.unlink()


def verify_provenance(base, record):
    upstream = record["upstream"]
    if upstream["evidence"] == "cargo-vcs":
        vcs = json.loads((base / ".cargo_vcs_info.json").read_text())
        revision, path = vcs["git"]["sha1"], vcs["path_in_vcs"]
    else:
        cargo = tomllib.loads((base / "Cargo.toml").read_text())
        revision = cargo["package"]["metadata"]["gpui-pre"]["zed-rev"]
        # Publisher metadata asserts a revision, not a git-to-package byte mapping.
        path = upstream["path"]
    if revision != upstream["revision"] or path != upstream["path"]:
        raise ReplayError(f"upstream provenance differs from archive: {record['name']}")


def validate_patch(patch):
    content = patch.read_bytes()
    # Git's C-quoted path syntax needs careful decoding. Our source filenames do
    # not need it; fail closed instead of accepting a second path grammar.
    changes = 0
    for line in content.splitlines():
        if line.startswith(b"diff --git "):
            match = re.fullmatch(rb"diff --git a/([^\s]+) b/([^\s]+)", line)
            if not match:
                raise ReplayError(f"unsupported patch paths in {patch}")
            relative_path(os.fsdecode(match[1]))
            relative_path(os.fsdecode(match[2]))
            changes += 1
        elif line.startswith((b"--- ", b"+++ ")):
            name = line[4:]
            if name != b"/dev/null":
                if not name.startswith((b"a/", b"b/")):
                    raise ReplayError("unsupported patch file path")
                relative_path(os.fsdecode(name[2:]))
        elif line.startswith((b"old mode ", b"new mode ", b"new file mode ", b"deleted file mode ")):
            if line.rsplit(b" ", 1)[1] not in (b"100644", b"100755"):
                raise ReplayError("patch may only create or modify regular files")
        elif line.startswith(b"index ") and line.endswith((b" 120000", b" 160000")):
            raise ReplayError("patch may not introduce symlinks or gitlinks")
        elif line.startswith((b"rename from ", b"rename to ", b"copy from ", b"copy to ")):
            relative_path(os.fsdecode(line.split(b" ", 2)[2]))
    if not changes:
        raise ReplayError(f"empty patch: {patch}")


def files_under(base, skip=None):
    files = {}
    for directory, dirs, names in os.walk(base, followlinks=False):
        current = Path(directory)
        dirs[:] = [name for name in dirs if not (skip and skip(current / name))]
        for name in dirs:
            if (current / name).is_symlink():
                raise ReplayError(f"source directory is a symlink: {current / name}")
        for name in names:
            path = current / name
            if skip and skip(path):
                continue
            if path.is_symlink() or not path.is_file():
                raise ReplayError(f"source is not regular: {path}")
            files[path.relative_to(base).as_posix()] = path
    return files


def vendor_files(root, name):
    base = root / "vendor" / name
    if base.is_symlink() or not base.is_dir():
        raise ReplayError(f"vendor directory missing or not regular: {base}")
    tracked = git(root, "ls-files", "-z", "--", f"vendor/{name}").split(b"\0")
    tracked_paths = {os.fsdecode(path) for path in tracked if path}

    def skip(path):
        # Only ignored Cargo build outputs are excluded. Other ignored files,
        # and untracked, nonignored sources still participate in comparison.
        relative = path.relative_to(root).as_posix()
        if path.name != "target" or not path.is_dir():
            return False
        if path.is_symlink():
            return False
        if any(item == relative or item.startswith(relative + "/") for item in tracked_paths):
            return False
        result = subprocess.run(["git", "check-ignore", "-q", "--", relative], cwd=root)
        return result.returncode == 0

    return files_under(base, skip)


def compare_files(rebuilt, vendored):
    missing = sorted(set(rebuilt) - set(vendored))
    added = sorted(set(vendored) - set(rebuilt))
    changed, modes = [], []
    for name in sorted(set(rebuilt) & set(vendored)):
        left, right = rebuilt[name], vendored[name]
        if left.read_bytes() != right.read_bytes():
            changed.append(name)
        if bool(left.stat().st_mode & 0o111) != bool(right.stat().st_mode & 0o111):
            modes.append(name)
    if missing or added or changed or modes:
        raise ReplayError(
            f"source mismatch: missing={missing}, added={added}, changed={changed}, executable-mode={modes}"
        )


def replay(root, record, archive, temporary):
    base = extract_archive(archive, temporary, f"{record['name']}-{record['version']}")
    verify_provenance(base, record)
    prune(base, record["omitted"])
    # Initialize a fresh index so --index enforces modes and exact preimages.
    git(base, "init", "-q")
    git(base, "add", "--all", "--force")
    for name in record["patches"]:
        patch = root / relative_path(name)
        validate_patch(patch)
        git(base, "apply", "--check", "--index", "--whitespace=nowarn", str(patch))
        git(base, "apply", "--index", "--whitespace=nowarn", str(patch))
    rebuilt = files_under(base, lambda path: path == base / ".git")
    for additional in record.get("additional_sources", []):
        verify_digest(base / relative_path(additional["path"]), additional["sha256"])
    compare_files(rebuilt, vendor_files(root, record["name"]))
    return len(rebuilt)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--archives-dir", type=Path, help="offline .crate inputs; never download missing files")
    args = parser.parse_args()
    root = args.root.resolve()
    cache = root / "target" / "vendor-patches" / "archives"
    try:
        packages = load_manifest(root, root / "vendor" / "patches" / "sources.json")
        for record in packages:
            archive = archive_for(record, cache, args.archives_dir)
            workspace = root / "target" / "vendor-patches"
            workspace.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=workspace) as temporary:
                count = replay(root, record, archive, Path(temporary))
            upstream = record["upstream"]
            print(f"{record['name']} {record['version']}: {count} files match; "
                  f"{upstream['revision']} ({upstream['evidence']})")
    except (ReplayError, OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.exit(1, f"vendor replay failed: {error}\n")


if __name__ == "__main__":
    main()
