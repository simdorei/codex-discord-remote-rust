from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_prompt_busy_result as discord_prompt_busy_result
from codex_thread_models import ThreadInfo

LogFunc = Callable[[str], None]
TimeoutGetter = Callable[[], float]


class PromptDeliveryBridge(Protocol):
    def wait_for_prompt_delivery(
        self,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        prompt: str,
        *,
        timeout_sec: float,
    ) -> ThreadInfo | None: ...


class CodexThreadIdleWaiter(Protocol):
    def __call__(
        self,
        target_thread_id: str | None,
        *,
        timeout_sec: float,
    ) -> Awaitable[tuple[str, str | None, str]]: ...


@dataclass(frozen=True, slots=True)
class MirroredBusyDelegationDeps:
    bridge: PromptDeliveryBridge
    get_pending_watch_timeout: TimeoutGetter
    wait_for_codex_thread_idle: CodexThreadIdleWaiter
    log: LogFunc


async def wait_for_mirrored_busy_delegation_settle(
    prompt: str,
    *,
    target_thread_id: str | None,
    recent_offsets: discord_prompt_busy_result.RecentOffsets,
    deps: MirroredBusyDelegationDeps,
) -> None:
    if not target_thread_id:
        return
    timeout_sec = deps.get_pending_watch_timeout()
    delivered_id = "-"
    if recent_offsets:
        try:
            delivered_thread = await asyncio.to_thread(
                deps.bridge.wait_for_prompt_delivery,
                recent_offsets,
                prompt,
                timeout_sec=timeout_sec,
            )
        except (OSError, RuntimeError) as exc:
            deps.log(
                f"ask_busy_mirror_delivery_wait_failed target={target_thread_id or '-'} "
                + f"error={str(exc)[:200]}"
            )
            delivered_thread = None
        if delivered_thread is not None:
            delivered_id = delivered_thread.id
            deps.log(
                f"ask_busy_mirror_delivery_recorded target={target_thread_id or '-'} "
                + f"delivered={delivered_thread.id}"
            )
        else:
            deps.log(
                f"ask_busy_mirror_delivery_pending_timeout target={target_thread_id or '-'} "
                + f"timeout={timeout_sec:g}"
            )

    state, resolved_thread_id, target_ref = await deps.wait_for_codex_thread_idle(
        target_thread_id,
        timeout_sec=timeout_sec,
    )
    deps.log(
        f"ask_busy_mirror_idle_wait_done target={target_thread_id or '-'} "
        + f"state={state} resolved={resolved_thread_id or '-'} ref={target_ref or '-'} "
        + f"delivered={delivered_id}"
    )
