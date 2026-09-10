#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

is_python312() {
  "$1" -c 'import sys; raise SystemExit(0 if sys.version_info[:2] == (3, 12) else 1)' >/dev/null 2>&1
}

if [ -n "${PYTHON_EXE:-}" ] && is_python312 "$PYTHON_EXE"; then
  python_exe=$PYTHON_EXE
elif command -v python3.12 >/dev/null 2>&1 && is_python312 python3.12; then
  python_exe=python3.12
elif command -v python3 >/dev/null 2>&1 && is_python312 python3; then
  python_exe=python3
elif command -v python >/dev/null 2>&1 && is_python312 python; then
  python_exe=python
else
  echo "ERROR: Python 3.12 was not found. Run ./install.sh first to install and pin PYTHON_EXE." >&2
  exit 1
fi

exec "$python_exe" "$script_dir/setup_discord_bot.py" --repo-root "$script_dir" "$@"
