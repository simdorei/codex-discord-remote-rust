from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import codex_discord_stale_busy_components as discord_stale_busy_components
import codex_discord_store as discord_store
from codex_discord_components import ComponentMessageLike

GetDbPathFunc = Callable[[], Path]
BusyChoiceRecordGetter = Callable[[str], discord_stale_busy_components.BusyChoiceRecord | None]
LogFunc = Callable[[str], None]


class PersistentComponentMessageLike(Protocol):
    @property
    def id(self) -> int | str | bytes | bytearray: ...


class PersistentComponentInteractionLike(Protocol):
    @property
    def message(self) -> PersistentComponentMessageLike | None: ...


@dataclass(frozen=True, slots=True)
class BusyComponentRuntime:
    get_db_path: GetDbPathFunc
    get_busy_choice_record_func: BusyChoiceRecordGetter
    cleanup_history_limit: int
    log: LogFunc

    def cleanup_expired_busy_choices(self, now: float | None = None) -> int:
        return discord_store.cleanup_expired_busy_choices(self.get_db_path(), now=now)

    def cleanup_expired_persistent_component_claims(self, now: float | None = None) -> int:
        return discord_store.cleanup_expired_persistent_component_claims(self.get_db_path(), now=now)

    def get_busy_choice_counts(self, now: float | None = None) -> tuple[int, int]:
        return discord_store.get_busy_choice_counts(self.get_db_path(), now=now)

    def get_persistent_component_claim_counts(self, now: float | None = None) -> tuple[int, int]:
        return discord_store.get_persistent_component_claim_counts(self.get_db_path(), now=now)

    def get_busy_choice_record(self, choice_id: str) -> discord_stale_busy_components.BusyChoiceRecord | None:
        return discord_store.get_busy_choice_record(self.get_db_path(), choice_id)

    def claim_busy_choice_record(self, choice_id: str) -> bool:
        return discord_store.claim_busy_choice_record(self.get_db_path(), choice_id)

    def claim_persistent_component_interaction(
        self,
        interaction: PersistentComponentInteractionLike,
        custom_id: str,
        *,
        ttl_seconds: float = 86400.0,
    ) -> bool:
        return discord_store.claim_persistent_component_interaction(
            self.get_db_path(),
            interaction,
            custom_id,
            ttl_seconds=ttl_seconds,
        )

    async def clear_stale_busy_choice_message_components(self, message: ComponentMessageLike) -> bool:
        return await discord_stale_busy_components.clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=self.get_busy_choice_record_func,
            log_func=self.log,
        )

    async def cleanup_stale_busy_choice_components_in_channel(
        self,
        channel: discord_stale_busy_components.MessageHistoryChannel | None,
        *,
        limit: int | None = None,
    ) -> int:
        return await discord_stale_busy_components.cleanup_stale_busy_choice_components_in_channel(
            channel,
            get_busy_choice_record=self.get_busy_choice_record_func,
            log_func=self.log,
            limit=self.cleanup_history_limit if limit is None else limit,
        )
