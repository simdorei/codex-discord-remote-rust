from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from codex_discord_store_connection import connect_store


ROOT = Path(__file__).resolve().parents[1]


class StoreConnectionTests(unittest.TestCase):
    def test_connection_policy_sets_busy_timeout(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            with connect_store(Path(temp_dir) / "mirror.sqlite") as conn:
                busy_timeout = int(conn.execute("PRAGMA busy_timeout").fetchone()[0])

        self.assertEqual(busy_timeout, 5_000)

    def test_discord_state_uses_the_shared_connection_factory(self) -> None:
        offenders = []
        for path in ROOT.glob("codex_discord*.py"):
            if path.name == "codex_discord_store_connection.py":
                continue
            if "sqlite3.connect" in path.read_text(encoding="utf-8"):
                offenders.append(path.name)

        self.assertEqual(offenders, [])


if __name__ == "__main__":
    _ = unittest.main()
