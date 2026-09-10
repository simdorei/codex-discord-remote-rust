from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_attachment_metadata as discord_attachment_metadata
import codex_discord_attachments as discord_attachments
import codex_discord_empty_content_notice as discord_empty_content_notice

GetIntFunc = Callable[[], int]
GetBoolFunc = Callable[[], bool]
MessageIdFunc = Callable[[discord_attachment_metadata.AttachmentMessageLike], int | None]


@dataclass(frozen=True, slots=True)
class MessagePayloadRuntime:
    attachment_download_dir: Path
    get_message_id: MessageIdFunc
    message_has_non_text_payload: discord_empty_content_notice.MessageHasNonTextPayloadFunc
    send_chunks: discord_empty_content_notice.SendChunksFunc
    log: discord_empty_content_notice.LogFunc
    attachments_enabled: GetBoolFunc = discord_attachments.discord_attachments_enabled
    get_attachment_max_bytes: GetIntFunc = discord_attachments.get_discord_attachment_max_bytes
    get_attachment_text_inline_max_bytes: GetIntFunc = (
        discord_attachments.get_discord_attachment_text_inline_max_bytes
    )

    async def build_prompt_with_discord_attachments(
        self,
        message: discord_attachment_metadata.AttachmentMessageLike,
        prompt: str,
    ) -> str:
        return await discord_attachments.build_prompt_with_discord_attachments(
            message,
            prompt,
            attachments_enabled=self.attachments_enabled(),
            max_bytes=self.get_attachment_max_bytes(),
            text_inline_max_bytes=self.get_attachment_text_inline_max_bytes(),
            attachment_download_dir=self.attachment_download_dir,
            log_func=self.log,
            get_message_id=self.get_message_id,
        )

    async def maybe_send_empty_content_notice(
        self,
        message: discord_empty_content_notice.EmptyContentMessage,
    ) -> None:
        await discord_empty_content_notice.maybe_send_empty_content_notice(
            message,
            deps=discord_empty_content_notice.make_empty_content_notice_deps(
                message_has_non_text_payload=self.message_has_non_text_payload,
                send_chunks=self.send_chunks,
                log_line=self.log,
            ),
        )
