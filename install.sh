#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python_exe=${PYTHON_EXE:-}
codex_exe=${CODEX_EXE:-}
codex_exe_was_explicit=0
codex_home=${CODEX_HOME:-}
skip_dependencies=0
skip_env_file=0
skip_steering_config=0
skip_codex_plugin=0
dry_run=0
required_python_major=3
required_python_minor=12
plugin_manifest_path="$script_dir/plugins/codex-discord-remote/.codex-plugin/plugin.json"
plugin_inventory_verifier_path="$script_dir/verify_codex_plugin_inventory.py"
plugin_marketplace_name=codex-discord-remote
plugin_ref=codex-discord-remote@codex-discord-remote

usage() {
  echo "Usage: ./install.sh [--python-exe PATH] [--codex-exe PATH] [--codex-home PATH] [--skip-dependencies] [--skip-env-file] [--skip-steering-config] [--skip-codex-plugin] [--dry-run]" >&2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --python-exe)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      python_exe=$2
      shift 2
      ;;
    --codex-exe)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      codex_exe=$2
      codex_exe_was_explicit=1
      shift 2
      ;;
    --codex-home)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      codex_home=$2
      shift 2
      ;;
    --skip-dependencies)
      skip_dependencies=1
      shift
      ;;
    --skip-env-file)
      skip_env_file=1
      shift
      ;;
    --skip-steering-config)
      skip_steering_config=1
      shift
      ;;
    --skip-codex-plugin)
      skip_codex_plugin=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

resolve_python() {
  if [ -n "$python_exe" ]; then
    if is_python312 "$python_exe"; then
      printf '%s\n' "$python_exe"
      return 0
    fi
    echo "PYTHON_EXE must point to Python 3.12.x: $python_exe" >&2
    exit 1
  fi

  for candidate in python3.12 python3 python; do
    if command -v "$candidate" >/dev/null 2>&1 && is_python312 "$candidate"; then
      command -v "$candidate"
      return 0
    fi
  done

  require_python312
  if [ "$dry_run" -eq 1 ]; then
    printf '%s\n' "python3.12"
    return 0
  fi
  for candidate in python3.12 python3 python; do
    if command -v "$candidate" >/dev/null 2>&1 && is_python312 "$candidate"; then
      command -v "$candidate"
      return 0
    fi
  done

  echo "Python 3.12.x was not found. Install Python 3.12, then rerun ./install.sh." >&2
  exit 1
}

is_python312() {
  "$1" -c "import sys; raise SystemExit(0 if sys.version_info[:2] == ($required_python_major, $required_python_minor) else 1)" >/dev/null 2>&1
}

require_python312() {
  if [ "$dry_run" -eq 1 ]; then
    echo "Would require Python 3.12 on PATH or --python-exe" >&2
    return 0
  fi
  echo "Python 3.12.x was not found. Install Python 3.12 or pass --python-exe, then rerun ./install.sh." >&2
  exit 1
}

python_executable_path() {
  "$1" -c "import sys; print(sys.executable)"
}

run_python() {
  py=$(resolve_python)
  if [ "$dry_run" -eq 1 ]; then
    echo "Would run: $py $*"
    return 0
  fi
  "$py" "$@"
}

