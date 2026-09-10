from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from dataclasses import dataclass
import math
import time
from typing import Protocol, TypeAlias

from codex_discord_runtime import normalize_runner_key
from codex_discord_session_mirror import SessionMirrorState

NowFunc = Callable[[], float]
ExceptionTypes: TypeAlias = tuple[type[BaseException], ...]
LogFunc = Callable[[str], None]
SessionMirrorEnabledFunc = Callable[[], bool]
MirroredThreadLookup = Callable[[int | None], str | None]
PrimeSessionMirrorCursor = Callable[[str | None], int | None]
RolloutPathMissingChecker = Callable[[str | None], bool]
OutputTargetActivator = Callable[[str | None], None]
ExpiredOutputTargetHandler = Callable[[str, float], None]


class SessionMirrorOutputChannel(Protocol):
    @property
    def id(self) -> int | None: ...


@dataclass(frozen=True, slots=True)
class PrepareMappedSessionMirrorOutputDeps:
    session_mirror_enabled: SessionMirrorEnabledFunc
    get_mirrored_codex_thread_id: MirroredThreadLookup
    prime_session_mirror_cursor_for_target: PrimeSessionMirrorCursor
    session_mirror_rollout_path_missing: RolloutPathMissingChecker
    activate_session_mirror_output_target: OutputTargetActivator
    activate_pending_session_mirror_output_target: OutputTargetActivator
    expected_exceptions: ExceptionTypes
    log: LogFunc


def cleanup_active_session_mirror_output_targets(
    state: SessionMirrorState,
    *,
    active_ttl_seconds: float,
    now: float | None = None,
    now_func: NowFunc = time.monotonic,
    on_expired: ExpiredOutputTargetHandler | None = None,
) -> None:
    current = now_func() if now is None else now
    expired = [
        target
        for target, activated_at in state.active_output_targets.items()
        if current - activated_at > active_ttl_seconds
    ]
    if on_expired is None:
        return
    for target in expired:
        activated_at = state.active_output_targets.get(target)
        if activated_at is None or state.expiring_output_targets.get(target) == activated_at:
            continue
        state.expiring_output_targets[target] = activated_at
        on_expired(target, activated_at)


def activate_session_mirror_output_target(
    state: SessionMirrorState,
    target_thread_id: str | None,
    *,
    active_ttl_seconds: float,
    now_func: NowFunc = time.monotonic,
    on_expired: ExpiredOutputTargetHandler | None = None,
) -> None:
    if not target_thread_id:
        return
    current = now_func()
    key = normalize_runner_key(target_thread_id)
    previous_activation = state.active_output_targets.get(key)
    if previous_activation is not None and current <= previous_activation:
        current = math.nextafter(previous_activation, math.inf)
    cleanup_active_session_mirror_output_targets(
        state,
        active_ttl_seconds=active_ttl_seconds,
        now=current,
        on_expired=on_expired,
    )
    state.active_output_targets[key] = current
    state.pending_cursor_targets.discard(key)
    _ = state.expiring_output_targets.pop(key, None)


def activate_pending_session_mirror_output_target(
    state: SessionMirrorState,
    target_thread_id: str | None,
    *,
    active_ttl_seconds: float,
    now_func: NowFunc = time.monotonic,
    on_expired: ExpiredOutputTargetHandler | None = None,
) -> None:
    if not target_thread_id:
        return
    key = normalize_runner_key(target_thread_id)
    activate_session_mirror_output_target(
        state,
        target_thread_id,
        active_ttl_seconds=active_ttl_seconds,
        now_func=now_func,
        on_expired=on_expired,
    )
    state.pending_cursor_targets.add(key)


def deactivate_session_mirror_output_target(state: SessionMirrorState, target_thread_id: str | None) -> None:
    if not target_thread_id:
        return
    key = normalize_runner_key(target_thread_id)
    _ = state.active_output_targets.pop(key, None)
    state.pending_cursor_targets.discard(key)
    _ = state.expiring_output_targets.pop(key, None)


