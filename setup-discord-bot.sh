#!/usr/bin/env sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
binary_path=""
dry_run=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary-path) [ "$#" -ge 2 ] || exit 2; binary_path=$2; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    *) echo "Usage: ./setup-discord-bot.sh [--binary-path PATH] [--dry-run]" >&2; exit 2 ;;
  esac
done
target=${CARGO_TARGET_DIR:-target}
case "$target" in /*|[A-Za-z]:[\\/]*) ;; *) target="$script_dir/$target" ;; esac
[ -n "$binary_path" ] || binary_path="$target/release/cdr-runtime"
[ -x "$binary_path" ] || { echo "Rust runtime not found: $binary_path; run ./install.sh first." >&2; exit 1; }
if [ "$dry_run" -eq 1 ]; then
  exec "$binary_path" --admin setup-discord --repo-root "$script_dir" --dry-run
fi
terminal_state=$(stty -g </dev/tty) || { echo "Interactive terminal required for hidden token entry." >&2; exit 1; }
trap 'stty "$terminal_state" </dev/tty' 0
trap 'exit 130' HUP INT TERM
printf 'Discord bot token (hidden): ' >/dev/tty
stty -echo </dev/tty
IFS= read -r bot_token </dev/tty
stty "$terminal_state" </dev/tty
printf '\nDiscord channel ID (optional): ' >/dev/tty
IFS= read -r channel_id </dev/tty
printf '%s\n%s\n' "$bot_token" "$channel_id" | "$binary_path" --admin setup-discord --repo-root "$script_dir" --input-lines
unset bot_token
