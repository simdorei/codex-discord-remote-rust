#!/usr/bin/env sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -f "$script_dir/.codex_discord_bot.disabled" ]; then
  echo "Codex Discord bot launch is disabled by its marker."
  exit 0
fi
runtime_mode=${CODEX_DISCORD_RUNTIME:-}
if [ -z "$runtime_mode" ] && [ -f "$script_dir/.codex_discord_runtime" ]; then
  runtime_mode=$(tr -d '\r\n' < "$script_dir/.codex_discord_runtime")
fi
[ -n "$runtime_mode" ] || runtime_mode=rust
if [ "$runtime_mode" != rust ]; then
  echo "This installation only supports the Rust runtime; unsupported selection: $runtime_mode" >&2
  exit 1
fi
binary="$script_dir/target/release/cdr-runtime"
[ -x "$binary" ] || { echo "Rust runtime not found: $binary; run ./install.sh first." >&2; exit 1; }
cd "$script_dir"
exec "$binary" --env "$script_dir/.env" "$@"
