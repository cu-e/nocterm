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
allocate_release() {
    releases=$1
    count=$(find "$releases" -mindepth 1 -maxdepth 1 -type d -name 'install.*' -print | wc -l)
    test "$count" -lt 8 || exit 80
    mktemp -d "$releases/install.XXXXXX"
}
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
"#;
pub(super) fn finish() -> String {
    format!(
        "cat > \"$work/env.sh\" <<'NOCTERM_ENV'\n{}{}",
        crate::managed::shell(),
        FINISH
    )
}
const FINISH: &str = r#"NOCTERM_ENV
if test -e "$base/env.sh" && test ! -f "$base/env.sh" || test -L "$base/env.sh"; then exit 79; fi
mv -f "$work/env.sh" "$base/env.sh"
staged_target=
rm -rf "$work"
trap - EXIT HUP INT TERM
"#;