get_env_value() {
  name=$1
  env_path="$script_dir/.env"
  [ -f "$env_path" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ''|\#*) continue ;;
      "$name="*)
        value=${line#*=}
        value=${value%\"}
        value=${value#\"}
        printf '%s\n' "$value"
        return 0
        ;;
    esac
  done < "$env_path"
}

set_env_value() {
  name=$1
  value=$2
  env_path="$script_dir/.env"
  tmp_path="$env_path.tmp"
  if [ -f "$env_path" ]; then
    awk -v name="$name" -v value="$value" '
      BEGIN { found = 0 }
      /^[[:space:]]*($|#)/ { print; next }
      index($0, "=") {
        key = $0
        sub(/=.*/, "", key)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
        if (key == name) {
          print name "=" value
          found = 1
          next
        }
      }
      { print }
      END {
        if (!found) {
          print name "=" value
        }
      }
    ' "$env_path" > "$tmp_path"
    mv "$tmp_path" "$env_path"
  else
    printf '%s=%s\n' "$name" "$value" > "$env_path"
  fi
}

resolve_codex() {
  codex=$(find_codex)
  if [ -n "$codex" ]; then
    printf '%s\n' "$codex"
    return 0
  fi
  echo "Codex CLI was not found. Set CODEX_EXE or install/enable the codex command." >&2
  return 1
}

find_codex() {
  if [ -n "$codex_exe" ]; then
    printf '%s\n' "$codex_exe"
    return 0
  fi
  env_codex=$(get_env_value CODEX_EXE || true)
  if [ -n "$env_codex" ]; then
    printf '%s\n' "$env_codex"
    return 0
  fi
  if command -v codex >/dev/null 2>&1; then
    command -v codex
    return 0
  fi
  return 0
}

resolve_codex_home() {
  default_codex_home="$HOME/.codex"
  if [ -n "$codex_home" ]; then
    case "$codex_home" in
      "~") resolved="$HOME" ;;
      "~/"*) resolved="$HOME/${codex_home#~/}" ;;
      *) resolved="$codex_home" ;;
    esac
    case "$(printf '%s' "$resolved" | tr '[:upper:]' '[:lower:]')" in
      *'/.sandbox-bin'*|*'/plugins/.plugin-appserver'*|*'/appdata/local/openai/codex/bin'|*'/app/resources')
        printf '%s\n' "$default_codex_home"
        ;;
      *)
        printf '%s\n' "$resolved"
        ;;
    esac
    return 0
  fi
  printf '%s\n' "$default_codex_home"
}

run_codex() {
  exe=$(resolve_codex)
  if [ "$dry_run" -eq 1 ]; then
    echo "Would run: $exe $*"
    return 0
  fi
  "$exe" "$@"
}

verify_codex_plugin_inventory() {
  if [ "$dry_run" -eq 1 ]; then
    echo "Would verify Codex marketplace and plugin inventory for $plugin_ref"
    return 0
  fi

  marketplace_inventory_path=$(mktemp "${TMPDIR:-/tmp}/codex-marketplaces.XXXXXX") || {
    echo "INSTALL_INCOMPLETE: could not create marketplace inventory file." >&2
    return 1
  }
  plugin_inventory_path=$(mktemp "${TMPDIR:-/tmp}/codex-plugins.XXXXXX") || {
    rm -f -- "$marketplace_inventory_path"
    echo "INSTALL_INCOMPLETE: could not create plugin inventory file." >&2
    return 1
  }
  trap 'rm -f -- "$marketplace_inventory_path" "$plugin_inventory_path"' 0 HUP INT TERM

  if ! run_codex plugin marketplace list --json > "$marketplace_inventory_path"; then
    echo "INSTALL_INCOMPLETE: Codex marketplace inventory query failed." >&2
    return 1
  fi
  if ! run_codex plugin list --json > "$plugin_inventory_path"; then
    echo "INSTALL_INCOMPLETE: Codex plugin inventory query failed." >&2
    return 1
  fi
  if ! run_python "$plugin_inventory_verifier_path" \
    --marketplace-inventory "$marketplace_inventory_path" \
    --plugin-inventory "$plugin_inventory_path" \
    --plugin-manifest "$plugin_manifest_path" \
    --expected-root "$script_dir" \
    --marketplace-name "$plugin_marketplace_name" \
    --plugin-id "$plugin_ref"; then
    return 1
  fi

  rm -f -- "$marketplace_inventory_path" "$plugin_inventory_path"
  trap - 0 HUP INT TERM
}

if [ "$skip_dependencies" -eq 0 ]; then
  [ -f "$script_dir/requirements.txt" ] || { echo "requirements.txt was not found: $script_dir/requirements.txt" >&2; exit 1; }
  echo "Installing Python dependencies from requirements.txt"
  run_python -m pip install --require-hashes -r "$script_dir/requirements.txt"
fi

