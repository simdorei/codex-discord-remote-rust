from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast

import codex_discord_store as store


class _FakeDiscordId:
    id: int

    def __init__(self, value: int) -> None:
        self.id = value


class _FakeBusyMessage:
    author: _FakeDiscordId
    channel: _FakeDiscordId

    def __init__(self, *, author_id: int, channel_id: int) -> None:
        self.author = _FakeDiscordId(author_id)
        self.channel = _FakeDiscordId(channel_id)


class _FakePersistentMessage:
    id: int

    def __init__(self, value: int) -> None:
        self.id = value


class _FakePersistentInteraction:
    message: _FakePersistentMessage | None

    def __init__(self, *, message_id: int | None) -> None:
        self.message = None if message_id is None else _FakePersistentMessage(message_id)


class StoreBusyChoiceTests(unittest.TestCase):
    def _db_path(self, temp_dir: str) -> Path:
        db_path = Path(temp_dir) / "mirror.sqlite"
        store.init_mirror_db(db_path)
        return db_path

    def _insert_busy_choice(
        self,
        conn: sqlite3.Connection,
        choice_id: str,
        *,
        expires_at: float,
        claimed_at: float | None,
    ) -> None:
        _ = conn.execute(
            "INSERT INTO busy_choices ("
            + "choice_id, owner_user_id, channel_id, target_thread_id, prompt, "
            + "allow_steer, created_at, expires_at, claimed_at"
            + ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                choice_id,
                123,
                456,
                "thread-1",
                f"prompt-{choice_id}",
                1,
                1.0,
                expires_at,
                claimed_at,
            ),
        )

    def test_busy_choice_create_get_and_claim_are_one_use(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            message = _FakeBusyMessage(author_id=123, channel_id=456)

            choice_id = store.create_busy_choice_record(
                db_path,
                message,
                "choose",
                "thread-1",
                allow_steer=True,
                ttl_seconds=60.0,
            )

            self.assertRegex(choice_id, r"^[0-9a-f]{24}$")
            record = store.get_busy_choice_record(db_path, choice_id)
            if record is None:
                self.fail("busy choice record was not persisted")
            expires_at = record["expires_at"]
            created_at = record["created_at"]
            self.assertEqual(
                record,
                {
                    "choice_id": choice_id,
                    "owner_user_id": 123,
                    "channel_id": 456,
                    "target_thread_id": "thread-1",
                    "prompt": "choose",
                    "allow_steer": True,
                    "created_at": created_at,
                    "expires_at": expires_at,
                },
            )
            self.assertIsInstance(expires_at, float)
            self.assertIsInstance(created_at, float)
            self.assertTrue(store.claim_busy_choice_record(db_path, choice_id))
            self.assertFalse(store.claim_busy_choice_record(db_path, choice_id))
            self.assertIsNone(store.get_busy_choice_record(db_path, choice_id))

    def test_busy_choice_cleanup_counts_and_expired_read_delete(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_busy_choice(
                    conn,
                    "active",
                    expires_at=200.0,
                    claimed_at=None,
                )
                self._insert_busy_choice(
                    conn,
                    "expired",
                    expires_at=50.0,
                    claimed_at=None,
                )
                self._insert_busy_choice(
                    conn,
                    "claimed",
                    expires_at=200.0,
                    claimed_at=90.0,
                )

            self.assertEqual(store.get_busy_choice_counts(db_path, now=100.0), (1, 2))
            self.assertIsNone(store.get_busy_choice_record(db_path, "expired"))
            self.assertEqual(store.cleanup_expired_busy_choices(db_path, now=100.0), 1)
            self.assertEqual(store.get_busy_choice_counts(db_path, now=100.0), (1, 0))
            with sqlite3.connect(db_path) as conn:
                rows = cast(
                    list[tuple[str]],
                    conn.execute(
                        "SELECT choice_id FROM busy_choices ORDER BY choice_id"
                    ).fetchall(),
                )
            self.assertEqual(rows, [("active",)])

    def test_persistent_component_claims_are_one_use_and_counted(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            interaction = _FakePersistentInteraction(message_id=999)

            self.assertTrue(
                store.claim_persistent_component_interaction(
                    db_path,
                    interaction,
                    "codex_approval:thread-1:2",
                    ttl_seconds=60.0,
                )
            )
            self.assertFalse(
                store.claim_persistent_component_interaction(
                    db_path,
                    interaction,
                    "codex_approval:thread-1:3",
                    ttl_seconds=60.0,
                )
            )
            self.assertTrue(
                store.claim_persistent_component_interaction(
                    db_path,
                    interaction,
                    "not-persistent",
                    ttl_seconds=60.0,
                )
            )
            self.assertEqual(store.get_persistent_component_claim_counts(db_path)[0], 1)

            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO persistent_component_claims ("
                    + "claim_key, created_at, expires_at"
                    + ") VALUES (?, ?, ?)",
                    ("stale", 1.0, 10.0),
                )

            self.assertEqual(
                store.get_persistent_component_claim_counts(db_path, now=20.0),
                (1, 1),
            )
            self.assertEqual(
                store.cleanup_expired_persistent_component_claims(db_path, now=20.0),
                1,
            )
            self.assertEqual(
                store.get_persistent_component_claim_counts(db_path, now=20.0),
                (1, 0),
            )


if __name__ == "__main__":
    _ = unittest.main()
