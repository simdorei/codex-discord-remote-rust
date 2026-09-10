"""Runtime state helpers for Discord runner and relay bookkeeping."""

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import AsyncGenerator, Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from types import TracebackType
import time
from typing import Protocol, TypeAlias, TypedDict


class RunnerQueueLike(Protocol):
    def qsize(self) -> int: ...


class RunnerLockLike(Protocol):
    async def __aenter__(self) -> None: ...

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None: ...


class RunnerState(TypedDict, total=False):
    queue: RunnerQueueLike
    active: bool
    target_thread_id: str | None


ResolveTargetRefFunc: TypeAlias = Callable[[str | None], tuple[str | None, str]]


@dataclass(slots=True)  # noqa: MUTABLE_OK
class DiscordRuntimeState:
    steering_handoffs: dict[str, float] = field(default_factory=dict)
    active_discord_relay_generations: dict[str, int] = field(default_factory=dict)
    recent_discord_origin_prompts: dict[str, float] = field(default_factory=dict)
    ask_delivery_locks: dict[str, asyncio.Lock] = field(default_factory=dict)
    active_direct_ask_lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    active_direct_ask_target_keys: set[str] = field(default_factory=set)
    codex_app_turn_condition: asyncio.Condition | None = None
    codex_app_active_target_key: str | None = None
    codex_app_active_target_count: int = 0


def normalize_runner_key(target_thread_id: str | None) -> str:
    return target_thread_id or "__selected__"


def mark_steering_handoff(
    handoffs: dict[str, float],
    target_thread_id: str | None,
    *,
    now_func: Callable[[], float] = time.monotonic,
) -> float:
    handoff_at = now_func()
    handoffs[normalize_runner_key(target_thread_id)] = handoff_at
    return handoff_at


def had_steering_handoff_since(
    handoffs: dict[str, float],
    target_thread_id: str | None,
    started_at: float,
) -> bool:
    return handoffs.get(normalize_runner_key(target_thread_id), 0.0) > started_at


def register_discord_relay(generations: dict[str, int], target_thread_id: str | None) -> int:
    key = normalize_runner_key(target_thread_id)
    generation = generations.get(key, 0) + 1
    generations[key] = generation
    return generation


def is_discord_relay_stale(
    generations: dict[str, int],
    target_thread_id: str | None,
    generation: int,
) -> bool:
    return generations.get(normalize_runner_key(target_thread_id), 0) > generation


def get_codex_app_turn_condition(state: DiscordRuntimeState) -> asyncio.Condition:
    if state.codex_app_turn_condition is None:
        state.codex_app_turn_condition = asyncio.Condition()
    return state.codex_app_turn_condition


@asynccontextmanager
async def codex_app_turn_slot(
    state: DiscordRuntimeState,
    target_thread_id: str | None,
    *,
    log: Callable[[str], None],
) -> AsyncGenerator[bool]:
    key = normalize_runner_key(target_thread_id)
    condition = get_codex_app_turn_condition(state)
    waited = False
    async with condition:
        while state.codex_app_active_target_key not in {None, key}:
            if not waited:
                log(
                    " ".join(
                        [
                            f"codex_app_turn_wait target={target_thread_id or '-'}",
                            f"active={state.codex_app_active_target_key or '-'}",
                        ]
                    )
                )
                waited = True
            _ = await condition.wait()
        if state.codex_app_active_target_key is None:
            state.codex_app_active_target_key = key
            state.codex_app_active_target_count = 1
        else:
            state.codex_app_active_target_count += 1
        if waited:
            log(
                " ".join(
                    [
                        f"codex_app_turn_wait_done target={target_thread_id or '-'}",
                        f"count={state.codex_app_active_target_count}",
                    ]
                )
            )

    try:
        yield waited
    finally:
        async with condition:
            if state.codex_app_active_target_key == key:
                state.codex_app_active_target_count = max(0, state.codex_app_active_target_count - 1)
                if state.codex_app_active_target_count == 0:
                    state.codex_app_active_target_key = None
                    condition.notify_all()
            else:
                log(
                    " ".join(
                        [
                            f"codex_app_turn_slot_mismatch target={target_thread_id or '-'}",
                            f"active={state.codex_app_active_target_key or '-'}",
                        ]
                    )
                )


async def build_runners_message(
    runners: Mapping[str, RunnerState],
    runners_lock: RunnerLockLike,
    *,
    resolve_target_ref_func: ResolveTargetRefFunc,
) -> str:
    async with runners_lock:
        items = list(runners.items())
    if not items:
        return "No active Discord runner queues."
    lines = ["Discord runner queues"]
    for key, runner in items:
        queue = runner.get("queue")
        queue_size = queue.qsize() if queue is not None else 0
        target_thread_id = str(runner.get("target_thread_id") or "").strip() or None
        _thread_id, target_ref = resolve_target_ref_func(target_thread_id)
        lines.append(
            f"- {target_ref}: active={bool(runner.get('active'))} queued={queue_size} key={key[:8]}"
        )
    return "\n".join(lines)
