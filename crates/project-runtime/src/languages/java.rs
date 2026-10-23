pub(super) const DISTRIBUTION: &str = r#"system_javac=$(PATH=/usr/bin:/bin command -v javac) || exit 77
jdk=$(dirname "$(dirname "$(readlink -f "$system_javac")")")
"$jdk/bin/java" -version
"$jdk/bin/javac" -version
link_managed "$jdk" "$base/java/current"
"#;
pub(super) const ADOPTIUM: &str = r#"case "$cpu" in x86_64) arch=x64;; aarch64) arch=aarch64;; *) exit 78;; esac
if test "$abi" = musl; then os=alpine-linux; else os=linux; fi
endpoint="https://api.adoptium.net/v3/binary/latest/$version/ga/$os/$arch/jdk/hotspot/normal/eclipse"
response=$(curl --proto '=https' --tlsv1.2 -sSI -o /dev/null -w '%{http_code}\n%{redirect_url}' "$endpoint") || exit 1
status=$(printf '%s\n' "$response" | head -n 1)
case "$status" in 404) exit 78;; 301|302|303|307|308) :;; *) exit 1;; esac
url=$(printf '%s\n' "$response" | tail -n 1)
case "$url" in https://github.com/adoptium/temurin*-binaries/releases/download/*.tar.gz) :;; *) exit 78;; esac
fetch "$url" "$work/jdk.tar.gz"
fetch "$url.sha256.txt" "$work/checksum"
checksum=$(awk 'NR == 1 {print $1}' "$work/checksum")
test "${#checksum}" -eq 64
case "$checksum" in *[!a-fA-F0-9]*) exit 1;; esac
printf '%s  %s\n' "$checksum" "$work/jdk.tar.gz" | sha256sum -c -
staged_target=$(mktemp -d "$base/java/releases/install.XXXXXX")
tar -xzf "$work/jdk.tar.gz" -C "$staged_target" --strip-components=1
actual=$("$staged_target/bin/javac" -version 2>&1)
case "$actual" in "javac $version"|"javac $version."*) :;; *) exit 78;; esac
"$staged_target/bin/java" -version
link_managed "$staged_target" "$base/java/current"
"#;
