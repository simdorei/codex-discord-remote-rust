from __future__ import annotations

import os
from pathlib import Path

from codex_discord_text import env_flag, parse_bounded_float_env, parse_int_set


class MissingRequiredEnvError(RuntimeError):
    def __init__(self, name: str) -> None:
        self.name: str = name
        super().__init__(f"Missing required environment variable: {name}")


def discord_qa_commands_enabled() -> bool:
    return env_flag("DISCORD_ENABLE_QA_COMMANDS", default=False)


def discord_host_commands_enabled() -> bool:
    return env_flag("DISCORD_ENABLE_HOST_COMMANDS", default=False)


def discord_stream_commentary_enabled() -> bool:
    return env_flag("DISCORD_STREAM_COMMENTARY", default=True)


def discord_startup_notify_enabled() -> bool:
    return env_flag("DISCORD_STARTUP_NOTIFY", default=False)


def discord_allow_all_channels_enabled() -> bool:
    return env_flag("DISCORD_ALLOW_ALL_CHANNELS", default=False)


def discord_message_content_enabled() -> bool:
    return env_flag("DISCORD_ENABLE_MESSAGE_CONTENT", default=True)


def get_discord_allowed_channel_ids() -> set[int]:
    return parse_int_set(os.environ.get("DISCORD_ALLOWED_CHANNEL_IDS", ""))


def get_discord_allowed_user_ids() -> set[int]:
    return parse_int_set(os.environ.get("DISCORD_ALLOWED_USER_IDS", ""))


def get_plain_ask_mention_user_ids() -> set[int]:
    return parse_int_set(os.environ.get("DISCORD_PLAIN_ASK_MENTION_USER_IDS", ""))


def get_discord_guild_id() -> int | None:
    raw = os.environ.get("DISCORD_GUILD_ID", "").strip()
    return int(raw) if raw else None


def get_startup_channel_id(channel_ids: set[int]) -> int | None:
    raw = os.environ.get("DISCORD_STARTUP_CHANNEL_ID", "").strip()
    if raw:
        return int(raw)
    if len(channel_ids) == 1:
        return next(iter(channel_ids))
    return None


def get_required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise MissingRequiredEnvError(name)
    return value


def load_local_env(path: Path) -> None:
    if not path.exists():
        return
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key and key not in os.environ:
            os.environ[key] = value


def get_ask_busy_retry_attempts(*, default: float) -> int:
    return int(
        parse_bounded_float_env(
            "DISCORD_ASK_BUSY_RETRY_ATTEMPTS",
            default=default,
            minimum=0.0,
            maximum=10.0,
        )
    )


def get_ask_busy_retry_delay_seconds(*, default: float) -> float:
    return parse_bounded_float_env(
        "DISCORD_ASK_BUSY_RETRY_DELAY_SECONDS",
        default=default,
        minimum=1.0,
        maximum=60.0,
    )


def get_steering_delivery_confirm_timeout(*, default: float) -> float:
    return parse_bounded_float_env(
        "DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS",
        default=default,
        minimum=3.0,
        maximum=120.0,
    )


def get_steering_pending_watch_timeout(*, default: float) -> float:
    return parse_bounded_float_env(
        "DISCORD_STEERING_PENDING_WATCH_TIMEOUT_SECONDS",
        default=default,
        minimum=10.0,
        maximum=600.0,
    )


def get_stale_busy_steer_block_seconds(*, default: float) -> float:
    return parse_bounded_float_env(
        "DISCORD_STALE_BUSY_STEER_BLOCK_SECONDS",
        default=default,
        minimum=60.0,
        maximum=3600.0,
    )


def get_history_poll_seconds(*, default: float) -> float:
    return parse_bounded_float_env(
        "DISCORD_HISTORY_POLL_SECONDS",
        default=default,
        minimum=0.0,
        maximum=300.0,
    )
