#!/bin/sh
# Explicit administrator installation for source builds and the AppImage.
# The .deb and .rpm packages install and start the broker themselves.
#
# Usage: install-vault-broker.sh [broker-binary [packaging-dir]]
set -eu
if [ "$(id -u)" != 0 ]; then
    echo 'Run this installer as administrator after building nocterm-vault-broker.' >&2
    exit 1
fi
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
broker_binary=${1:-"$repo_dir/target/release/nocterm-vault-broker"}
packaging_dir=${2:-"$repo_dir/packaging/linux"}
if [ ! -f "$broker_binary" ]; then
    echo 'First run: cargo build --release -p nocterm-vault-broker' >&2
    exit 1
fi
install -o root -g root -m 0755 -d /usr/local/libexec/nocterm
install -o root -g root -m 0755 "$broker_binary" /usr/local/libexec/nocterm/nocterm-vault-broker
install -o root -g root -m 0644 "$packaging_dir/dev.nocterm.VaultBroker1.conf" /etc/dbus-1/system.d/dev.nocterm.VaultBroker1.conf
sed 's#^ExecStart=.*#ExecStart=/usr/local/libexec/nocterm/nocterm-vault-broker#' \
    "$packaging_dir/nocterm-vault-broker.service" > /etc/systemd/system/nocterm-vault-broker.service
chmod 0644 /etc/systemd/system/nocterm-vault-broker.service
systemctl daemon-reload
systemctl reload dbus.service 2>/dev/null || systemctl reload dbus-broker.service 2>/dev/null || true
systemctl enable nocterm-vault-broker.service
systemctl restart nocterm-vault-broker.service
echo 'Nocterm vault broker installed. Open Settings → Vault and turn on Fingerprint.'
