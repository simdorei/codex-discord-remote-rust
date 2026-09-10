from __future__ import annotations

import asyncio
from collections.abc import Callable
from dataclasses import dataclass

import codex_discord_app_server_subscription_release as subscription_release


@dataclass(frozen=True, slots=True)
class SessionMirrorReleaseRuntimeDeps:
    transport_enabled: Callable[[], bool]
    client: subscription_release.SubscriptionReleaseClient
    get_output_target_activation: Callable[[str | None], float | None]
    deactivate_output_target_if_unchanged: Callable[[str | None, float | None], bool]
    clear_expiring_output_target: Callable[[str, float], None]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class SessionMirrorReleaseRuntime:
    deps: SessionMirrorReleaseRuntimeDeps

    async def release_output_target(self, target_thread_id: str | None) -> bool:
        return await subscription_release.release_session_mirror_output_target(
            target_thread_id,
            transport_enabled=self.deps.transport_enabled,
            client=self.deps.client,
            get_output_target_activation=self.deps.get_output_target_activation,
            deactivate_output_target_if_unchanged=(
                self.deps.deactivate_output_target_if_unchanged
            ),
            log=self.deps.log,
        )

    def schedule_expired_output_target(
        self,
        target_thread_id: str,
        expected_activation: float,
    ) -> None:
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            self.deps.clear_expiring_output_target(
                target_thread_id,
                expected_activation,
            )
            self.deps.log(
                "session_mirror_expiry_release_deferred "
                + f"target={target_thread_id} reason=no_running_event_loop"
            )
            return
        task = loop.create_task(
            self._release_expired_output_target(
                target_thread_id,
                expected_activation,
            )
        )
        task.add_done_callback(
            lambda done: self._log_task_failure(target_thread_id, done)
        )

    async def _release_expired_output_target(
        self,
        target_thread_id: str,
        expected_activation: float,
    ) -> None:
        try:
            _ = await subscription_release.release_session_mirror_output_target(
                target_thread_id,
                transport_enabled=self.deps.transport_enabled,
                client=self.deps.client,
                get_output_target_activation=self.deps.get_output_target_activation,
                deactivate_output_target_if_unchanged=(
                    self.deps.deactivate_output_target_if_unchanged
                ),
                log=self.deps.log,
                expected_activation=expected_activation,
            )
        finally:
            self.deps.clear_expiring_output_target(
                target_thread_id,
                expected_activation,
            )

    def _log_task_failure(
        self,
        target_thread_id: str,
        task: asyncio.Task[None],
    ) -> None:
        if task.cancelled():
            return
        error = task.exception()
        if error is not None:
            self.deps.log(
                "session_mirror_expiry_release_failed "
                + f"target={target_thread_id} error_type={type(error).__name__} "
                + f"error={str(error)[:300]}"
            )


__all__ = [
    "SessionMirrorReleaseRuntime",
    "SessionMirrorReleaseRuntimeDeps",
]
