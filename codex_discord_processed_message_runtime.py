from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias, cast

import codex_discord_attachment_metadata as discord_attachment_metadata
import codex_discord_diagnostics_history as discord_diagnostics_history
import codex_discord_message_identity as discord_message_identity
import codex_discord_seen_cache as discord_seen_cache
import codex_discord_store as discord_store

GetDbPathFunc = Callable[[], Path]
LogFunc = Callable[[str], None]


class DiscordMessageIdCarrier(Protocol):
    @property
    def id(self) -> discord_message_identity.RawDiscordMessageId: ...


DiscordMessageIdInput: TypeAlias = (
    DiscordMessageIdCarrier
    | discord_attachment_metadata.AttachmentMessageLike
    | discord_diagnostics_history.DiscordHistoryMessage
)
SeenCacheOwner: TypeAlias = discord_seen_cache.SeenCacheOwner
GetMessageIdFunc = Callable[[DiscordMessageIdInput], int | None]
ClaimPersistentMessageIdFunc = Callable[[int], bool]


@dataclass(frozen=True, slots=True)
class ProcessedMessageRuntime:
    get_db_path: GetDbPathFunc
    processed_message_id_limit: int
    get_message_id_func: GetMessageIdFunc
    claim_persistent_message_id_func: ClaimPersistentMessageIdFunc
    log: LogFunc

    def get_discord_message_id(self, message: DiscordMessageIdInput) -> int | None:
        return discord_message_identity.coerce_discord_message_id(
            getattr(message, "id", None)
        )

    def claim_persistent_discord_message_id(
        self, message_id: int, now: float | None = None
    ) -> bool:
        try:
            return discord_store.claim_persistent_discord_message_id(
                self.get_db_path(),
                message_id,
                now=now,
            )
        except (OSError, sqlite3.Error) as exc:
            self.log(
                f"processed_message_persist_failed message={message_id} error_type={type(exc).__name__}"
            )
            return False

    def claim_discord_message(
        self, owner: SeenCacheOwner, message: DiscordMessageIdInput
    ) -> bool:
        message_id = self.get_message_id_func(message)
        if message_id is None:
            return True
        processed = discord_seen_cache.get_or_create_seen_map(
            owner, "_processed_message_ids"
        )
        if processed is not None and message_id in processed:
            return False
        if not self.claim_persistent_message_id_func(message_id):
            return False
        if processed is not None:
            discord_seen_cache.remember_limited_seen_key(
                processed,
                message_id,
                limit=self.processed_message_id_limit,
            )
        return True

    def mark_discord_message_processed(
        self, owner: SeenCacheOwner, message: DiscordMessageIdInput
    ) -> None:
        message_id = self.get_message_id_func(message)
        if message_id is None:
            return
        processed = getattr(owner, "_processed_message_ids", None)
        if isinstance(processed, dict):
            discord_seen_cache.remember_limited_seen_key(
                cast(discord_seen_cache.SeenCacheMap, processed),
                message_id,
                limit=self.processed_message_id_limit,
            )
        try:
            discord_store.mark_processed_discord_message_id(
                self.get_db_path(), message_id
            )
        except (OSError, sqlite3.Error) as exc:
            self.log(
                f"processed_message_mark_failed message={message_id} error_type={type(exc).__name__}"
            )