def get_session_mirror_output_target_activation(
    state: SessionMirrorState,
    target_thread_id: str | None,
) -> float | None:
    if not target_thread_id:
        return None
    return state.active_output_targets.get(normalize_runner_key(target_thread_id))


def deactivate_session_mirror_output_target_if_unchanged(
    state: SessionMirrorState,
    target_thread_id: str | None,
    expected_activation: float | None,
) -> bool:
    if not target_thread_id:
        return True
    key = normalize_runner_key(target_thread_id)
    if state.active_output_targets.get(key) != expected_activation:
        return False
    _ = state.active_output_targets.pop(key, None)
    state.pending_cursor_targets.discard(key)
    _ = state.expiring_output_targets.pop(key, None)
    return True


def clear_expiring_session_mirror_output_target(
    state: SessionMirrorState,
    target_thread_id: str,
    expected_activation: float,
) -> None:
    key = normalize_runner_key(target_thread_id)
    if state.expiring_output_targets.get(key) == expected_activation:
        _ = state.expiring_output_targets.pop(key, None)


def is_active_session_mirror_output_target(
    state: SessionMirrorState,
    target_thread_id: str | None,
    *,
    active_ttl_seconds: float,
    now_func: NowFunc = time.monotonic,
    on_expired: ExpiredOutputTargetHandler | None = None,
) -> bool:
    if not target_thread_id:
        return False
    cleanup_active_session_mirror_output_targets(
        state,
        active_ttl_seconds=active_ttl_seconds,
        now_func=now_func,
        on_expired=on_expired,
    )
    return normalize_runner_key(target_thread_id) in state.active_output_targets


def is_pending_session_mirror_cursor_target(
    state: SessionMirrorState,
    target_thread_id: str | None,
    *,
    active_ttl_seconds: float,
    now_func: NowFunc = time.monotonic,
    on_expired: ExpiredOutputTargetHandler | None = None,
) -> bool:
    if not target_thread_id:
        return False
    cleanup_active_session_mirror_output_targets(
        state,
        active_ttl_seconds=active_ttl_seconds,
        now_func=now_func,
        on_expired=on_expired,
    )
    return normalize_runner_key(target_thread_id) in state.pending_cursor_targets


def clear_pending_session_mirror_cursor_target(state: SessionMirrorState, target_thread_id: str | None) -> None:
    if not target_thread_id:
        return
    state.pending_cursor_targets.discard(normalize_runner_key(target_thread_id))


async def prepare_mapped_session_mirror_output(
    channel: SessionMirrorOutputChannel,
    target_thread_id: str | None,
    *,
    deps: PrepareMappedSessionMirrorOutputDeps,
) -> bool:
    if not deps.session_mirror_enabled() or not target_thread_id:
        return False
    channel_id = getattr(channel, "id", None)
    try:
        if deps.get_mirrored_codex_thread_id(channel_id) != target_thread_id:
            deps.log(
                f"session_mirror_output_prepare_skipped target={target_thread_id} "
                + f"reason=channel_not_mapped channel={channel_id or '-'}"
            )
            return False
    except deps.expected_exceptions as exc:
        deps.log(
            f"session_mirror_output_prepare_failed target={target_thread_id} "
            + f"reason=mapping_unavailable channel={channel_id or '-'} error_type={type(exc).__name__}"
        )
        return False
    cursor = await asyncio.to_thread(deps.prime_session_mirror_cursor_for_target, target_thread_id)
    if cursor is None:
        if await asyncio.to_thread(deps.session_mirror_rollout_path_missing, target_thread_id):
            deps.activate_pending_session_mirror_output_target(target_thread_id)
            deps.log(f"session_mirror_output_pending target={target_thread_id} reason=session_missing")
            return True
        deps.log(f"session_mirror_output_prepare_failed target={target_thread_id} reason=cursor_unavailable")
        return False
    deps.activate_session_mirror_output_target(target_thread_id)
    return True
