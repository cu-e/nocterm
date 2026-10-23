// Only private Nocterm paths and newly allocated staging are mutated by managed recipes.
pub(super) const PREPARE: &str = r#"set -eu
umask 077
nocterm_home=$1; version=$2; abi=$3; cpu=$4
base="$nocterm_home/.local/share/nocterm/toolchains"
ensure_dir() {
    test ! -L "$1" || exit 79
    if test -e "$1"; then test -d "$1" && test -w "$1" || exit 79; else mkdir -m 700 "$1" || exit 79; fi
}
ensure_dir "$nocterm_home"
ensure_dir "$nocterm_home/.local"
ensure_dir "$nocterm_home/.local/share"
ensure_dir "$nocterm_home/.local/share/nocterm"
if test ! -e "$base" && test ! -L "$base"; then
    mkdir -m 700 "$base" || exit 79
    printf '%s\n' nocterm-toolchains-v1 > "$base/.nocterm-owned"
fi
ensure_dir "$base"
test -f "$base/.nocterm-owned" && test ! -L "$base/.nocterm-owned" || exit 79
test "$(cat "$base/.nocterm-owned")" = nocterm-toolchains-v1 || exit 79
test "$(stat -c %u "$base")" = "$(id -u)" || exit 79
for directory in bin python rust bun node java; do ensure_dir "$base/$directory"; done
for directory in python bun node java; do ensure_dir "$base/$directory/releases"; done
work=$(mktemp -d "$base/.stage.XXXXXX")
staged_target=
trap 'rm -rf "$work"; if test -n "$staged_target"; then rm -rf "$staged_target"; fi' EXIT HUP INT TERM
link_managed() {
    target=$1; destination=$2
    if test -L "$destination"; then case "$(readlink "$destination")" in "$base/"*|/usr/bin/*|/bin/*|/usr/lib/jvm/*) :;; *) exit 79;; esac
    elif test -e "$destination"; then exit 79; fi
    ln -s "$target" "$work/link"
    mv -Tf "$work/link" "$destination"
    if test -n "$staged_target"; then case "$target" in "$staged_target"|"$staged_target/"*) staged_target=;; esac; fi
}
fetch() {
    status=$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -sSL -o "$2" -w '%{http_code}' "$1") || return 1
    case "$status" in 200) :;; 404) exit 78;; *) return 1;; esac
}
export CARGO_HOME="$base/rust/cargo"
export RUSTUP_HOME="$base/rust/rustup"
export PATH="$base/bin:$CARGO_HOME/bin:$base/bun/current/bin:$base/node/current/bin:$base/java/current/bin:$PATH"
"#;
pub(super) const FINISH: &str = r#"cat > "$work/env.sh" <<'NOCTERM_ENV'
nocterm_toolchains="$HOME/.local/share/nocterm/toolchains"
export PATH="$nocterm_toolchains/bin:$nocterm_toolchains/rust/cargo/bin:$nocterm_toolchains/bun/current/bin:$nocterm_toolchains/node/current/bin:$nocterm_toolchains/java/current/bin:$PATH"
if test -x "$nocterm_toolchains/rust/cargo/bin/rustup"; then
    export CARGO_HOME="$nocterm_toolchains/rust/cargo"
    export RUSTUP_HOME="$nocterm_toolchains/rust/rustup"
fi
if test -x "$nocterm_toolchains/bun/current/bin/bun"; then export BUN_INSTALL="$nocterm_toolchains/bun/current"; fi
if test -x "$nocterm_toolchains/java/current/bin/java"; then export JAVA_HOME="$nocterm_toolchains/java/current"; fi
unset nocterm_toolchains
NOCTERM_ENV
if test -e "$base/env.sh" && test ! -f "$base/env.sh" || test -L "$base/env.sh"; then exit 79; fi
mv -f "$work/env.sh" "$base/env.sh"
staged_target=
rm -rf "$work"
trap - EXIT HUP INT TERM
"#;
