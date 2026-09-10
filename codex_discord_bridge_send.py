from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Sequence
from typing import Protocol, TypeVar

ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SendResultT_co = TypeVar("SendResultT_co", covariant=True)
ArchiveOwnerT = TypeVar("ArchiveOwnerT")


class SendChunksFunc(Protocol[ChannelT_contra, SendResultT_co]):
    def __call__(
        self,
        target: ChannelT_contra,
        text: str,
        *,
        context: str,
    ) -> Awaitable[SendResultT_co]: ...


async def run_bridge_and_send(
    target: ChannelT,
    argv: list[str],
    title: str,
    failure_title: str | None = None,
    archive_cleanup_owner: ArchiveOwnerT | None = None,
    *,
    run_bridge_command_func: Callable[[list[str]], tuple[int, str]],
    cleanup_archive_mirror_after_bridge_command_func: Callable[
        [ArchiveOwnerT | None, list[str], int, str],
        Awaitable[str | None],
    ],
    split_delivery_chunks_func: Callable[[str], Sequence[str]],
    send_chunks_func: SendChunksFunc[ChannelT, int],
    format_log_argv_func: Callable[[list[str]], str],
    log_func: Callable[[str], None],
) -> tuple[int, str]:
    exit_code, output = await asyncio.to_thread(run_bridge_command_func, argv)
    cleanup_warning = await cleanup_archive_mirror_after_bridge_command_func(
        archive_cleanup_owner,
        argv,
        exit_code,
        output,
    )
    prefix = title if exit_code == 0 else f"{failure_title or title} failed (exit {exit_code})"
    display_output = output or "(no output)"
    if cleanup_warning:
        display_output = f"{display_output}\n\n{cleanup_warning}"
    text = f"{prefix}\n\n{display_output}"
    chunks = split_delivery_chunks_func(text)
    log_func(
        f"bridge_command_done title={title!r} exit={exit_code} "
        + f"chunks={len(chunks)} argv={format_log_argv_func(argv)}"
    )
    sent_chunks = await send_chunks_func(target, text, context=f"bridge_command:{title}")
    log_func(f"bridge_command_sent title={title!r} exit={exit_code} chunks={len(chunks)}")
    if sent_chunks != len(chunks):
        log_func(
            f"bridge_command_chunk_count_changed title={title!r} "
            + f"planned={len(chunks)} sent={sent_chunks}"
        )
    return exit_code, output


__all__ = [
    "SendChunksFunc",
    "run_bridge_and_send",
]
