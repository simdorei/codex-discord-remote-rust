import unittest

import codex_thread_models as models


class ThreadModelsTests(unittest.TestCase):
    def test_thread_info_defaults_archived_at_to_zero(self) -> None:
        thread = models.ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="session.jsonl",
            model="gpt-5.5",
            reasoning_effort="xhigh",
            tokens_used=10,
        )

        self.assertEqual(thread.archived_at, 0)

    def test_window_info_reports_dimensions(self) -> None:
        window = models.WindowInfo(
            hwnd=1,
            title="Codex",
            left=10,
            top=20,
            right=110,
            bottom=70,
        )

        self.assertEqual(window.width, 100)
        self.assertEqual(window.height, 50)
