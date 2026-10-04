#!/usr/bin/env python3
"""Collect original third-party notices for shipped Nocterm dependencies.

Install cargo-about with `cargo install cargo-about --version 0.9.2 --locked`.
Regeneration reads locked crate archives and checked-in pinned upstream notices.
Cargo may fetch missing archives from the network; original notice snapshots are local.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "THIRD_PARTY_NOTICES.txt"
VERSION = "0.9.2"
MARKER = "Input fingerprint: "
CONTENT_MARKER = "Content SHA-256: "
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|notice|ofl|copyright)([._-].*)?$", re.I)


def tracked_inputs():
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT, check=True, capture_output=True,
    )
    paths = set()
    for name in result.stdout.decode().split("\0"):
        if not name:
            continue
        path = Path(name)
        if (name in {"Cargo.lock", "about.toml", ".gitattributes", "rust-toolchain.toml", "rust-toolchain", ".github/workflows/package.yml", "scripts/generate-license-notices.py", "scripts/prepare-appimage-runtime.py"}
                or path.name == "Cargo.toml"
                or name.startswith(("vendor/", "licensing/", ".cargo/"))
                or "assets" in path.parts):
            if (ROOT / path).is_file():
                paths.add(name)
    return sorted(paths)


def fingerprint():
    digest = hashlib.sha256()
    for name in tracked_inputs():
        digest.update(name.encode() + b"\0")
        digest.update((ROOT / name).read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def validate_rust_version():
    sources = json.loads((ROOT / "licensing" / "sources.json").read_text(encoding="utf-8"))
    versions = [source["version"] for source in sources if source.get("component") == "rust-std"]
    if len(versions) != 1:
        raise RuntimeError("Record exactly one pinned rust-std notice version in licensing/sources.json")
    expected = versions[0]
    toolchain = (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    channel = re.search(r'^channel\s*=\s*"([^"\n]+)"', toolchain, re.M)
    if not channel or channel[1] != expected:
        raise RuntimeError("Pinned Rust toolchain changed; update the rust-std notice snapshot")
    installed = subprocess.run(["rustc", "--version"], cwd=ROOT, check=True,
                               text=True, encoding="utf-8", capture_output=True).stdout.split()
    if len(installed) < 2 or installed[1] != expected:
        raise RuntimeError("Installed Rust compiler differs from the pinned rust-std notice version " + expected)


def fast_check(output=None):
    validate_rust_version()
    content = (output or OUTPUT).read_text(encoding="utf-8")
    expected = MARKER + fingerprint()
    if expected not in content.splitlines():
        raise RuntimeError("License inputs changed; regenerate THIRD_PARTY_NOTICES.txt")
    lines = content.splitlines(keepends=True)
    checksums = [line for line in lines if line.startswith(CONTENT_MARKER)]
    payload = "".join(line for line in lines if not line.startswith(CONTENT_MARKER))
    if (len(checksums) != 1 or checksums[0].strip() != CONTENT_MARKER
            + hashlib.sha256(payload.encode()).hexdigest()):
        raise RuntimeError("THIRD_PARTY_NOTICES.txt was edited; regenerate it")
    print("Third-party notice inputs are current.")


def collect(cargo_about):
    version = subprocess.run([cargo_about, "--version"], check=True,
                             text=True, encoding="utf-8", capture_output=True).stdout.strip()
    if version != "cargo-about " + VERSION:
        raise RuntimeError(f"Expected cargo-about {VERSION}, found {version}")
    reports = []
    for manifest in ("Cargo.toml", "crates/vault-broker/Cargo.toml"):
        result = subprocess.run(
            [cargo_about, "generate", "--locked", "--fail", "--format", "json",
             "--config", str(ROOT / "about.toml"), "--manifest-path", str(ROOT / manifest)],
            cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8",
        )
        # Preserve diagnostics: missing or unrecognized files must remain visible.
        if result.stderr:
            print(result.stderr, file=sys.stderr, end="")
        reports.append(json.loads(result.stdout))
    return reports


def original_files(directory):
    for path in sorted(directory.rglob("*")):
        if (path.is_file()
                and path.suffix.lower() not in {".svg", ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp"}
                and (LICENSE_NAME.match(path.name) or path.name == "FTL.TXT"
                     or any(part.lower() in {".licenses", "licenses", "licences"}
                            for part in path.relative_to(directory).parts[:-1]))):
            yield path


def render(reports):
    validate_rust_version()
    packages = {}
    sections = {}
    reference_terms = set()
    actual = set()
    synthesized = set()
    for report in reports:
        for entry in report["crates"]:
            package = entry["package"]
            packages[(package["name"], package["version"])] = (package, entry["license"])
        for license in report["licenses"]:
            text = license["text"].strip()
            key = (license["id"], text)
            users = sections.setdefault(key, set())
            if license["source_path"] is None:
                reference_terms.add(key)
            for user in license["used_by"]:
                package = user["crate"]
                identity = (package["name"], package["version"])
                users.add(identity)
                if license["source_path"]:
                    actual.add(identity)
                else:
                    synthesized.add(identity)

    supplemental = []
    sources_path = ROOT / "licensing" / "sources.json"
    if sources_path.exists():
        sources = json.loads(sources_path.read_text(encoding="utf-8"))
        for source in sources:
            path = ROOT / "licensing" / source["path"]
            if not path.resolve().is_relative_to((ROOT / "licensing").resolve()):
                raise RuntimeError("Supplemental source path escapes licensing directory")
            if hashlib.sha256(path.read_bytes()).hexdigest() != source["sha256"]:
                raise RuntimeError("Pinned license checksum mismatch: " + source["path"])
            for identity in source["packages"]:
                name, version = identity.rsplit("@", 1)
                actual.add((name, version))
            label = ", ".join(source["packages"]) + " / " + source["path"]
            supplemental.append((label + "\nSource: " + source["url"],
                                 path.read_text(encoding="utf-8").strip(), source.get("kind", "notice")))
    # freetype-sys omits the FreeType license referenced by its bundled LICENSE.TXT.
    if any(name == "freetype-sys" for name, _ in packages):
        ftl = ROOT / "licensing" / "supplemental" / "freetype" / "FTL.TXT"
        if not ftl.is_file():
            raise RuntimeError("Supply the pinned FreeType license at licensing/supplemental/freetype/FTL.TXT")
    for identity, (package, _) in sorted(packages.items()):
        directory = Path(package["manifest_path"]).parent
        files = list(original_files(directory))
        # Pinned original notices supplied for upstream archives that omit them.
        override = ROOT / "licensing" / "crates" / f"{identity[0]}-{identity[1]}"
        if override.exists():
            files.extend(original_files(override))
        for path in files:
            relative = path.relative_to(override if path.is_relative_to(override) else directory)
            label = f"{identity[0]} {identity[1]} / {relative.as_posix()}"
            supplemental.append((label, path.read_text(encoding="utf-8").strip(), "notice"))
        if any(path.parent in {directory, override}
               and path.name.lower() not in {"license-lucide", "license-boringssl", "license-other-bits"}
               for path in files):
            actual.add(identity)
    missing = synthesized - actual
    if missing:
        names = ", ".join(f"{name} {version}" for name, version in sorted(missing))
        raise RuntimeError("Only synthesized SPDX text available; supply original notices: " + names)

    lines = [
        "Nocterm third-party licenses and notices", "=" * 40, "",
        "Generated by scripts/generate-license-notices.py using cargo-about " + VERSION + ".",
        MARKER + fingerprint(),
        "Cargo.lock SHA-256: " + hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
        "",
        "Scope: runtime and build dependencies of Nocterm and its optional vault broker",
        "for Linux x86_64, Windows x86_64, macOS x86_64 and macOS arm64.",
        "Installer launcher components and their source-availability notices are included.",
        "This combined notice may include dependencies unused by a particular release.",
        "Development dependencies are excluded. Upstream notices are reproduced below.",
        "Pinned original notices supplement incomplete crate archives. For upstreams that",
        "publish no separate license file, declared SPDX terms are accompanied by their",
        "original licensing declarations and any available source copyright headers.",
        "SPDX reference sections retain template placeholders; actual attribution is",
        "provided by the dependency inventory and original notices below.",
        "Nocterm's own license is supplied separately in LICENSE.",
        "LicenseRef-Nocterm-Third-Party identifies the component terms reproduced in this",
        "file; it does not replace those licenses or create a new license grant.",
        "This software is based in part on the work of the FreeType Team.",
        "",
        "MPL-2.0 source availability: unmodified covered dependency source can be obtained",
        "from the exact-version crate download links listed below. These archives contain",
        "the covered files and build metadata; full license terms are reproduced here.",
        "Vendored modifications retain",
        "their upstream licenses; see each vendor directory's Nocterm change notes.",
        "", "Dependency inventory", "-" * 40,
    ]
    for (name, version), (package, expression) in sorted(packages.items()):
        lines.extend([f"{name} {version}: {expression}"])
        if package.get("authors"):
            lines.append("Authors: " + "; ".join(package["authors"]))
        source = package.get("source") or ""
        if source.startswith("registry+"):
            lines.append(f"Source: https://crates.io/api/v1/crates/{name}/{version}/download")
        elif source.startswith("git+"):
            lines.append("Source: " + source)
        else:
            lines.append("Source: vendor/" + name)
        if package.get("repository"):
            lines.append("Repository: " + package["repository"])
        lines.append("")
    corpus = {}

    def add_notice(text, label):
        normalized = "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").strip().splitlines())
        corpus.setdefault(normalized, set()).add(label)

    for (identifier, text), users in sorted(sections.items()):
        title = "SPDX reference license terms: " if (identifier, text) in reference_terms else "License: "
        add_notice(text, title + identifier + "\nUsed by: "
                   + ", ".join(f"{n} {v}" for n, v in sorted(users)))
    for label, text, kind in supplemental:
        title = "Original licensing declaration: " if kind == "declaration" else "Original notice: "
        add_notice(text, title + label)
    supplemental_root = ROOT / "licensing" / "supplemental"
    if supplemental_root.exists():
        for path in original_files(supplemental_root):
            add_notice(path.read_text(encoding="utf-8"),
                       "Bundled component: " + path.relative_to(supplemental_root).as_posix())
    for text, labels in sorted(corpus.items()):
        lines.extend(["=" * 72, "\n".join(sorted(labels)), "", text, ""])
    content = "\n".join(lines).rstrip() + "\n"
    checksum = CONTENT_MARKER + hashlib.sha256(content.encode()).hexdigest() + "\n"
    rendered_lines = content.splitlines(keepends=True)
    rendered_lines.insert(4, checksum)
    return "".join(rendered_lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="regenerate and compare notices")
    parser.add_argument("--check-inputs", action="store_true", help="fast local packaging drift check")
    parser.add_argument("--cargo-about", default=os.environ.get("CARGO_ABOUT", "cargo-about"))
    parser.add_argument("--from-json", type=Path, nargs="+", help="render already harvested cargo-about JSON")
    args = parser.parse_args()
    try:
        if args.check_inputs:
            fast_check()
            return
        reports = ([json.loads(path.read_text(encoding="utf-8")) for path in args.from_json]
                   if args.from_json else collect(args.cargo_about))
        content = render(reports)
        if args.check:
            if OUTPUT.read_text(encoding="utf-8") != content:
                raise RuntimeError("THIRD_PARTY_NOTICES.txt is stale; regenerate it")
            print("Third-party notices are current.")
        else:
            OUTPUT.write_text(content, encoding="utf-8", newline="\n")
            print("Generated THIRD_PARTY_NOTICES.txt")
    except (RuntimeError, OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr, end="")
        print(f"License notice generation failed: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
