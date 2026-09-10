# pyright: reportArgumentType=false, reportAssignmentType=false, reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
import codex_discord_projects as discord_projects
from codex_thread_models import ThreadInfo


class BrokenPathError(ValueError):
    pass


class DiscordProjectFilterIntegrationTests(unittest.TestCase):
    def test_filter_mirrorable_threads_ignores_deleted_workspace_projects(self) -> None:
        original_global_state_path: Path = bridge.GLOBAL_STATE_PATH
        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                temp_path = Path(temp_dir)
                saved = temp_path / "saved"
                deleted = temp_path / "deleted"
                state_path = temp_path / "global.json"
                _ = state_path.write_text(
                    json.dumps(
                        {
                            "project-order": [str(saved)],
                            "electron-saved-workspace-roots": [str(saved)],
                        },
                    ),
                    encoding="utf-8",
                )
                bridge.GLOBAL_STATE_PATH = state_path
                threads = [
                    ThreadInfo(
                        id="saved-thread",
                        title="saved",
                        cwd=str(saved),
                        updated_at=1,
                        rollout_path="saved.jsonl",
                        model="gpt",
                        reasoning_effort="high",
                        tokens_used=1,
                    ),
                    ThreadInfo(
                        id="deleted-thread",
                        title="deleted",
                        cwd=str(deleted),
                        updated_at=2,
                        rollout_path="deleted.jsonl",
                        model="gpt",
                        reasoning_effort="high",
                        tokens_used=1,
                    ),
                    ThreadInfo(
                        id="projectless-thread",
                        title="chat",
                        cwd="",
                        updated_at=3,
                        rollout_path="chat.jsonl",
                        model="gpt",
                        reasoning_effort="high",
                        tokens_used=1,
                    ),
                ]

                filtered = bot.filter_mirrorable_threads(threads)

            self.assertEqual(
                [thread.id for thread in filtered],
                ["saved-thread", "projectless-thread"],
            )
        finally:
            bridge.GLOBAL_STATE_PATH = original_global_state_path

    def test_normalize_project_key_surfaces_normalization_error(self) -> None:
        class BrokenBridge:
            @staticmethod
            def normalize_workspace_path(path: str) -> str:
                raise BrokenPathError(f"bad path: {path}")

        with self.assertRaisesRegex(BrokenPathError, r"bad path: C:\\bad"):
            _ = discord_projects.normalize_project_key(
                r"C:\bad",
                bridge_module=BrokenBridge(),
                projectless_chat_key=bot.CODEX_PROJECTLESS_CHAT_KEY,
            )


if __name__ == "__main__":
    _ = unittest.main()
