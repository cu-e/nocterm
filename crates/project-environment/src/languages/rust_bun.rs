pub(super) const RUST: &str = r#"ensure_dir "$CARGO_HOME"
ensure_dir "$RUSTUP_HOME"
fetch https://sh.rustup.rs "$work/rustup.sh"
case "$cpu" in x86_64|aarch64) :;; *) exit 78;; esac
host="$cpu-unknown-linux-$abi"
sh "$work/rustup.sh" -y --no-modify-path --profile minimal --default-host "$host" --default-toolchain "$version"
if test "$version" != stable; then test "$(rustc --version | cut -d ' ' -f 2)" = "$version"; fi
rustc --version
cargo --version
"#;
pub(super) const BUN: &str = r#"case "$cpu" in x86_64) arch=x64;; aarch64) arch=aarch64;; *) exit 78;; esac
platform="bun-linux-$arch"
if test "$abi" = musl; then platform="$platform-musl"; fi
if test "$arch" = x64; then platform="$platform-baseline"; fi
if test "$version" = latest; then release=latest/download; else release="download/bun-v$version"; fi
fetch "https://github.com/oven-sh/bun/releases/$release/$platform.zip" "$work/bun.zip"
unzip -q "$work/bun.zip" -d "$work/unpacked"
staged_target=$(mktemp -d "$base/bun/releases/install.XXXXXX")
mkdir -m 700 "$staged_target/bin"
cp "$work/unpacked/$platform/bun" "$staged_target/bin/bun"
chmod 700 "$staged_target/bin/bun"
if test "$version" != latest; then test "$("$staged_target/bin/bun" --version)" = "$version"; fi
"$staged_target/bin/bun" --version
link_managed "$staged_target" "$base/bun/current"
"#;
