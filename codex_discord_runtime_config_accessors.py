from __future__ import annotations

from dataclasses import dataclass

import codex_discord_runtime_config as discord_runtime_config
from codex_discord_text import env_flag, parse_bounded_float_env


@dataclass(frozen=True, slots=True)
class RuntimeConfigAccessors:
    steering_delivery_confirm_timeout_default: float
    steering_pending_watch_timeout_default: float
    ask_busy_retry_delay_seconds_default: float
    startup_channel_probe_timeout_default: float

    def discord_session_mirror_enabled(self) -> bool:
        return env_flag("DISCORD_SESSION_MIRROR", default=True)

    def get_steering_delivery_confirm_timeout(self) -> float:
        return discord_runtime_config.get_steering_delivery_confirm_timeout(
            default=self.steering_delivery_confirm_timeout_default,
        )

    def get_steering_pending_watch_timeout(self) -> float:
        return discord_runtime_config.get_steering_pending_watch_timeout(
            default=self.steering_pending_watch_timeout_default,
        )

    def get_ask_busy_retry_delay_seconds(self) -> float:
        return discord_runtime_config.get_ask_busy_retry_delay_seconds(
            default=self.ask_busy_retry_delay_seconds_default,
        )

    def get_startup_channel_probe_timeout(self) -> float:
        return parse_bounded_float_env(
            "DISCORD_STARTUP_CHANNEL_PROBE_TIMEOUT_SECONDS",
            default=self.startup_channel_probe_timeout_default,
            minimum=0.25,
            maximum=60.0,
        )
