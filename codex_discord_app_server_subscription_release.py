from __future__ import annotations

import asyncio
from collections.abc import Callable
from typing import Protocol, cast

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_app_server_transport_replies import CodexAppServerTransportError
from codex_app_server_transport_subscriptions import ThreadReleaseOutcome


_CAPTURE_ACTIVATION = object()


class SubscriptionReleaseClient(Protocol):
    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot: ...

    def release_thread_subscription_if_terminal(
        self,
        thread_id: str,
        *,
        expected_generation: int,
    ) -> ThreadReleaseOutcome: ...


async def release_session_mirror_output_target(
    thread_id: str | None,
    *,
    transport_enabled: Callable[[], bool],
    client: SubscriptionReleaseClient,
    get_output_target_activation: Callable[[str | None], float | None],
    deactivate_output_target_if_unchanged: Callable[[str | None, float | None], bool],
    log: Callable[[str], None],
    expected_activation: float | None | object = _CAPTURE_ACTIVATION,
) -> bool:
    if expected_activation is _CAPTURE_ACTIVATION:
        activation = get_output_target_activation(thread_id)
    else:
        activation = cast(float | None, expected_activation)
    if get_output_target_activation(thread_id) != activation:
        log(
            "session_mirror_subscription_release_deferred "
            + f"target={thread_id or '-'} reason=output_target_reactivated_before_release"
        )
        return False
    if thread_id is None or not transport_enabled():
        return deactivate_output_target_if_unchanged(thread_id, activation)

    snapshot = client.lifecycle_snapshot()
    if not snapshot.healthy or snapshot.generation <= 0:
        log(
            "session_mirror_subscription_release_deferred "
            + f"target={thread_id} reason=app_server_unhealthy "
            + f"generation={snapshot.generation}"
        )
        return False

    try:
        outcome = await asyncio.to_thread(
            client.release_thread_subscription_if_terminal,
            thread_id,
            expected_generation=snapshot.generation,
        )
    except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
        log(
            "session_mirror_subscription_release_failed "
            + f"target={thread_id} generation={snapshot.generation} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        return False
    if not outcome.released:
        log(
            "session_mirror_subscription_release_deferred "
            + f"target={thread_id} reason={outcome.status.value} "
            + f"generation={snapshot.generation}"
        )
        return False
    if not deactivate_output_target_if_unchanged(thread_id, activation):
        log(
            "session_mirror_subscription_release_deferred "
            + f"target={thread_id} reason=output_target_reactivated_after_release "
            + f"generation={snapshot.generation}"
        )
        return False
    return True


__all__ = [
    "SubscriptionReleaseClient",
    "release_session_mirror_output_target",
]
