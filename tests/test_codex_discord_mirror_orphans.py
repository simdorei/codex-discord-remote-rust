from __future__ import annotations

import unittest
from collections.abc import AsyncIterator

import codex_discord_mirror_sync_result as mirror_sync_result
import codex_discord_mirror_orphans as mirror_orphans


class _DeleteFailure(Exception):
    pass


class _FakeThread:
    id: int
    owner_id: int | None
    name: str
    deleted: bool
    _delete_error: BaseException | None

    def __init__(
        self,
        thread_id: int,
        *,
        owner_id: int | None = None,
        name: str = "thread",
        delete_error: BaseException | None = None,
    ) -> None:
        self.id = thread_id
        self.owner_id = owner_id
        self.name = name
        self.deleted = False
        self._delete_error = delete_error

    async def delete(self, *, reason: str) -> None:
        if self._delete_error is not None:
            raise self._delete_error
        self.deleted = bool(reason)


async def _iter_threads(threads: list[_FakeThread]) -> AsyncIterator[_FakeThread]:
    for thread in threads:
        yield thread


class _FakeChannel:
    threads: list[_FakeThread]
    _archived_threads: list[_FakeThread]
    _archived_error: BaseException | None
    name: str

    def __init__(
        self,
        *,
        active_threads: list[_FakeThread] | None = None,
        archived_threads: list[_FakeThread] | None = None,
        archived_error: BaseException | None = None,
        name: str = "project",
    ) -> None:
        self.threads = active_threads or []
        self._archived_threads = archived_threads or []
        self._archived_error = archived_error
        self.name = name

    def archived_threads(self, *, limit: int) -> AsyncIterator[_FakeThread]:
        if self._archived_error is not None:
            raise self._archived_error
        return _iter_threads(self._archived_threads[:limit])


class MirrorOrphanCleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_cleanup_deletes_active_and_archived_orphan_threads(self) -> None:
        owned_active = _FakeThread(10, owner_id=123)
        owned_archived = _FakeThread(11, owner_id=None)
        known = _FakeThread(12, owner_id=123)
        foreign = _FakeThread(13, owner_id=999)
        channel = _FakeChannel(
            active_threads=[owned_active, known, foreign],
            archived_threads=[owned_archived],
        )

        result = await mirror_orphans.cleanup_orphan_discord_threads(
            [channel],
            {12},
            123,
            is_known_thread_id=lambda thread_id: thread_id == 12,
            delivery_exceptions=(_DeleteFailure,),
        )

        self.assertEqual(result["deleted"], 2)
        self.assertEqual(result["skipped"], 2)
        self.assertEqual(result["failed"], 0)
        self.assertTrue(owned_active.deleted)
        self.assertTrue(owned_archived.deleted)
        self.assertFalse(known.deleted)
        self.assertFalse(foreign.deleted)

    async def test_cleanup_dedupes_thread_seen_in_active_and_archived_lists(self) -> None:
        duplicate = _FakeThread(10, owner_id=123)
        channel = _FakeChannel(active_threads=[duplicate], archived_threads=[duplicate])

        result = await mirror_orphans.cleanup_orphan_discord_threads(
            [channel],
            set(),
            123,
            is_known_thread_id=lambda thread_id: False,
            delivery_exceptions=(_DeleteFailure,),
        )

        self.assertEqual(result["deleted"], 1)
        self.assertTrue(duplicate.deleted)

    async def test_cleanup_records_delete_and_archive_failures(self) -> None:
        failing_thread = _FakeThread(10, owner_id=123, delete_error=_DeleteFailure("delete"))
        channel = _FakeChannel(
            active_threads=[failing_thread],
            archived_error=_DeleteFailure("archive"),
        )

        result = await mirror_orphans.cleanup_orphan_discord_threads(
            [channel],
            set(),
            123,
            is_known_thread_id=lambda thread_id: False,
            delivery_exceptions=(_DeleteFailure,),
        )

        self.assertEqual(result["deleted"], 0)
        self.assertEqual(result["failed"], 2)
        self.assertEqual(len(mirror_sync_result.cleanup_errors(result)), 2)


if __name__ == "__main__":
    _ = unittest.main()
