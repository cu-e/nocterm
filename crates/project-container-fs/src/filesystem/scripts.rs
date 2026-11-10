//! Fixed GNU/BusyBox recipes. User paths are arguments, never shell source.
pub(super) const COMMON: &str = r#"set -efu
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
identity() { stat -c '%d:%i' -- "$1"; }
exists() { test -e "$1" || test -L "$1"; }
checked() {
    case "$1" in /*) :;; *) exit 74;; esac
    case "$1/" in */../*|*/./*) exit 74;; esac
}
pin_dir() {
    checked "$1"
    rest=${1#/}
    cd -P / || exit 72
    while test -n "$rest"; do
        component=${rest%%/*}
        if test "$component" = "$rest"; then rest=; else rest=${rest#*/}; fi
        test -n "$component" || continue
        test ! -L "$component" || exit 75
        test -d "$component" || { exists "$component" && exit 74; exit 72; }
        before=$(identity "$component") || exit 71
        cd -P -- "$component" || exit 71
        test "$(identity .)" = "$before" || exit 75
    done
}
pin_parent() {
    checked "$1"
    leaf=${1##*/}
    parent=${1%/*}
    test -n "$parent" || parent=/
    test -n "$leaf" || exit 74
    pin_dir "$parent"
}
metadata() { stat -c '%d:%i:%f:%s:%a:%u:%g:%Y' -- "$1"; }
regular() { test ! -L "$1" && test -f "$1"; }
"#;
pub(super) const PROBE: &str = r#"for tool in stat find ln mv rm rmdir chmod mkdir mktemp cat head dd readlink grep; do command -v "$tool" >/dev/null 2>&1 || exit 76; done
stat -c '%d:%i:%f:%s:%a:%u:%g:%Y' / >/dev/null 2>&1 || exit 76
test -d /proc/self/fd || exit 76
ln --help 2>&1 | grep -q -- '-T' || exit 76
mv --help 2>&1 | grep -q -- '-T' || exit 76
find /dev/null -maxdepth 0 -print0 >/dev/null 2>&1 || exit 76
printf '%s' nocterm-fs-v1
"#;
pub(super) const HOME: &str = r#"case "${HOME:-}" in /*) printf '%s' "$HOME";; *) pwd -P;; esac
"#;
pub(super) const STAT: &str = r#"if test "$1" = /; then metadata /; else pin_parent "$1"; exists "$leaf" || exit 72; metadata "$leaf" || exit 71; fi
"#;
pub(super) const LIST: &str = r#"pin_dir "$1"
find . -mindepth 1 -maxdepth 1 -exec sh -c '
for entry do
    details=$(stat -c "%d:%i:%f:%s:%a:%u:%g:%Y" -- "$entry") || exit 71
    printf "%s\000%s\000" "${entry#./}" "$details"
done' nocterm-list {} +
"#;
pub(super) const CREATE_DIR: &str = r#"pin_parent "$1"
if exists "$leaf"; then test ! -L "$leaf" && test -d "$leaf" || exit 73; else mkdir -- "$leaf" || { exists "$leaf" && exit 73; exit 71; }; fi
"#;
pub(super) const REMOVE_FILE: &str = r#"pin_parent "$1"
exists "$leaf" || exit 72
test -L "$leaf" || test ! -d "$leaf" || exit 74
rm -- "$leaf" || exit 71
"#;
pub(super) const REMOVE_DIR: &str = r#"pin_parent "$1"
exists "$leaf" || exit 72
test ! -L "$leaf" && test -d "$leaf" || exit 74
rmdir -- "$leaf" || exit 71
"#;
pub(super) const PERMISSIONS: &str = r#"pin_parent "$1"
exists "$leaf" || exit 72
test ! -L "$leaf" || exit 76
chmod "$2" -- "$leaf" || exit 71
"#;
// Linux hard-link publication is atomic no-clobber. Directories need a different primitive.
pub(super) const RENAME: &str = r#"pin_parent "$1"
old_parent=$(identity .)
old_name=$leaf
regular "$old_name" || { exists "$old_name" || exit 72; exit 76; }
old_id=$(identity "$old_name") || exit 71
exec 3< .
pin_parent "$2"
new_name=$leaf
new_parent=$(identity .)
new_path=/proc/self/fd/3/$old_name
regular "$new_path" && test "$(identity "$new_path")" = "$old_id" || exit 75
if ln -T -- "$new_path" "$new_name"; then
    test "$(identity "$new_path")" = "$old_id" || exit 75
    rm -- "$new_path" || exit 71
else
    exists "$new_name" && exit 73
    exit 71
fi
"#;
pub(super) const PARENT: &str = r#"pin_parent "$1"
identity .
"#;
pub(super) const UPLOAD_CREATE: &str = r#"pin_parent "$1"
test "$(identity .)" = "$2" || exit 75
if test "$4" = create && exists "$leaf"; then exit 73; fi
if test "$4" = replace && exists "$leaf"; then regular "$leaf" || exit 73; fi
umask 077
mkdir -m 700 -- "$3" || exit 73
cd -P -- "$3" || exit 71
(set -C; : > data) || exit 71
printf '%s\000%s\000' "$(identity .)" "$(identity data)"
"#;
pub(super) const UPLOAD_CHECK: &str = r#"pin_dir "$1"
test "$(identity .)" = "$2" || exit 75
test ! -L "$3" && test -d "$3" && test "$(identity "$3")" = "$4" || exit 75
cd -P -- "$3" || exit 71
test "$(identity .)" = "$4" || exit 75
regular data && test "$(identity data)" = "$5" || exit 75
"#;
pub(super) const UPLOAD_WRITE: &str = r#"test "$(stat -c %s data)" = "$6" || exit 75
exec 3>>data
test -f /proc/self/fd/3 && test "$(stat -Lc '%d:%i' /proc/self/fd/3)" = "$5" || exit 75
cat >&3 || exit 71
exec 3>&-
test "$(stat -c %s data)" = "$7" || exit 75
printf '%s' "$7"
"#;
pub(super) const UPLOAD_PUBLISH: &str = r#"test "$(stat -c %s data)" = "$6" || exit 75
# Cwd and fd4 retain the admitted staging directory while returning to its parent.
exec 4< .
cd -P .. || exit 71
test "$(identity .)" = "$2" || exit 75
regular "/proc/self/fd/4/data" && test "$(identity "/proc/self/fd/4/data")" = "$5" || exit 75
if test "$8" = create; then
    if ! ln -T -- /proc/self/fd/4/data "$7"; then
        regular /proc/self/fd/4/data && test "$(identity /proc/self/fd/4/data)" = "$5" || exit 75
        exists "$7" && exit 73
        exit 71
    fi
    rm -- /proc/self/fd/4/data || exit 71
else
    if exists "$7"; then regular "$7" || exit 73; fi
    mv -T -- /proc/self/fd/4/data "$7" || exit 71
fi
test ! -L "$3" && test "$(identity "$3")" = "$4" || exit 75
rmdir -- "$3" || exit 71
printf '%s' published
"#;
pub(super) const UPLOAD_CLEAN: &str = r#"pin_dir "$1"
test "$(identity .)" = "$2" || exit 75
test ! -L "$3" && test -d "$3" && test "$(identity "$3")" = "$4" || exit 75
cd -P -- "$3" || exit 71
test "$(identity .)" = "$4" || exit 75
if exists data; then regular data && test "$(identity data)" = "$5" || exit 75; rm -- data || exit 71; fi
cd -P .. || exit 71
test "$(identity .)" = "$2" && test ! -L "$3" && test "$(identity "$3")" = "$4" || exit 75
rmdir -- "$3" || exit 71
"#;
pub(super) const DOWNLOAD: &str = r#"pin_parent "$1"
exists "$leaf" || exit 72
regular "$leaf" || exit 76
admitted=$(identity "$leaf") || exit 71
exec 3<"$leaf"
test -f /proc/self/fd/3 && test "$(stat -Lc '%d:%i' /proc/self/fd/3)" = "$admitted" || exit 75
printf '%s\000' "$(stat -Lc %s /proc/self/fd/3)"
cat <&3
"#;
