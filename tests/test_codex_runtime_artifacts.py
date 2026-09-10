from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_runtime_control_artifacts_are_ignored_by_git() -> None:
    ignored_paths = {
        line.strip()
        for line in (ROOT / ".gitignore").read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }

    assert {
        ".codex_discord_bot.disabled",
        ".codex_discord_bot.heartbeat",
        ".codex_discord_bot.restart.claimed.*",
        ".codex_discord_bot.stop",
        ".codex_discord_bot.stop.claimed.*",
    } <= ignored_paths
