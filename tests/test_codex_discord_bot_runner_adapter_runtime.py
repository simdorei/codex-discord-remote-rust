from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from types import ModuleType
import unittest
from unittest import mock

import codex_discord_bot_runner_adapter_runtime as adapter_runtime


class FakeClient:
    def __init__(self) -> None:
        self.external_work_guard: Callable[[], bool] | None = None

    def set_external_work_guard(self, guard: Callable[[], bool] | None) -> None:
        self.external_work_guard = guard

    def lifecycle_snapshot(self) -> object:
        return object()

    def start(self) -> None:
        return None

    def delivery_admission(self, _generation: int | None) -> object:
        return object()

    def notify_child_cleanup_blocker_changed(self) -> None:
        return None

    def get_thread_turn_states(self, *_args: object, **_kwargs: object) -> object:
        return object()

    def wait_for_turn_completion(self, *_args: object, **_kwargs: object) -> object:
        return object()


class BotRunnerAdapterRuntimeTests(unittest.TestCase):
    def test_durable_queue_registers_external_queue_guard_on_shared_client(self) -> None:
        module = ModuleType("fake_bot_runner_adapter")
        db_path = Path("mirror.sqlite")
        setattr(module, "MIRROR_DB_PATH", db_path)
        setattr(module, "log_line", lambda _text: None)
        client = FakeClient()

        with mock.patch.object(
            adapter_runtime.app_server_transport,
            "DEFAULT_CLIENT",
            client,
        ):
            _ = adapter_runtime.BotRunnerAdapterRuntime(
                module=module
            ).make_durable_queue_runtime()

        guard = client.external_work_guard
        self.assertIsNotNone(guard)
        assert guard is not None
        with mock.patch.object(
            adapter_runtime.store,
            "has_replayable_queue_jobs",
            return_value=True,
        ) as has_replayable_jobs:
            with mock.patch.object(
                adapter_runtime.store,
                "list_queue_jobs",
                side_effect=AssertionError("raw queue rows must not drive cleanup"),
            ):
                self.assertTrue(guard())
        has_replayable_jobs.assert_called_once_with(db_path)


if __name__ == "__main__":
    _ = unittest.main()
