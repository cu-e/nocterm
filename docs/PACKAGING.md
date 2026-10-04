# Packaging

Release installers are built by [`.github/workflows/package.yml`](../.github/workflows/package.yml).
`release.yml` calls it for every release and attaches the files to the GitHub
release; it can also be started manually from the Actions tab. The version comes
from `version.txt`.

| Platform | Artifact | Tooling |
| --- | --- | --- |
| Windows 10 1809+ / 11 (x64) | `Nocterm-<version>-x64-setup.exe` | [Inno Setup 6](https://jrsoftware.org/isinfo.php), [`packaging/windows/nocterm.iss`](../packaging/windows/nocterm.iss) |
| Debian, Ubuntu | `nocterm_<version>_amd64.deb` | [cargo-deb](https://github.com/kornelski/cargo-deb) |
| Fedora, RHEL, openSUSE | `nocterm-<version>-1.x86_64.rpm` | [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm) |
| Any Linux | `Nocterm-<version>-x86_64.AppImage` | [appimagetool](https://github.com/AppImage/appimagetool) |

## Windows

The installer uses the modern wizard, installs per user without elevation by
default and offers an all-users installation on its first page. It adds a Start
menu entry, optional desktop shortcut and optional `PATH` entry, closes a running
Nocterm before upgrading and registers a standard uninstaller. Release builds use
the GUI subsystem, and `build.rs` embeds the icon and version information.

Build it on Windows:

```powershell
python3 scripts/generate-license-notices.py --check-inputs
cargo build --release --locked -p nocterm
iscc /DAppVersion=0.1.0 packaging\windows\nocterm.iss
```

GPUI compiles its DirectX shaders with `fxc.exe` from the Windows SDK, so the
application cannot be cross-compiled from Linux. Windows Hello device unlock
needs no installation step.

## Linux

```sh
cargo install --locked cargo-deb cargo-generate-rpm
# Requires curl and Python 3 for verified AppImage runtime downloads
# appimagetool: https://github.com/AppImage/appimagetool/releases
scripts/package-linux.sh            # all formats into target/packages
scripts/package-linux.sh deb rpm    # selected formats
```

The `.deb` and `.rpm` packages install:

| Path | Purpose |
| --- | --- |
| `/usr/bin/nocterm` | application |
| `/usr/libexec/nocterm/nocterm-vault-broker` | fingerprint vault broker |
| `/usr/lib/systemd/system/nocterm-vault-broker.service` | hardened broker unit |
| `/usr/share/dbus-1/system.d/dev.nocterm.VaultBroker1.conf` | system-bus policy |
| `/usr/share/dbus-1/system-services/dev.nocterm.VaultBroker1.service` | on-demand D-Bus activation |
| `/usr/share/applications/dev.nocterm.Nocterm.desktop` and icons | desktop entry |

The post-install script reloads systemd and the system bus, then enables and
starts the broker, so fingerprint unlock is available right after installation.
Both packages recommend `fprintd`. Removing the package stops and disables the
broker. See [DEVICE_UNLOCK.md](DEVICE_UNLOCK.md) for its security model.

An AppImage cannot install system services. It bundles the broker and installs
it after administrator authentication (pkexec) when asked explicitly:

```sh
./Nocterm-<version>-x86_64.AppImage --install-vault-broker
```

The Vault settings page shows this exact command when the broker is missing.

## License notices

All four installer formats include the application's `LICENSE` and
`THIRD_PARTY_NOTICES.txt`. The notices preserve third-party license texts and
copyright notices; they cover the application and the Linux vault broker,
including dependencies for supported platforms. They do not change the license
of any dependency or apply Nocterm's PolyForm license to upstream code.

On Windows both files are installed beside `nocterm.exe` (`LICENSE.txt` and
`THIRD_PARTY_NOTICES.txt`). Linux packages and AppImage place them under
`usr/share/doc/nocterm/` (inside the AppImage for that format).

Keep these files with redistributed binaries. Changes to `Cargo.lock`, vendored
forks, native dependencies or bundled assets require a notice review before a
release. A release Rust toolchain change also requires updating the matching
standard-library copyright snapshot in `licensing/supplemental/rust/` and
regenerating the notices; the fast check detects toolchain input changes. Native
FreeType version changes require reviewing its snapshot and attribution as well.
See [notice source provenance](../licensing/README.md). Preserve vendored license files and the per-file modification notices
when updating a fork. Release binary and installer extensions (`.exe`, `.deb`,
`.rpm`, `.AppImage`, including different letter case) are ignored throughout the
repository; built artifacts belong in `target/packages` and release attachments.

Regenerate the committed notices after reviewing dependency and asset changes:

```sh
cargo install --locked --version 0.9.2 --features cli cargo-about
python3 scripts/generate-license-notices.py
python3 scripts/generate-license-notices.py --check
```

Python 3 is required. Local packaging verifies notice inputs with
`python3 scripts/generate-license-notices.py --check-inputs`, without installing
cargo-about or compiling the application. The packaging workflow first checks
that regeneration matches the committed file; a stale or incomplete notice file
blocks packaging. Direct `cargo deb`, `cargo generate-rpm` or `iscc` invocations
must also use current notices.

The AppImage launcher is pinned separately in `licensing/appimage-runtime.json`.
Packaging downloads and checks the launcher and corresponding full source
archives into `target/packaging/appimage-runtime/`. It ships the launcher,
libfuse and squashfuse sources, the libfuse patch and build instructions under
`usr/share/doc/nocterm/appimage-runtime-sources/` inside the AppImage. See
[launcher license and relinking details](../licensing/supplemental/appimage/README.md).
The upstream Alpine build inputs float; exact binary reproduction has not been
verified.

The RPM License tag combines `LicenseRef-PolyForm-Perimeter-1.0.1` with
`LicenseRef-Nocterm-Third-Party`. The latter references the existing bundled
component terms in `THIRD_PARTY_NOTICES.txt`; it does not introduce a new license.
