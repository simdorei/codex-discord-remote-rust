from __future__ import annotations

import unittest

from codex_discord_mirror_rows import (
    MirrorCheckRow,
    MirrorCheckRowError,
    MirrorListRow,
    MirrorListRowError,
    mirror_check_row_from_db,
    mirror_list_row_from_db,
)


class MirrorRowConversionTests(unittest.TestCase):
    def test_list_row_converts_values_from_db(self) -> None:
        row: dict[str, str | int | None] = {
            "thread_title": None,
            "codex_thread_id": "codex-123",
            "project_name": None,
            "discord_thread_id": "456",
        }

        self.assertEqual(
            mirror_list_row_from_db(row),
            MirrorListRow(
                title="",
                codex_thread_id="codex-123",
                project_name="",
                discord_thread_id=456,
            ),
        )

    def test_list_row_missing_discord_thread_id_raises_typed_error(self) -> None:
        row: dict[str, str | int | None] = {
            "thread_title": "Thread",
            "codex_thread_id": "codex-123",
            "project_name": "repo",
            "discord_thread_id": None,
        }

        with self.assertRaises(MirrorListRowError) as caught:
            _ = mirror_list_row_from_db(row)

        self.assertIsInstance(caught.exception, RuntimeError)
        self.assertEqual(str(caught.exception), "Mirror list row is missing discord_thread_id.")

    def test_check_row_converts_values_from_db(self) -> None:
        row: dict[str, str | int | None] = {
            "codex_thread_id": None,
            "project_key": None,
            "discord_thread_id": "789",
        }

        self.assertEqual(
            mirror_check_row_from_db(row),
            MirrorCheckRow(
                codex_thread_id="",
                project_key="",
                discord_thread_id=789,
            ),
        )

    def test_check_row_missing_discord_thread_id_raises_typed_error(self) -> None:
        row: dict[str, str | int | None] = {
            "codex_thread_id": "codex-123",
            "project_key": "C:\\repo",
            "discord_thread_id": None,
        }

        with self.assertRaises(MirrorCheckRowError) as caught:
            _ = mirror_check_row_from_db(row)

        self.assertIsInstance(caught.exception, RuntimeError)
        self.assertEqual(str(caught.exception), "Mirror check row is missing discord_thread_id.")


if __name__ == "__main__":
    _ = unittest.main()
