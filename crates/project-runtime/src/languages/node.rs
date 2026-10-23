pub(super) const DISTRIBUTION: &str = r#"system_node=$(PATH=/usr/bin:/bin command -v node) || exit 77
system_npm=$(PATH=/usr/bin:/bin command -v npm) || exit 77
"$system_node" --version
"$system_npm" --version
link_managed "$system_node" "$base/bin/node"
link_managed "$system_npm" "$base/bin/npm"
"#;
pub(super) const OFFICIAL: &str = r#"test "$abi" = gnu || exit 78
case "$cpu" in x86_64) arch=x64;; aarch64) arch=arm64;; *) exit 78;; esac
case "$version" in *.*) release="v$version";; *) release="latest-v$version.x";; esac
url="https://nodejs.org/dist/$release"
fetch "$url/SHASUMS256.txt" "$work/SHASUMS256.txt"
archive=$(awk -v arch="$arch" '$2 ~ ("^node-v[0-9]+\\.[0-9]+\\.[0-9]+-linux-" arch "\\.tar\\.xz$") {print $2}' "$work/SHASUMS256.txt")
test "$(printf '%s\n' "$archive" | wc -l)" -eq 1 && test -n "$archive"
case "$version" in *.*) test "$archive" = "node-v$version-linux-$arch.tar.xz";; *) case "$archive" in "node-v$version."*) :;; *) exit 78;; esac;; esac
fetch "$url/$archive" "$work/$archive"
(cd "$work"; awk -v file="$archive" '$2 == file {print}' SHASUMS256.txt | sha256sum -c -)
staged_target=$(mktemp -d "$base/node/releases/install.XXXXXX")
tar -xJf "$work/$archive" -C "$staged_target" --strip-components=1
actual=$("$staged_target/bin/node" --version)
case "$version" in *.*) test "$actual" = "v$version";; *) case "$actual" in "v$version."*) :;; *) exit 78;; esac;; esac
"$staged_target/bin/node" "$staged_target/lib/node_modules/npm/bin/npm-cli.js" --version
link_managed "$staged_target" "$base/node/current"
link_managed "$base/node/current/bin/node" "$base/bin/node"
link_managed "$base/node/current/bin/npm" "$base/bin/npm"
"#;
