from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_delivery as discord_delivery
import codex_discord_delivery_runtime as discord_delivery_runtime
from codex_discord_bot_module_context import update_runtime_state


@dataclass(frozen=True, slots=True)
class BotDeliveryAdapterRuntime:
    module: ModuleType

    def make_delivery_runtime(self) -> discord_delivery_runtime.DiscordDeliveryRuntime:
        return discord_delivery_runtime.DiscordDeliveryRuntime(
            state=cast(discord_delivery.DiscordDeliveryState, getattr(self.module, "DISCORD_DELIVERY_STATE")),
            get_retry_delays_seconds=self.get_retry_delays_seconds,
            get_chunk_markers_enabled=self.get_chunk_markers_enabled,
            get_legacy_stopping=self.get_legacy_stopping,
            set_legacy_stopping=self.set_legacy_stopping,
            log=self.log_line,
        )

    def get_retry_delays_seconds(self) -> tuple[float, ...]:
        return tuple(cast(tuple[float, ...], getattr(self.module, "DISCORD_SEND_RETRY_DELAYS_SECONDS")))

    def get_chunk_markers_enabled(self) -> bool:
        return cast(bool, getattr(self.module, "DISCORD_CHUNK_MARKERS_ENABLED"))

    def get_legacy_stopping(self) -> bool:
        return cast(bool, getattr(self.module, "discord_delivery_stopping"))

    def set_legacy_stopping(self, stopping: bool) -> None:
        update_runtime_state(
            self.module,
            {
                "discord_delivery_stopping": stopping,
                "DISCORD_DELIVERY_STOPPING": stopping,
            },
        )

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], getattr(self.module, "log_line"))(message)
