#!/usr/bin/env sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
binary_path=""
codex_exe=${CODEX_EXE:-}
codex_home=${CODEX_HOME:-}
explicit_exe=0
explicit_home=0
skip_build=0
skip_env=0
skip_plugin=0
dry_run=0
usage() {
  echo "Usage: ./install.sh [--binary-path PATH] [--skip-build] [--codex-exe PATH] [--codex-home PATH] [--skip-env-file] [--skip-codex-plugin] [--dry-run]" >&2
}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary-path) [ "$#" -ge 2 ] || { usage; exit 2; }; binary_path=$2; shift 2 ;;
    --codex-exe) [ "$#" -ge 2 ] || { usage; exit 2; }; codex_exe=$2; explicit_exe=1; shift 2 ;;
    --codex-home) [ "$#" -ge 2 ] || { usage; exit 2; }; codex_home=$2; explicit_home=1; shift 2 ;;
    --skip-build) skip_build=1; shift ;;
    --skip-env-file) skip_env=1; shift ;;
    --skip-codex-plugin) skip_plugin=1; shift ;;
    --skip-dependencies|--skip-steering-config) echo "Compatibility option: Cargo manages dependencies; Codex follow-up settings are unchanged."; shift ;;
    --dry-run) dry_run=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done
target=${CARGO_TARGET_DIR:-target}
case "$target" in /*|[A-Za-z]:[\\/]*) ;; *) target="$script_dir/$target" ;; esac
[ -n "$binary_path" ] || binary_path="$target/release/cdr-runtime"
if [ "$skip_build" -eq 0 ] && [ "$binary_path" != "$target/release/cdr-runtime" ]; then
  echo "Binary path differs from Cargo build output. Set CARGO_TARGET_DIR or use --skip-build with an already verified artifact." >&2
  exit 1
fi
canonical="$script_dir/target/release/cdr-runtime"
run() {
  if [ "$dry_run" -eq 1 ]; then printf 'Would run: %s\n' "$*"; else "$@"; fi
}
checked_codex() {
  if CODEX_HOME="$codex_home" "$codex_exe" "$@"; then return 0; else
    code=$?
    echo "INSTALL_INCOMPLETE: required Codex plugin command failed (exit $code). Update Codex or pass --codex-exe with a compatible CLI." >&2
    return "$code"
  fi
}
checked_inventory() {
  inventory_file=$1
  shift
  if checked_codex "$@" > "$inventory_file"; then return 0; else
    code=$?
    cat "$inventory_file" >&2
    return "$code"
  fi
}
get_env_value() {
  [ -f "$script_dir/.env" ] || return 0
  awk -v wanted="$1" '
    /^[[:space:]]*#/ { next }
    index($0, "=") {
      key=substr($0,1,index($0,"=")-1); gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
      if(key==wanted) {
        value=substr($0,index($0,"=")+1); gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
        gsub(/^["\047]|["\047]$/, "", value); print value; exit
      }
    }' "$script_dir/.env"
}
saved_home=$(get_env_value CODEX_HOME)
if [ "$explicit_home" -eq 0 ] && [ -n "$saved_home" ]; then codex_home=$saved_home; fi
[ -n "$codex_home" ] || codex_home="$HOME/.codex"
case "$codex_home" in '~') codex_home="$HOME" ;; '~/'*) codex_home="$HOME/${codex_home#\~/}" ;; esac
# Resolve once, before writes. The saved profile and every Codex child must use
# the same absolute directory even when installation runs outside the repository.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    profile_absolute=$(realpath -m -- "$codex_home")
    codex_home=$(cygpath -am -- "$profile_absolute")
    profile_guard=$(printf '%s' "$codex_home" | tr '[:upper:]' '[:lower:]') ;;
  *)
    case "$codex_home" in /*) ;; *) codex_home="$(pwd -P)/$codex_home" ;; esac
    codex_home=$(printf '%s\n' "$codex_home" | awk -F / '
      { for (i=1; i<=NF; i++) {
          if ($i=="" || $i==".") continue
          if ($i=="..") { if(n>0) n--; continue }
          parts[++n]=$i
        }
        printf "/"; for(i=1; i<=n; i++) printf "%s%s", parts[i], (i<n ? "/" : "")
      }')
    profile_guard=$codex_home ;;
esac
profile_guard=$(printf '%s' "$profile_guard" | sed 's:/*$::')
case "$profile_guard/" in
  *'/.sandbox-bin/'*|*'/plugins/.plugin-appserver/'*|*'/app/resources/'*|*'/appdata/local/openai/codex/bin/'*)
    echo "CODEX_HOME points at a runtime executable directory, not a Codex profile." >&2; exit 1 ;;
esac
if [ "$skip_build" -eq 0 ]; then
  if [ "$dry_run" -eq 0 ] && [ -f "$canonical" ]; then
    echo "An installed runtime exists at the build destination; build externally and use verified deployment." >&2
    exit 1
  fi
  (cd "$script_dir"; run cargo build --release --locked -p cdr-runtime --bin cdr-runtime)
fi
if [ "$dry_run" -eq 0 ] && [ ! -x "$binary_path" ]; then
  echo "Rust runtime not found or not executable: $binary_path" >&2
  exit 1
fi
if [ "$dry_run" -eq 1 ]; then
  echo "Would verify the runtime at the launcher path without replacing an installed version."
elif [ -e "$canonical" ]; then
  if ! cmp -s "$binary_path" "$canonical"; then
    echo "Existing runtime differs; use verified deployment. The installed bot was not stopped or changed." >&2
    exit 1
  fi
else
  (
    mkdir -p "$(dirname -- "$canonical")"
    staged=$(mktemp "$canonical.install.XXXXXX")
    trap 'rm -f -- "$staged"' 0
    cp "$binary_path" "$staged"
    chmod +x "$staged"
    cmp "$binary_path" "$staged"
    # An atomic hard-link publication refuses a concurrently installed version.
    ln "$staged" "$canonical"
  )
fi
if [ "$skip_env" -eq 0 ]; then
  if [ ! -f "$script_dir/.env" ]; then
    run cp "$script_dir/.env.example" "$script_dir/.env"
  fi
  if [ "$explicit_exe" -eq 1 ]; then
    run "$binary_path" --admin configure-install --repo-root "$script_dir" --codex-home "$codex_home" --codex-exe "$codex_exe"
  else
    run "$binary_path" --admin configure-install --repo-root "$script_dir" --codex-home "$codex_home"
    if [ -n "$codex_exe" ]; then
      echo "Inherited CODEX_EXE was not saved; existing explicit settings remain unchanged."
    else
      echo "PATH-discovered Codex command was not saved; automatic executable discovery remains enabled."
    fi
  fi
fi
if [ "$dry_run" -eq 1 ]; then
  echo "Would discover Codex with Rust management."
elif [ -n "$codex_exe" ]; then
  codex_exe=$("$binary_path" --admin discover-codex --repo-root "$script_dir" --codex-exe "$codex_exe")
else
  codex_exe=$("$binary_path" --admin discover-codex --repo-root "$script_dir")
fi
if [ "$skip_plugin" -eq 0 ]; then
  if [ "$skip_build" -eq 0 ]; then (cd "$script_dir"; run cargo build --release --locked -p cdr-pro --bin cdr-pro-helper); fi
  helper="$(dirname -- "$binary_path")/cdr-pro-helper"
  plugin="$script_dir/plugins/codex-discord-remote"
  if [ "$dry_run" -eq 1 ]; then
    echo "Would stage and verify the Rust Pro helper before atomic publication."
    echo "Would install and verify Codex plugin inventory."
  else
    (
      destination="$plugin/bin/cdr-pro-helper"
      mkdir -p "$plugin/bin"
      if [ -f "$destination" ] && cmp -s "$helper" "$destination"; then exit 0; fi
      staged_helper=$(mktemp "$destination.install.XXXXXX")
      trap 'rm -f -- "$staged_helper"' 0
      cp "$helper" "$staged_helper"
      chmod +x "$staged_helper"
      if ! cmp "$helper" "$staged_helper"; then
        echo "INSTALL_INCOMPLETE: staged Pro helper does not match the built artifact." >&2
        exit 1
      fi
      mv -f "$staged_helper" "$destination"
    )
    checked_codex plugin marketplace add "$script_dir"
    checked_codex plugin add codex-discord-remote@codex-discord-remote
    inventory=$(mktemp -d "${TMPDIR:-/tmp}/cdr-inventory.XXXXXX")
    trap 'rm -f -- "$inventory/marketplaces.json" "$inventory/plugins.json"; rmdir -- "$inventory"' 0
    checked_inventory "$inventory/marketplaces.json" plugin marketplace list --json
    checked_inventory "$inventory/plugins.json" plugin list --json
    "$binary_path" --admin verify-plugin-inventory --repo-root "$script_dir" \
      --marketplace-inventory "$inventory/marketplaces.json" --plugin-inventory "$inventory/plugins.json" \
      --plugin-manifest "$plugin/.codex-plugin/plugin.json"
  fi
fi
if [ "$dry_run" -eq 1 ]; then
  echo "Dry run complete. Plugin inventory was not verified."
else
  printf 'rust\n' > "$script_dir/.codex_discord_runtime"
  echo "Install complete. Run ./setup-discord-bot.sh; restart Codex to load installed skills."
fi
