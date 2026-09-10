from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tempfile
import unittest

import codex_discord_project_types as project_types
import codex_discord_projects as projects


@dataclass(frozen=True, slots=True)
class FakeThread:
    cwd: str | None


class FakeChooseThreadError(RuntimeError):
    pass


class FakeBridge:
    GLOBAL_STATE_PATH: Path = Path("state.json")

    def __init__(
        self,
        *,
        data: project_types.JsonObject | None = None,
        threads: dict[str, FakeThread] | None = None,
        fail_choose: bool = False,
    ) -> None:
        self.data: project_types.JsonObject = {} if data is None else data
        self.threads: dict[str, FakeThread] = (
            {"thread-1": FakeThread(r"C:\Repo")} if threads is None else threads
        )
        self.fail_choose: bool = fail_choose

    def normalize_workspace_path(self, path: str) -> str:
        return path.replace("\\", "/").rstrip("/").lower()

    def strip_windows_extended_prefix(self, path: str) -> str:
        return path.removeprefix("\\\\?\\")

    def get_thread_workspace_name(self, thread: project_types.ProjectThread) -> str:
        return Path(str(thread.cwd or "")).name or "-"

    def load_json(self, path: Path) -> project_types.JsonObject:
        _ = path
        return self.data

    def choose_thread(self, thread_id: str, cwd: str | None) -> project_types.ProjectThread:
        _ = cwd
        if self.fail_choose:
            raise FakeChooseThreadError("Thread not found")
        return self.threads[thread_id]


class DiscordProjectHelperTests(unittest.TestCase):
    def test_project_types_are_reexported(self) -> None:
        self.assertIs(projects.BridgeProjectModule, project_types.BridgeProjectModule)
        self.assertIs(projects.ProjectThread, project_types.ProjectThread)
        self.assertIs(projects.GetThreadCwd, project_types.GetThreadCwd)
        self.assertIs(projects.ProjectKeysMatch, project_types.ProjectKeysMatch)

    def test_project_key_filtering_keeps_saved_and_projectless_threads(self) -> None:
        bridge: project_types.BridgeProjectModule = FakeBridge(
            data={"project-order": [r"C:\Repo"]},
        )
        projectless = FakeThread(r"C:\Users\me\Documents\Codex\2026-06-20\new-chat")
        chatgpt_projectless = FakeThread(
            r"C:\Users\me\Documents\Codex\2026-07-22\chatgpt-conversation-6a50"
        )
        unsaved = FakeThread(r"C:\Other")

        self.assertEqual(
            projects.get_project_key(
                FakeThread(r"\\?\C:\Repo"),
                bridge_module=bridge,
                projectless_chat_key="projectless-chat",
            ),
            "c:/repo",
        )
        self.assertEqual(
            projects.filter_mirrorable_threads(
                [FakeThread(r"C:\Repo"), projectless, chatgpt_projectless, unsaved],
                bridge_module=bridge,
                projectless_chat_key="projectless-chat",
            ),
            [FakeThread(r"C:\Repo"), projectless, chatgpt_projectless],
        )

    def test_chatgpt_conversation_cwd_uses_shared_projectless_chat(self) -> None:
        bridge: project_types.BridgeProjectModule = FakeBridge()
        thread = FakeThread(
            r"\\?\C:\Users\me\Documents\Codex\2026-07-22\chatgpt-conversation-b3f8"
        )

        self.assertTrue(
            projects.is_codex_projectless_chat_cwd(
                str(thread.cwd),
                bridge_module=bridge,
            )
        )
        self.assertEqual(
            projects.get_project_key(
                thread,
                bridge_module=bridge,
                projectless_chat_key="projectless-chat",
            ),
            "projectless-chat",
        )
        self.assertEqual(projects.get_project_name(thread, bridge_module=bridge), "채팅")

    def test_chatgpt_conversation_cwd_requires_codex_date_directory(self) -> None:
        bridge: project_types.BridgeProjectModule = FakeBridge()
        invalid_paths = (
            r"C:\Repo\chatgpt-conversation-b3f8",
            r"C:\Users\me\Documents\Codex\not-a-date\chatgpt-conversation-b3f8",
            r"C:\Users\me\Documents\Other\2026-07-22\chatgpt-conversation-b3f8",
        )

        for cwd in invalid_paths:
            with self.subTest(cwd=cwd):
                self.assertFalse(
                    projects.is_codex_projectless_chat_cwd(cwd, bridge_module=bridge)
                )

    def test_resolve_new_thread_cwd_prefers_mirrored_thread_then_project_channel(self) -> None:
        bridge: project_types.BridgeProjectModule = FakeBridge(
            threads={"thread-1": FakeThread(r"C:\Repo")},
        )

        self.assertEqual(
            projects.resolve_discord_new_thread_cwd(
                222,
                bridge_module=bridge,
                projectless_chat_key="projectless-chat",
                get_mirrored_codex_thread_id_func=lambda _channel_id: "thread-1",
                get_thread_cwd_func=lambda _thread_id: r"C:\Thread",
                get_mirror_project_for_channel_func=lambda _channel_id: (r"C:\Project", "Project"),
                find_projectless_new_chat_cwd_func=lambda: r"C:\NewChat",
            ),
            r"C:\Thread",
        )

        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(
                projects.resolve_discord_new_thread_cwd(
                    333,
                    bridge_module=bridge,
                    projectless_chat_key="projectless-chat",
                    get_mirrored_codex_thread_id_func=lambda _channel_id: None,
                    get_thread_cwd_func=lambda _thread_id: None,
                    get_mirror_project_for_channel_func=lambda _channel_id: (tmp, "Project"),
                    find_projectless_new_chat_cwd_func=lambda: r"C:\NewChat",
                ),
                tmp,
            )

    def test_get_thread_cwd_returns_none_for_missing_thread(self) -> None:
        bridge: project_types.BridgeProjectModule = FakeBridge(fail_choose=True)

        self.assertIsNone(projects.get_thread_cwd("missing", bridge_module=bridge))


if __name__ == "__main__":
    _ = unittest.main()
