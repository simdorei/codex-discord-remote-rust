from __future__ import annotations

import os
import re
import time
from collections.abc import Iterable
from pathlib import Path
from typing import Final, Protocol

from codex_discord_text import env_flag, parse_bounded_int_arg

DISCORD_ATTACHMENT_MAX_BYTES_DEFAULT: Final = 25 * 1024 * 1024
DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES_DEFAULT: Final = 32 * 1024
DISCORD_ATTACHMENT_TEXT_PREVIEW_CHARS: Final = 12000
TEXT_ATTACHMENT_EXTENSIONS: Final = frozenset(
    {
        ".bat",
        ".cmd",
        ".css",
        ".csv",
        ".html",
        ".ini",
        ".js",
        ".json",
        ".log",
        ".md",
        ".ps1",
        ".py",
        ".rs",
        ".sh",
        ".toml",
        ".ts",
        ".tsx",
        ".txt",
        ".xml",
        ".yaml",
        ".yml",
    }
)


type AttachmentTextValue = str | bytes | bytearray | int | None
type AttachmentSizeValue = int | str | bytes | bytearray | None


class AttachmentPayloadLike(Protocol):
    pass


class AttachmentMetadataLike(Protocol):
    @property
    def filename(self) -> AttachmentTextValue: ...

    @property
    def size(self) -> AttachmentSizeValue: ...

    @property
    def content_type(self) -> AttachmentTextValue: ...


class AttachmentChannelLike(Protocol):
    @property
    def id(self) -> AttachmentTextValue: ...


class AttachmentMessageLike(Protocol):
    @property
    def attachments(self) -> Iterable[AttachmentMetadataLike] | None: ...

    @property
    def embeds(self) -> Iterable[AttachmentPayloadLike] | None: ...

    @property
    def stickers(self) -> Iterable[AttachmentPayloadLike] | None: ...

    @property
    def channel(self) -> AttachmentChannelLike | None: ...


def iter_objects[T](value: Iterable[T] | str | bytes | bytearray | None) -> Iterable[T]:
    if value is None or isinstance(value, str | bytes | bytearray):
        return ()
    return value


def _coerce_nonnegative_int(value: AttachmentSizeValue) -> int | None:
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, int):
        return max(0, value)
    if isinstance(value, str):
        raw = value
    else:
        raw = _decode_ascii_bytes(value)
        if raw is None:
            return None
    try:
        return max(0, int(raw))
    except ValueError:
        return None


def _decode_ascii_bytes(value: bytes | bytearray) -> str | None:
    try:
        return bytes(value).decode("ascii")
    except UnicodeDecodeError:
        return None


def discord_attachments_enabled() -> bool:
    return env_flag("DISCORD_ENABLE_ATTACHMENTS", default=True)


def get_discord_attachment_max_bytes() -> int:
    return parse_bounded_int_arg(
        os.environ.get("DISCORD_ATTACHMENT_MAX_BYTES", ""),
        default=DISCORD_ATTACHMENT_MAX_BYTES_DEFAULT,
        minimum=1,
        maximum=100 * 1024 * 1024,
    )


def get_discord_attachment_text_inline_max_bytes() -> int:
    return parse_bounded_int_arg(
        os.environ.get("DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES", ""),
        default=DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES_DEFAULT,
        minimum=0,
        maximum=1024 * 1024,
    )


def message_has_non_text_payload(message: AttachmentMessageLike) -> bool:
    return bool(
        getattr(message, "attachments", None)
        or getattr(message, "embeds", None)
        or getattr(message, "stickers", None)
    )


def sanitize_attachment_filename(filename: AttachmentTextValue, index: int) -> str:
    raw_name = Path(str(filename or f"attachment-{index}")).name
    safe_name = re.sub(r"[^A-Za-z0-9._ -]+", "_", raw_name).strip(" .")
    if not safe_name:
        safe_name = f"attachment-{index}"
    return safe_name[:120]


def get_attachment_size(attachment: AttachmentMetadataLike) -> int | None:
    return _coerce_nonnegative_int(attachment.size)


def is_text_attachment(filename: str, content_type: AttachmentTextValue) -> bool:
    lowered_type = str(content_type or "").lower()
    if lowered_type.startswith("text/"):
        return True
    suffix = Path(filename).suffix.lower()
    return suffix in TEXT_ATTACHMENT_EXTENSIONS


def get_message_attachment_dir(
    message: AttachmentMessageLike,
    *,
    attachment_download_dir: Path,
    message_id: int | None,
) -> Path:
    channel = message.channel
    channel_id = "unknown" if channel is None else channel.id
    resolved_message_id = message_id or int(time.time() * 1000)
    return attachment_download_dir / str(channel_id or "unknown") / str(resolved_message_id)
