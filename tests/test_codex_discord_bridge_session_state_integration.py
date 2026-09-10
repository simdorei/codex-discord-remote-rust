from __future__ import annotations

import unittest
from typing import Protocol, cast

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
from codex_thread_models import ThreadInfo


class SyncSessionIndex(Protocol):
    def __call__(self) -> int:
        ...


class LoadRecentThreads(Protocol):
    def __call__(self, limit: int = 0) -> list[ThreadInfo]:
        ...


class GetSelectedThreadId(Protocol):
    def __call__(self) -> str | None:
        ...


class ChooseThread(Protocol):
    def __call__(self, thread_id: str | None = None, cwd: str | None = None) -> ThreadInfo:
        ...


class SetSelectedThreadId(Protocol):
    def __call__(self, thread_id: str | None) -> None:
        ...


class GetThreadWorkspaceRef(Protocol):
    def __call__(self, selected: ThreadInfo, threads: list[ThreadInfo] | None = None) -> str:
        ...


class GetThreadWorkspaceName(Protocol):
    def __call__(self, selected: ThreadInfo) -> str:
        ...


class WorkspaceUnavailableError(RuntimeError):
    pass


class WorkspaceRefBugError(TypeError):
    pass


def make_thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="title",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path="session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


class DiscordBridgeSessionStateIntegrationTests(unittest.TestCase):
    def test_refresh_codex_bridge_session_state_replaces_stale_selected_thread(self) -> None:
        original_sync_session_index = cast(
            SyncSessionIndex,
            getattr(bridge, "sync_session_index_with_state"),
        )
        original_load_recent_threads = cast(LoadRecentThreads, getattr(bridge, "load_recent_threads"))
        original_get_selected_thread_id = cast(GetSelectedThreadId, getattr(bridge, "get_selected_thread_id"))
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_set_selected_thread_id = cast(SetSelectedThreadId, getattr(bridge, "set_selected_thread_id"))
        original_get_thread_workspace_ref = cast(
            GetThreadWorkspaceRef,
            getattr(bridge, "get_thread_workspace_ref"),
        )
        selected_updates: list[str | None] = []
        try:
            thread = make_thread()

            def fake_load_recent_threads(limit: int = 0) -> list[ThreadInfo]:
                _ = limit
                return [thread]

            def fake_choose_thread(
                thread_id: str | None = None,
                cwd: str | None = None,
            ) -> ThreadInfo:
                _ = thread_id, cwd
                return thread

            def fake_set_selected_thread_id(thread_id: str | None) -> None:
                selected_updates.append(thread_id)

            def fake_get_thread_workspace_ref(
                selected: ThreadInfo,
                threads: list[ThreadInfo] | None = None,
            ) -> str:
                _ = selected, threads
                return "repo"

            setattr(bridge, "sync_session_index_with_state", lambda: 1)
            setattr(bridge, "load_recent_threads", fake_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", lambda: "stale-thread")
            setattr(bridge, "choose_thread", fake_choose_thread)
            setattr(bridge, "set_selected_thread_id", fake_set_selected_thread_id)
            setattr(bridge, "get_thread_workspace_ref", fake_get_thread_workspace_ref)

            state = bot.refresh_codex_bridge_session_state()

            self.assertEqual(state["selected_action"], "stale_replaced")
            self.assertEqual(state["selected_thread_id"], "thread-1")
            self.assertEqual(state["selected_ref"], "repo")
            self.assertEqual(selected_updates, ["thread-1"])
        finally:
            setattr(bridge, "sync_session_index_with_state", original_sync_session_index)
            setattr(bridge, "load_recent_threads", original_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", original_get_selected_thread_id)
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "set_selected_thread_id", original_set_selected_thread_id)
            setattr(bridge, "get_thread_workspace_ref", original_get_thread_workspace_ref)

    def test_refresh_codex_bridge_session_state_workspace_ref_runtime_error_falls_back_to_name(self) -> None:
        original_sync_session_index = cast(
            SyncSessionIndex,
            getattr(bridge, "sync_session_index_with_state"),
        )
        original_load_recent_threads = cast(LoadRecentThreads, getattr(bridge, "load_recent_threads"))
        original_get_selected_thread_id = cast(GetSelectedThreadId, getattr(bridge, "get_selected_thread_id"))
        original_get_thread_workspace_ref = cast(
            GetThreadWorkspaceRef,
            getattr(bridge, "get_thread_workspace_ref"),
        )
        original_get_thread_workspace_name = cast(
            GetThreadWorkspaceName,
            getattr(bridge, "get_thread_workspace_name"),
        )
        try:
            thread = make_thread()

            def fake_load_recent_threads(limit: int = 0) -> list[ThreadInfo]:
                _ = limit
                return [thread]

            def fake_get_thread_workspace_ref(
                selected: ThreadInfo,
                threads: list[ThreadInfo] | None = None,
            ) -> str:
                _ = selected, threads
                raise WorkspaceUnavailableError("workspace unavailable")

            def fake_get_thread_workspace_name(selected: ThreadInfo) -> str:
                _ = selected
                return "fallback-repo"

            setattr(bridge, "sync_session_index_with_state", lambda: 1)
            setattr(bridge, "load_recent_threads", fake_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", lambda: "thread-1")
            setattr(bridge, "get_thread_workspace_ref", fake_get_thread_workspace_ref)
            setattr(bridge, "get_thread_workspace_name", fake_get_thread_workspace_name)

            state = bot.refresh_codex_bridge_session_state()

            self.assertEqual(state["selected_action"], "kept")
            self.assertEqual(state["selected_thread_id"], "thread-1")
            self.assertEqual(state["selected_ref"], "fallback-repo")
        finally:
            setattr(bridge, "sync_session_index_with_state", original_sync_session_index)
            setattr(bridge, "load_recent_threads", original_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", original_get_selected_thread_id)
            setattr(bridge, "get_thread_workspace_ref", original_get_thread_workspace_ref)
            setattr(bridge, "get_thread_workspace_name", original_get_thread_workspace_name)

    def test_refresh_codex_bridge_session_state_workspace_ref_type_error_is_not_fallback(self) -> None:
        original_sync_session_index = cast(
            SyncSessionIndex,
            getattr(bridge, "sync_session_index_with_state"),
        )
        original_load_recent_threads = cast(LoadRecentThreads, getattr(bridge, "load_recent_threads"))
        original_get_selected_thread_id = cast(GetSelectedThreadId, getattr(bridge, "get_selected_thread_id"))
        original_get_thread_workspace_ref = cast(
            GetThreadWorkspaceRef,
            getattr(bridge, "get_thread_workspace_ref"),
        )
        original_get_thread_workspace_name = cast(
            GetThreadWorkspaceName,
            getattr(bridge, "get_thread_workspace_name"),
        )
        try:
            thread = make_thread()

            def fake_load_recent_threads(limit: int = 0) -> list[ThreadInfo]:
                _ = limit
                return [thread]

            def fake_get_thread_workspace_ref(
                selected: ThreadInfo,
                threads: list[ThreadInfo] | None = None,
            ) -> str:
                _ = selected, threads
                raise WorkspaceRefBugError("workspace ref bug")

            def fake_get_thread_workspace_name(selected: ThreadInfo) -> str:
                _ = selected
                return "fallback-repo"

            setattr(bridge, "sync_session_index_with_state", lambda: 1)
            setattr(bridge, "load_recent_threads", fake_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", lambda: "thread-1")
            setattr(bridge, "get_thread_workspace_ref", fake_get_thread_workspace_ref)
            setattr(bridge, "get_thread_workspace_name", fake_get_thread_workspace_name)

            with self.assertRaisesRegex(TypeError, "workspace ref bug"):
                _ = bot.refresh_codex_bridge_session_state()
        finally:
            setattr(bridge, "sync_session_index_with_state", original_sync_session_index)
            setattr(bridge, "load_recent_threads", original_load_recent_threads)
            setattr(bridge, "get_selected_thread_id", original_get_selected_thread_id)
            setattr(bridge, "get_thread_workspace_ref", original_get_thread_workspace_ref)
            setattr(bridge, "get_thread_workspace_name", original_get_thread_workspace_name)


if __name__ == "__main__":
    _ = unittest.main()
