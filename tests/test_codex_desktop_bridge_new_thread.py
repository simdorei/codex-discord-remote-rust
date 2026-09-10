from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_desktop_bridge_new_thread as new_thread


class ResolveNewThreadCwdTests(unittest.TestCase):
    def test_returns_existing_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            self.assertEqual(new_thread.resolve_new_thread_cwd(temp_dir), str(Path(temp_dir)))

    def test_rejects_missing_path_with_typed_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            missing = Path(temp_dir) / "missing"

            with self.assertRaises(new_thread.NewThreadCwdDoesNotExistError) as raised:
                _ = new_thread.resolve_new_thread_cwd(str(missing))

        self.assertEqual(str(raised.exception), f"New-thread cwd does not exist: {missing}")

    def test_rejects_file_path_with_typed_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            file_path = Path(temp_dir) / "not-a-directory.txt"
            _ = file_path.write_text("", encoding="utf-8")

            with self.assertRaises(new_thread.NewThreadCwdNotDirectoryError) as raised:
                _ = new_thread.resolve_new_thread_cwd(str(file_path))

        self.assertEqual(str(raised.exception), f"New-thread cwd is not a directory: {file_path}")


if __name__ == "__main__":
    _ = unittest.main()
