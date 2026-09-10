from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SessionMirrorItem = Mapping[str, str]
InteractiveOptions = list[tuple[str, str]]
ParseInteractiveNotice = Callable[[str], tuple[str, str, InteractiveOptions]]
FormatSessionMirrorText = Callable[[SessionMirrorItem], str]


class SessionMirrorInteractiveSender(Protocol[ChannelT_contra]):
    def __call__(
        self,
        channel: ChannelT_contra,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: InteractiveOptions,
    ) -> Awaitable[None]: ...


class SessionMirrorChunkSender(Protocol[ChannelT_contra]):
    def __call__(self, channel: ChannelT_contra, content: str, *, context: str) -> Awaitable[None]: ...


class SessionMirrorAttachmentSender(Protocol[ChannelT_contra]):
    def __call__(
        self,
        channel: ChannelT_contra,
        content: str,
        attachment_url: str,
        filename: str,
        *,
        context: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class SessionMirrorItemDeliveryDeps(Generic[ChannelT]):
    parse_interactive_notice: ParseInteractiveNotice
    send_interactive_prompt: SessionMirrorInteractiveSender[ChannelT]
    send_chunks: SessionMirrorChunkSender[ChannelT]
    send_attachment: SessionMirrorAttachmentSender[ChannelT]
    format_session_mirror_text: FormatSessionMirrorText


async def send_session_mirror_item(
    channel: ChannelT,
    item: SessionMirrorItem,
    *,
    target_thread_id: str,
    target_ref: str,
    deps: SessionMirrorItemDeliveryDeps[ChannelT],
) -> None:
    kind = item.get("kind") or ""
    text = item.get("text") or ""
    attachment_url = item.get("attachment_url") or ""
    if attachment_url:
        await deps.send_attachment(
            channel,
            deps.format_session_mirror_text(item),
            attachment_url,
            item.get("attachment_filename") or "codex-image-output.png",
            context=f"session_mirror:{kind or 'unknown'}:{target_thread_id}",
        )
        return
    if kind == "interactive":
        state, prompt, options = deps.parse_interactive_notice(text)
        if state:
            await deps.send_interactive_prompt(
                channel,
                target_thread_id,
                target_ref,
                state,
                prompt,
                options,
            )
            return
    await deps.send_chunks(
        channel,
        deps.format_session_mirror_text(item),
        context=f"session_mirror:{kind or 'unknown'}:{target_thread_id}",
    )
