from __future__ import annotations

from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Protocol, TypeGuard

from codex_discord_attachment_metadata import (
    AttachmentMessageLike,
    AttachmentMetadataLike,
    DISCORD_ATTACHMENT_MAX_BYTES_DEFAULT as DISCORD_ATTACHMENT_MAX_BYTES_DEFAULT,
    DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES_DEFAULT as DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES_DEFAULT,
    DISCORD_ATTACHMENT_TEXT_PREVIEW_CHARS as DISCORD_ATTACHMENT_TEXT_PREVIEW_CHARS,
    TEXT_ATTACHMENT_EXTENSIONS as TEXT_ATTACHMENT_EXTENSIONS,
    discord_attachments_enabled as discord_attachments_enabled,
    get_attachment_size as get_attachment_size,
    get_discord_attachment_max_bytes as get_discord_attachment_max_bytes,
    get_discord_attachment_text_inline_max_bytes as get_discord_attachment_text_inline_max_bytes,
    get_message_attachment_dir as get_message_attachment_dir,
    iter_objects as _iter_objects,
    is_text_attachment as is_text_attachment,
    message_has_non_text_payload as message_has_non_text_payload,
    sanitize_attachment_filename as sanitize_attachment_filename,
)
from codex_discord_attachment_prompt import (
    AttachmentTextPreview,
    render_attachment_prompt,
    render_saved_attachment_detail,
)

type AttachmentBytes = bytes | bytearray | memoryview | None
type LogFunc = Callable[[str], None]
type MessageIdFunc = Callable[[AttachmentMessageLike], int | None]


class SaveAttachment(Protocol):
    def save(self, destination: Path) -> Awaitable[None]: ...


class ReadAttachment(Protocol):
    def read(self) -> Awaitable[AttachmentBytes]: ...


class UnsupportedDiscordAttachmentError(RuntimeError):
    pass


type PromptAttachment = AttachmentMetadataLike


def _has_save_method(value: PromptAttachment) -> TypeGuard[SaveAttachment]:
    return callable(getattr(value, "save", None))


def _has_read_method(value: PromptAttachment) -> TypeGuard[ReadAttachment]:
    return callable(getattr(value, "read", None))


async def save_discord_attachment(attachment: PromptAttachment, destination: Path) -> None:
    if _has_save_method(attachment):
        await attachment.save(destination)
        return
    if _has_read_method(attachment):
        data = await attachment.read()
        _ = destination.write_bytes(bytes(data or b""))
        return
    raise UnsupportedDiscordAttachmentError("Discord attachment object does not support save/read")


def read_attachment_text_preview(
    path: Path,
    *,
    limit_chars: int = DISCORD_ATTACHMENT_TEXT_PREVIEW_CHARS,
) -> str:
    text = path.read_bytes().decode("utf-8", errors="replace")
    if len(text) <= limit_chars:
        return text
    return text[:limit_chars].rstrip() + "\n\n[truncated]"


async def _process_prompt_attachment(
    *,
    index: int,
    attachment: PromptAttachment,
    attachment_dir: Path,
    message_id: int | None,
    max_bytes: int,
    text_inline_max_bytes: int,
    log_func: LogFunc,
) -> tuple[str, AttachmentTextPreview | None]:
    filename = sanitize_attachment_filename(attachment.filename, index)
    size = get_attachment_size(attachment)
    content_type = str(attachment.content_type or "").strip()
    if size is not None and size > max_bytes:
        detail = f"{index}. {filename} skipped: file is {size} bytes; limit is {max_bytes} bytes."
        log_func(
            f"attachment_skipped reason=size message={message_id or '-'} "
            + f"filename={filename[:80]} size={size} limit={max_bytes}"
        )
        return detail, None

    destination = attachment_dir / f"{index:02d}-{filename}"
    try:
        await save_discord_attachment(attachment, destination)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - external Discord attachment boundary
        log_func(
            f"attachment_save_failed message={message_id or '-'} "
            + f"filename={filename[:80]} error_type={type(exc).__name__}"
        )
        return f"{index}. {filename} failed to save: {type(exc).__name__}.", None

    saved_size = destination.stat().st_size
    detail = render_saved_attachment_detail(
        index=index,
        filename=filename,
        destination=destination,
        content_type=content_type,
        size_bytes=saved_size,
    )
    log_func(
        f"attachment_saved message={message_id or '-'} "
        + f"filename={filename[:80]} size={saved_size} path={destination}"
    )
    if (
        text_inline_max_bytes > 0
        and saved_size <= text_inline_max_bytes
        and is_text_attachment(filename, content_type)
    ):
        try:
            return detail, AttachmentTextPreview(
                filename=filename,
                preview=read_attachment_text_preview(destination),
            )
        except OSError as exc:
            log_func(
                f"attachment_preview_failed message={message_id or '-'} "
                + f"filename={filename[:80]} error_type={type(exc).__name__}"
            )
    return detail, None


async def build_prompt_with_discord_attachments(
    message: AttachmentMessageLike,
    prompt: str,
    *,
    attachments_enabled: bool,
    max_bytes: int,
    text_inline_max_bytes: int,
    attachment_download_dir: Path,
    log_func: LogFunc,
    get_message_id: MessageIdFunc,
) -> str:
    raw_attachments = message.attachments
    attachments = list(_iter_objects(raw_attachments))
    if not attachments or not attachments_enabled:
        return prompt

    base_prompt = (prompt or "").strip() or "Please inspect the attached Discord file(s)."
    message_id = get_message_id(message)
    attachment_dir = get_message_attachment_dir(
        message,
        attachment_download_dir=attachment_download_dir,
        message_id=message_id,
    )
    attachment_dir.mkdir(parents=True, exist_ok=True)

    details: list[str] = []
    previews: list[AttachmentTextPreview] = []
    for index, attachment in enumerate(attachments, start=1):
        detail, preview = await _process_prompt_attachment(
            index=index,
            attachment=attachment,
            attachment_dir=attachment_dir,
            message_id=message_id,
            max_bytes=max_bytes,
            text_inline_max_bytes=text_inline_max_bytes,
            log_func=log_func,
        )
        details.append(detail)
        if preview is not None:
            previews.append(preview)

    return render_attachment_prompt(base_prompt, details, previews)