if [ "$skip_env_file" -eq 0 ]; then
  if [ -f "$script_dir/.env" ]; then
    echo ".env already exists: $script_dir/.env"
  elif [ -f "$script_dir/.env.example" ]; then
    if [ "$dry_run" -eq 1 ]; then
      echo "Would create: $script_dir/.env from .env.example"
    else
      cp "$script_dir/.env.example" "$script_dir/.env"
      echo "Created: $script_dir/.env"
    fi
  else
    echo ".env.example was not found; skipping .env creation."
  fi

  if [ "$dry_run" -eq 1 ]; then
    echo "Would set PYTHON_EXE to the resolved Python 3.12 executable in .env"
    echo "Would set CODEX_HOME to the resolved Codex home path in .env"
    existing_codex_command=$(get_env_value CODEX_EXE || true)
    if [ "$codex_exe_was_explicit" -eq 1 ]; then
      echo "Would set explicitly configured CODEX_EXE in .env"
    elif [ -n "$existing_codex_command" ]; then
      echo "Would preserve existing CODEX_EXE in .env"
    elif [ -n "$codex_exe" ]; then
      echo "Inherited CODEX_EXE would not be saved to .env"
    else
      echo "Would leave CODEX_EXE unchanged; PATH-discovered Codex commands are not saved to .env"
    fi
  else
    py=$(resolve_python)
    py_path=$(python_executable_path "$py")
    set_env_value PYTHON_EXE "$py_path"
    echo "Configured PYTHON_EXE=$py_path"
    codex_home_path=$(resolve_codex_home)
    set_env_value CODEX_HOME "$codex_home_path"
    echo "Configured CODEX_HOME=$codex_home_path"
    if [ "$codex_exe_was_explicit" -eq 1 ]; then
      set_env_value CODEX_EXE "$codex_exe"
      echo "Configured explicit CODEX_EXE=$codex_exe"
    else
      existing_codex_command=$(get_env_value CODEX_EXE || true)
      if [ -n "$existing_codex_command" ]; then
        echo "Preserved existing CODEX_EXE in .env."
      elif [ -n "$codex_exe" ]; then
        echo "Inherited CODEX_EXE was not saved to .env."
      elif command -v codex >/dev/null 2>&1; then
        echo "PATH-discovered Codex command was not saved to .env."
      else
        echo "CODEX_EXE remains unset; no Codex command was found on PATH."
      fi
    fi
  fi
fi

echo "Discovering Codex Desktop executable."
run_python "$script_dir/codex_desktop_bridge.py" discover_codex

if [ "$skip_steering_config" -eq 1 ]; then
  echo "Skipping steering config: installer no longer changes Codex Desktop follow-up mode."
fi

if [ "$skip_codex_plugin" -eq 1 ]; then
  echo "Skipping Codex plugin install."
else
  [ -f "$script_dir/.agents/plugins/marketplace.json" ] || { echo "Codex plugin marketplace was not found: $script_dir/.agents/plugins/marketplace.json" >&2; exit 1; }
  [ -f "$plugin_manifest_path" ] || { echo "INSTALL_INCOMPLETE: Codex plugin manifest was not found: $plugin_manifest_path" >&2; exit 1; }
  [ -f "$plugin_inventory_verifier_path" ] || { echo "INSTALL_INCOMPLETE: Codex plugin inventory verifier was not found: $plugin_inventory_verifier_path" >&2; exit 1; }
  echo "Installing Codex plugin marketplace from this repository."
  run_codex plugin marketplace add "$script_dir" || { echo "INSTALL_INCOMPLETE: Codex plugin marketplace installation failed; !pro is not ready." >&2; exit 1; }
  run_codex plugin add "$plugin_ref" || { echo "INSTALL_INCOMPLETE: Codex plugin installation failed; !pro is not ready." >&2; exit 1; }
  verify_codex_plugin_inventory
fi

if [ "$dry_run" -eq 1 ]; then
  echo "Dry run complete."
  echo "Plugin inventory was not verified."
else
  echo "Install complete."
  echo "Setup required: run ./setup-discord-bot.sh and paste the Discord bot token when prompted."
  echo "After setup, restart Codex so bundled skills reload, then run ./codex-discord-bot.sh or the platform launcher."
fi
