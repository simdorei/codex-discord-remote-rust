from __future__ import annotations

import base64
import binascii
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import TypeVar
from urllib.parse import unquote, urlparse

import codex_discord_delivery as discord_delivery
import codex_discord_delivery_state as discord_delivery_state

SentMessageT = TypeVar("SentMessageT")
GetRetryDelaysFunc = Callable[[], tuple[float, ...]]
GetBoolFunc = Callable[[], bool]
SetBoolFunc = Callable[[bool], None]


class AttachmentDataUrlError(ValueError):
    pass


CODEX_SESSION_MIRROR_ATTACHMENT_DIR = Path(__file__).resolve().parent / ".codex-remote-attachments"


def decode_data_url_attachment(data_url: str) -> bytes:
    header, separator, payload = data_url.partition(",")
    if separator != "," or ";base64" not in header:
        raise AttachmentDataUrlError("attachment data URL must be base64 encoded")
    try:
        return base64.b64decode(payload, validate=True)
    except binascii.Error as exc:
        raise AttachmentDataUrlError("attachment data URL has invalid base64 payload") from exc


def _attachment_source_path(attachment_source: str) -> Path:
    source = attachment_source.strip()
    if source.startswith("file://"):
        parsed = urlparse(source)
        if parsed.netloc:
            raise AttachmentDataUrlError("attachment file URL must be local")
        raw_path = unquote(parsed.path)
        if len(raw_path) >= 3 and raw_path[0] == "/" and raw_path[2] == ":":
            raw_path = raw_path[1:]
        return Path(raw_path)
    if "://" in source:
        raise AttachmentDataUrlError("attachment source must be a data URL or local output file")
    return Path(source)


def _resolve_attachment_output_path(attachment_source: str) -> Path:
    path = _attachment_source_path(attachment_source)
    raw_path = str(path)
    if raw_path.startswith(("\\\\", "//")):
        raise AttachmentDataUrlError("attachment source must not be a network path")
    try:
        resolved = path.resolve(strict=True)
    except OSError as exc:
        raise AttachmentDataUrlError(f"attachment source is not a file: {attachment_source}") from exc
    output_root = CODEX_SESSION_MIRROR_ATTACHMENT_DIR.resolve()
    try:
        _ = resolved.relative_to(output_root)
    except ValueError as exc:
        raise AttachmentDataUrlError(
            f"attachment source must be under {CODEX_SESSION_MIRROR_ATTACHMENT_DIR}"
        ) from exc
    if not resolved.is_file():
        raise AttachmentDataUrlError(f"attachment source is not a file: {attachment_source}")
    return resolved


def is_attachment_source_allowed(attachment_source: str) -> bool:
    source = attachment_source.strip()
    if source.startswith("data:"):
        return True
    try:
        _ = _resolve_attachment_output_path(source)
    except AttachmentDataUrlError:
        return False
    return True


def read_attachment_source_bytes(attachment_source: str) -> bytes:
    source = attachment_source.strip()
    if source.startswith("data:"):
        return decode_data_url_attachment(source)
    path = _resolve_attachment_output_path(source)
    try:
        return path.read_bytes()
    except OSError as exc:
        raise AttachmentDataUrlError(f"attachment source could not be read: {source}") from exc


@dataclass(frozen=True, slots=True)
class DiscordDeliveryRuntime:
    state: discord_delivery.DiscordDeliveryState
    get_retry_delays_seconds: GetRetryDelaysFunc
    get_chunk_markers_enabled: GetBoolFunc
    get_legacy_stopping: GetBoolFunc
    set_legacy_stopping: SetBoolFunc
    log: discord_delivery_state.LogFunc

    def sync_legacy_config(self) -> None:
        self.state.retry_delays_seconds = tuple(self.get_retry_delays_seconds())
        self.state.chunk_markers_enabled = bool(self.get_chunk_markers_enabled())
        self.state.stopping = bool(self.get_legacy_stopping())

    def set_stopping(self, reason: str) -> None:
        self.sync_legacy_config()
        discord_delivery.set_discord_delivery_stopping(
            self.state,
            reason,
            log_func=self.log,
        )
        self.set_legacy_stopping(self.state.stopping)

    def clear_stopping(self) -> None:
        discord_delivery.clear_discord_delivery_stopping(self.state)
        self.set_legacy_stopping(False)

    def is_stopping(self) -> bool:
        self.sync_legacy_config()
        return self.state.stopping

    def begin(self, label: str, *, allow_during_stop: bool = False) -> str:
        self.sync_legacy_config()
        return discord_delivery.begin_discord_delivery(
            self.state,
            label,
            log_func=self.log,
            allow_during_stop=allow_during_stop,
        )

    def end(self, token: str) -> None:
        discord_delivery.end_discord_delivery(self.state, token)

    async def wait_for_drain(self, *, timeout_seconds: float, reason: str) -> bool:
        return await discord_delivery.wait_for_discord_delivery_drain(
            self.state,
            timeout_seconds=timeout_seconds,
            reason=reason,
            log_func=self.log,
        )

    def split_chunks(self, text: str) -> list[str]:
        self.sync_legacy_config()
        return discord_delivery.split_delivery_chunks(text, state=self.state)

    async def send_chunks(
        self,
        target: discord_delivery_state.Messageable,
        text: str,
        *,
        context: str = "send_chunks",
        allow_during_stop: bool = False,
    ) -> int:
        self.sync_legacy_config()
        return await discord_delivery.send_chunks(
            self.state,
            target,
            text,
            log_func=self.log,
            context=context,
            allow_during_stop=allow_during_stop,
        )

    async def send_restarting_notice(
        self,
        target: discord_delivery_state.Messageable,
    ) -> None:
        self.sync_legacy_config()
        await discord_delivery.send_discord_restarting_notice(
            self.state,
            target,
            log_func=self.log,
        )

    async def send_attachment(
        self,
        target: discord_delivery.AttachmentTarget[SentMessageT],
        content: str,
        attachment_url: str,
        filename: str,
        *,
        context: str = "send_attachment",
        allow_during_stop: bool = False,
    ) -> SentMessageT:
        self.sync_legacy_config()
        return await discord_delivery.send_attachment_bytes(
            self.state,
            target,
            content,
            filename,
            read_attachment_source_bytes(attachment_url),
            log_func=self.log,
            context=context,
            allow_during_stop=allow_during_stop,
        )

    async def send_message_tracked(
        self,
        target: discord_delivery.TrackedMessageTarget[SentMessageT],
        content: str,
        *,
        view: discord_delivery.DiscordMessageView | None = None,
        context: str = "send_message",
        allow_during_stop: bool = False,
    ) -> SentMessageT:
        self.sync_legacy_config()
        return await discord_delivery.send_message_tracked(
            self.state,
            target,
            content,
            log_func=self.log,
            view=view,
            context=context,
            allow_during_stop=allow_during_stop,
        )

    async def send_interaction_response_tracked(
        self,
        interaction: discord_delivery_state.InteractionLike,
        content: str,
        *,
        ephemeral: bool = False,
        context: str = "interaction_response",
        allow_during_stop: bool = False,
    ) -> None:
        self.sync_legacy_config()
        await discord_delivery.send_interaction_response_tracked(
            self.state,
            interaction,
            content,
            log_func=self.log,
            ephemeral=ephemeral,
            context=context,
            allow_during_stop=allow_during_stop,
        )

    async def send_interaction_not_allowed(
        self,
        interaction: discord_delivery_state.InteractionLike,
    ) -> None:
        self.sync_legacy_config()
        await discord_delivery.send_interaction_not_allowed(
            self.state,
            interaction,
            log_func=self.log,
        )
