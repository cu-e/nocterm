pub(super) const DISTRIBUTION: &str = r#"system_python=$(PATH=/usr/bin:/bin command -v python3) || exit 77
"$system_python" --version
"$system_python" -m pip --version
link_managed "$system_python" "$base/bin/python3"
link_managed "$system_python" "$base/bin/python"
"#;
pub(super) const MANAGED: &str = r#"fetch https://astral.sh/uv/install.sh "$work/install.sh"
UV_UNMANAGED_INSTALL="$work/bin" sh "$work/install.sh"
staged_target=$(mktemp -d "$base/python/releases/install.XXXXXX")
UV_PYTHON_INSTALL_DIR="$staged_target" UV_PYTHON_BIN_DIR="$work/pythonbin" "$work/bin/uv" python install --default "$version"
interpreter=$(readlink -f "$work/pythonbin/python3")
"$interpreter" -c 'import sys; want=tuple(map(int,sys.argv[1].split("."))); assert sys.version_info[:len(want)] == want' "$version"
"$work/bin/uv" pip install --python "$interpreter" --break-system-packages pip
"$interpreter" -m venv "$work/venv"
"$work/venv/bin/python" -m pip --version
link_managed "$interpreter" "$base/bin/python3"
link_managed "$interpreter" "$base/bin/python"
printf '%s\n' '#!/bin/sh' 'exec "$(dirname "$0")/python3" -m pip "$@"' > "$work/pip"
chmod 700 "$work/pip"
if test -e "$base/bin/pip" && test ! -f "$base/bin/pip" || test -L "$base/bin/pip"; then exit 79; fi
mv -f "$work/pip" "$base/bin/pip"
link_managed "$base/bin/pip" "$base/bin/pip3"
python --version
python3 -m pip --version
"#;
