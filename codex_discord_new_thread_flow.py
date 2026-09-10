from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import sqlite3
import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

BotT = TypeVar("BotT")
BotT_contra = TypeVar("BotT_contra", contravariant=True)
CodexThreadT = TypeVar("CodexThreadT")
DiscordThreadT = TypeVar("DiscordThreadT", bound="DiscordThreadLike")
DiscordThreadT_co = TypeVar("DiscordThreadT_co", bound="DiscordThreadLike", covariant=True)


class DiscordThreadLike(Protocol):
    @property
    def id(self) -> int: ...


class MirrorSingleCodexThreadFunc(Protocol[BotT_contra, DiscordThreadT_co]):
    def __call__(
        self,
        bot: BotT_contra,
        codex_thread_id: str,
        *,
        preferred_project_channel_id: int | None = None,
    ) -> Awaitable[DiscordThreadT_co]: ...


@dataclass(frozen=True, slots=True)
class NewThreadFlowDeps(Generic[BotT, CodexThreadT, DiscordThreadT]):
    resolve_new_thread_cwd: Callable[[int | None], str | None]
    run_bridge_command: Callable[[list[str]], tuple[int, str]]
    parse_bridge_output_value: Callable[[str, str], str | None]
    choose_thread: Callable[[str, str | None], CodexThreadT]
    get_project_key: Callable[[CodexThreadT], str]
    resolve_project_channel_id: Callable[[int | None, str], int | None]
    mirror_single_codex_thread: MirrorSingleCodexThreadFunc[BotT, DiscordThreadT]
    prepare_mapped_session_mirror_output: Callable[[DiscordThreadT, str], Awaitable[bool]]
    delivery_exceptions: tuple[type[BaseException], ...]
    log: Callable[[str], None]


def format_discord_new_thread_prefix(exit_code: int, new_thread_id: str | None) -> str:
    if exit_code == 0:
        return "New"
    if new_thread_id:
        return f"New created but verification failed (exit {exit_code})"
    return f"New failed (exit {exit_code})"


def _build_new_thread_argv(
    discord_channel_id: int | None,
    prompt: str,
    *,
    deps: NewThreadFlowDeps[BotT, CodexThreadT, DiscordThreadT],
) -> list[str]:
    argv = ["new"]
    target_cwd = deps.resolve_new_thread_cwd(discord_channel_id)
    if target_cwd:
        argv.extend(["--cwd", target_cwd])
        deps.log(f"new_thread_cwd channel={discord_channel_id} cwd={target_cwd}")
    else:
        deps.log(f"new_thread_cwd channel={discord_channel_id} cwd=default")
    argv.append(prompt)
    return argv


async def _mirror_new_thread(
    bot: BotT,
    discord_channel_id: int | None,
    new_thread_id: str,
    *,
    deps: NewThreadFlowDeps[BotT, CodexThreadT, DiscordThreadT],
) -> str:
    preferred_project_channel_id = None
    codex_thread = await asyncio.to_thread(deps.choose_thread, new_thread_id, None)
    preferred_project_channel_id = deps.resolve_project_channel_id(
        discord_channel_id,
        deps.get_project_key(codex_thread),
    )
    discord_thread = await deps.mirror_single_codex_thread(
        bot,
        new_thread_id,
        preferred_project_channel_id=preferred_project_channel_id,
    )
    deps.log(
        f"new_thread_mirrored codex_thread={new_thread_id} "
        + f"discord_thread={discord_thread.id}"
    )
    prepared = await deps.prepare_mapped_session_mirror_output(discord_thread, new_thread_id)
    deps.log(
        f"new_thread_session_mirror_prepared codex_thread={new_thread_id} "
        + f"discord_thread={discord_thread.id} prepared={prepared}"
    )
    return f"Mirrored Discord thread: <#{discord_thread.id}>"


async def run_discord_new_thread(
    bot: BotT,
    discord_channel_id: int | None,
    prompt: str,
    *,
    deps: NewThreadFlowDeps[BotT, CodexThreadT, DiscordThreadT],
) -> tuple[int, str]:
    argv = _build_new_thread_argv(discord_channel_id, prompt, deps=deps)
    exit_code, output = await asyncio.to_thread(deps.run_bridge_command, argv)
    new_thread_id = (
        deps.parse_bridge_output_value(output, "target_thread")
        or deps.parse_bridge_output_value(output, "selected_thread")
    )
    prefix = format_discord_new_thread_prefix(exit_code, new_thread_id)
    parts = [f"{prefix}\n\n{output or '(no output)'}"]
    if new_thread_id:
        try:
            parts.append(
                await _mirror_new_thread(
                    bot,
                    discord_channel_id,
                    new_thread_id,
                    deps=deps,
                )
            )
        except deps.delivery_exceptions as caught_exc:
            exc = caught_exc
            deps.log("new_thread_mirror_failed\n" + traceback.format_exc())
            parts.append(f"Mirror update failed: {type(exc).__name__}: {exc}")
        except sqlite3.Error as exc:
            deps.log("new_thread_mirror_failed\n" + traceback.format_exc())
            parts.append(f"Mirror update failed: {type(exc).__name__}: {exc}")
    elif exit_code == 0:
        deps.log("new_thread_mirror_skipped reason=no_thread_id")
        parts.append("Mirror update skipped: new thread id was not found in bridge output.")
    return exit_code, "\n\n".join(parts)
