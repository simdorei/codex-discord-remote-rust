from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_text import format_log_argv

LogFunc = Callable[[str], None]
ResolveTargetArgs = Callable[[int | None, str | None], list[str]]


class StopActionChannel(Protocol):
    @property
    def id(self) -> int | str | None: ...


class StopActionInteraction(Protocol):
    pass


class BridgeRunner(Protocol):
    def __call__(
        self,
        target: StopActionChannel,
        argv: list[str],
        title: str,
    ) -> Awaitable[tuple[int, str]]: ...


class DirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: StopActionInteraction,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class BusyChoiceStopActionDeps:
    resolve_target_args: ResolveTargetArgs
    run_bridge_and_send: BridgeRunner
    send_direct_followup: DirectFollowupSender
    log: LogFunc


async def handle_busy_choice_stop_action(
    interaction: StopActionInteraction,
    channel: StopActionChannel,
    target_thread_id: str | None,
    *,
    user_id: int,
    deps: BusyChoiceStopActionDeps,
) -> bool:
    argv = build_stop_argv(
        coerce_channel_id(channel),
        target_thread_id,
        resolve_target_args=deps.resolve_target_args,
    )
    target = target_thread_id or "-"
    deps.log(f"busy_choice_stop_start user={user_id} target={target} argv={format_log_argv(argv)}")
    await deps.send_direct_followup(
        interaction,
        "Stop request sent for this Codex reply.",
        log_prefix="button_followup",
        context="busy_choice_stop_requested",
    )
    exit_code, _output = await deps.run_bridge_and_send(channel, argv, "Stop")
    deps.log(f"busy_choice_stop_done user={user_id} target={target} exit={exit_code}")
    return True


def build_stop_argv(
    channel_id: int | None,
    target_thread_id: str | None,
    *,
    resolve_target_args: ResolveTargetArgs,
) -> list[str]:
    argv = ["stop"]
    if target_thread_id:
        argv.extend(["--thread-id", target_thread_id])
        return argv
    argv.extend(resolve_target_args(channel_id, None))
    return argv


def coerce_channel_id(channel: StopActionChannel) -> int | None:
    raw_channel_id = getattr(channel, "id", None)
    if raw_channel_id is None:
        return None
    try:
        return int(str(raw_channel_id))
    except ValueError:
        return None
