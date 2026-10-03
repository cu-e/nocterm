#!/bin/sh
# Explicit administrator installation; never invoked by the application.
set -eu
if [ "$(id -u)" != 0 ]; then
    echo 'Run this installer as administrator after building nocterm-vault-broker.' >&2
    exit 1
fi
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
broker_binary=${1:-"$repo_dir/target/release/nocterm-vault-broker"}
if [ ! -f "$broker_binary" ]; then
    echo 'First run: cargo build --release -p nocterm-vault-broker' >&2
    exit 1
fi
install -o root -g root -m 0755 -d /usr/local/libexec
install -o root -g root -m 0755 "$broker_binary" /usr/local/libexec/nocterm-vault-broker
install -o root -g root -m 0644 "$repo_dir/packaging/linux/dev.nocterm.VaultBroker1.conf" /etc/dbus-1/system.d/dev.nocterm.VaultBroker1.conf
install -o root -g root -m 0644 "$repo_dir/packaging/linux/nocterm-vault-broker.service" /etc/systemd/system/nocterm-vault-broker.service
systemctl daemon-reload
systemctl enable --now nocterm-vault-broker.service
