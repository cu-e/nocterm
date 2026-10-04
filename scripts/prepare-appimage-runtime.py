#!/usr/bin/env python3
"""Fetch the pinned AppImage launcher and corresponding LGPL relinking sources."""

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import subprocess

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "licensing/appimage-runtime.json"
DESTINATION = ROOT / "target/packaging/appimage-runtime"


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(item):
    destination = DESTINATION / item["filename"]
    expected = item["sha256"]
    if destination.exists():
        if checksum(destination) != expected:
            raise ValueError(f"Cached file checksum mismatch: {destination}; remove it and retry")
        return destination
    # Publish only complete, verified downloads; retain no partial archive on error.
    error = None
    for attempt in range(3):
        with tempfile.NamedTemporaryFile(dir=DESTINATION, delete=False) as output:
            temporary = Path(output.name)
            try:
                subprocess.run([
                    "curl", "--fail", "--location", "--silent", "--show-error",
                    "--retry", "3", "--connect-timeout", "15", "--max-time", "120",
                    "--output", str(temporary), item["url"],
                ], check=True)
                if checksum(temporary) != expected:
                    raise ValueError(f"Downloaded file checksum mismatch: {item['url']}")
                temporary.replace(destination)
                return destination
            except (OSError, ValueError, subprocess.CalledProcessError) as failure:
                error = failure
                print(f"Download attempt {attempt + 1} failed: {failure}", file=sys.stderr)
            finally:
                temporary.unlink(missing_ok=True)
    raise ValueError(f"Could not download verified file: {item['url']}: {error}")



def main():
    manifest = json.loads(MANIFEST.read_text())
    DESTINATION.mkdir(parents=True, exist_ok=True)
    runtime = download(manifest["runtime"])
    runtime.chmod(0o755)
    for item in manifest["sources"]:
        download(item)
    print(f"Verified AppImage runtime and corresponding sources in {DESTINATION}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        print(f"AppImage runtime preparation failed: {error}", file=sys.stderr)
        sys.exit(1)
