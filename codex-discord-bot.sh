#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
script="$script_dir/codex_discord_bot.py"
env_file="$script_dir/.env"
lock_dir="$script_dir/.codex_discord_bot.lock"
pid_file="$lock_dir/launcher.pid"
launcher_log_path="$script_dir/discord_launcher.log"

log_launcher() {
  message=$1
  timestamp=$(date '+%Y-%m-%dT%H:%M:%S')
  printf '[%s] %s\n' "$timestamp" "$message" >> "$launcher_log_path"
}

load_env_value() {
  name=$1
  [ -f "$env_file" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "$name="*)
        value=${line#*=}
        value=${value%\"}
        value=${value#\"}
        printf '%s\n' "$value"
        return 0
        ;;
    esac
  done < "$env_file"
}

export_env_value_if_missing() {
  name=$1
  eval "current=\${$name:-}"
  [ -n "$current" ] && return 0
  value=$(load_env_value "$name" || true)
  [ -n "$value" ] || return 0
  export "$name=$value"
}

for env_name in PYTHON_EXE CODEX_HOME CODEX_EXE CODEX_DESKTOP_EXE CODEX_DISCORD_LOG_PATH; do
  export_env_value_if_missing "$env_name"
done

log_path=${CODEX_DISCORD_LOG_PATH:-"$script_dir/codex_discord_bot.log"}
python_exe=${PYTHON_EXE:-}

resolve_python() {
  if [ -n "$python_exe" ]; then
    if is_python312 "$python_exe"; then
      printf '%s\n' "$python_exe"
      return 0
    fi
    echo "ERROR: PYTHON_EXE must point to Python 3.12.x: $python_exe" >&2
    exit 1
  else
    env_python=$(load_env_value PYTHON_EXE || true)
    if [ -n "$env_python" ] && is_python312 "$env_python"; then
      printf '%s\n' "$env_python"
      return 0
    fi
    for candidate in python3.12 python3 python; do
      if command -v "$candidate" >/dev/null 2>&1 && is_python312 "$candidate"; then
        command -v "$candidate"
        return 0
      fi
    done
    echo "ERROR: Python 3.12 was not found. Run ./install.sh to install and pin PYTHON_EXE." >&2
    exit 1
  fi
}

is_python312() {
  "$1" -c 'import sys; raise SystemExit(0 if sys.version_info[:2] == (3, 12) else 1)' >/dev/null 2>&1
}

pid_alive() {
  pid=$1
  [ -n "$pid" ] || return 1
  kill -0 "$pid" >/dev/null 2>&1
}

if [ ! -f "$script" ]; then
  echo "ERROR: Script not found: $script" >&2
  exit 1
fi

echo
echo "Codex Discord frontend bridge is starting."
echo "Script: $script"
echo "Log:    $log_path"
echo
log_launcher "visible_start script=$script log=$log_path"

if mkdir "$lock_dir" 2>/dev/null; then
  :
else
  existing_pid=""
  [ -f "$pid_file" ] && existing_pid=$(cat "$pid_file")
  if pid_alive "$existing_pid"; then
    echo "Codex Discord bot is already running."
    echo "Log: $log_path"
    log_launcher "already_running pid=$existing_pid script=$script log=$log_path"
    exit 0
  fi
  echo "Removing stale Codex Discord bot lock."
  rm -rf "$lock_dir"
  mkdir "$lock_dir"
fi

cleanup() {
  rm -rf "$lock_dir"
}
trap cleanup EXIT INT TERM
printf '%s\n' "$$" > "$pid_file"

py=$(resolve_python)
log_launcher "run python_exe=$py script=$script"
set +e
"$py" "$script" "$@"
exit_code=$?
set -e
log_launcher "exit code=$exit_code script=$script"
exit "$exit_code"
