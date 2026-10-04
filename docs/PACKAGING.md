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
cargo build --release -p nocterm
iscc /DAppVersion=0.1.0 packaging\windows\nocterm.iss
```

GPUI compiles its DirectX shaders with `fxc.exe` from the Windows SDK, so the
application cannot be cross-compiled from Linux. Windows Hello device unlock
needs no installation step.

## Linux

```sh
cargo install --locked cargo-deb cargo-generate-rpm
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
