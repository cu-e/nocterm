#!/bin/sh
# Builds Linux release packages into target/packages: .deb, .rpm and AppImage.
# Requires cargo-deb, cargo-generate-rpm and appimagetool (see docs/PACKAGING.md).
#
# Usage: scripts/package-linux.sh [deb] [rpm] [appimage]   (default: all)
set -eu
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"
formats=${*:-deb rpm appimage}
version=$(cat version.txt)
arch=$(uname -m)
out=target/packages
mkdir -p "$out"

cargo build --release --locked -p nocterm -p nocterm-vault-broker

for format in $formats; do
    case "$format" in
    deb)
        cargo deb --no-build --no-strip --deb-version "$version" -o "$out/"
        ;;
    rpm)
        cargo generate-rpm --set-metadata "version = \"$version\"" -o "$out/"
        ;;
    appimage)
        appdir=target/packaging/Nocterm.AppDir
        rm -rf "$appdir"
        install -Dm755 target/release/nocterm "$appdir/usr/bin/nocterm"
        install -Dm755 target/release/nocterm-vault-broker "$appdir/usr/libexec/nocterm/nocterm-vault-broker"
        install -Dm755 scripts/install-vault-broker.sh "$appdir/usr/share/nocterm/install-vault-broker.sh"
        install -Dm644 LICENSE "$appdir/usr/share/doc/nocterm/LICENSE"
        for file in dev.nocterm.VaultBroker1.conf nocterm-vault-broker.service; do
            install -Dm644 "packaging/linux/$file" "$appdir/usr/share/nocterm/packaging/$file"
        done
        install -Dm644 packaging/linux/dev.nocterm.Nocterm.desktop "$appdir/usr/share/applications/dev.nocterm.Nocterm.desktop"
        install -Dm644 assets/icons/nocterm-256.png "$appdir/usr/share/icons/hicolor/256x256/apps/dev.nocterm.Nocterm.png"
        install -Dm644 assets/icons/nocterm.svg "$appdir/usr/share/icons/hicolor/scalable/apps/dev.nocterm.Nocterm.svg"
        install -Dm755 packaging/linux/appimage/AppRun "$appdir/AppRun"
        cp packaging/linux/dev.nocterm.Nocterm.desktop "$appdir/"
        cp assets/icons/nocterm-256.png "$appdir/dev.nocterm.Nocterm.png"
        ln -sf dev.nocterm.Nocterm.png "$appdir/.DirIcon"
        ARCH="$arch" appimagetool --no-appstream "$appdir" "$out/Nocterm-$version-$arch.AppImage"
        ;;
    *)
        echo "Unknown package format: $format" >&2
        exit 1
        ;;
    esac
done
ls -l "$out"
