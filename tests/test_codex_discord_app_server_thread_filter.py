from __future__ import annotations

import unittest

import codex_discord_app_server_thread_filter as thread_filter
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject
from codex_thread_models import ThreadInfo


def _thread(thread_id: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=thread_id,
        cwd="C:\\repo",
        updated_at=1,
        rollout_path=f"{thread_id}.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


class FakeClient:
    def __init__(self, outcomes: list[bool | Exception], *, restart_allowed: bool = True) -> None:
        self.outcomes = outcomes
        self.reads: list[str] = []
        self.restart_count = 0
        self.restart_allowed = restart_allowed

    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject:
        self.reads.append(thread_id)
        _ = include_turns
        outcome = self.outcomes.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return {"ok": outcome}

    def try_restart_if_quiescent(self) -> bool:
        self.restart_count += 1
        return self.restart_allowed


class AppServerThreadFilterTests(unittest.TestCase):
    def test_refreshes_app_server_once_when_thread_not_found_then_keeps_recovered_thread(self) -> None:
        client = FakeClient(
            [
                CodexAppServerTransportError("thread/read failed: Thread not found: thread-1"),
                True,
            ]
        )
        logs: list[str] = []

        filtered = thread_filter.filter_app_server_available_threads_with_deps(
            [_thread("thread-1")],
            deps=thread_filter.AppServerThreadFilterDeps(
                app_server_transport_enabled=lambda: True,
                get_client=lambda: client,
                log=logs.append,
            ),
        )

        self.assertEqual([thread.id for thread in filtered], ["thread-1"])
        self.assertEqual(client.restart_count, 1)
        self.assertEqual(client.reads, ["thread-1", "thread-1"])
        self.assertIn("mirror_sync_app_server_refresh_recovered target=thread-1", logs)

    def test_excludes_thread_when_still_not_found_after_refresh(self) -> None:
        client = FakeClient(
            [
                CodexAppServerTransportError("thread/read failed: Thread not found: thread-1"),
                CodexAppServerTransportError("thread/read failed: Thread not found: thread-1"),
            ]
        )
        logs: list[str] = []

        filtered = thread_filter.filter_app_server_available_threads_with_deps(
            [_thread("thread-1")],
            deps=thread_filter.AppServerThreadFilterDeps(
                app_server_transport_enabled=lambda: True,
                get_client=lambda: client,
                log=logs.append,
            ),
        )

        self.assertEqual(filtered, [])
        self.assertEqual(client.restart_count, 1)
        self.assertIn("mirror_sync_app_server_thread_unavailable target=thread-1 error=thread_not_found", logs)

    def test_surfaces_non_thread_not_found_transport_errors(self) -> None:
        client = FakeClient([CodexAppServerTransportError("thread/read failed: transport down")])

        with self.assertRaises(CodexAppServerTransportError):
            _ = thread_filter.filter_app_server_available_threads_with_deps(
                [_thread("thread-1")],
                deps=thread_filter.AppServerThreadFilterDeps(
                    app_server_transport_enabled=lambda: True,
                    get_client=lambda: client,
                    log=lambda _message: None,
                ),
            )

        self.assertEqual(client.restart_count, 0)

    def test_does_not_restart_or_retry_when_shared_transport_is_busy(self) -> None:
        client = FakeClient(
            [CodexAppServerTransportError("thread/read failed: Thread not found: thread-1")],
            restart_allowed=False,
        )
        logs: list[str] = []

        filtered = thread_filter.filter_app_server_available_threads_with_deps(
            [_thread("thread-1")],
            deps=thread_filter.AppServerThreadFilterDeps(
                app_server_transport_enabled=lambda: True,
                get_client=lambda: client,
                log=logs.append,
            ),
        )

        self.assertEqual(filtered, [])
        self.assertEqual(client.restart_count, 1)
        self.assertEqual(client.reads, ["thread-1"])
        self.assertIn("mirror_sync_app_server_refresh_deferred target=thread-1", logs)


if __name__ == "__main__":
    unittest.main()
