from __future__ import annotations

from collections.abc import AsyncIterator, Mapping
from dataclasses import dataclass, field
import sys
from types import MappingProxyType, ModuleType
from typing import override
import unittest

import codex_discord_mirror_access as mirror_access


@dataclass(frozen=True, slots=True)
class FakeThread:
    id: int
    archived: bool = False


@dataclass(frozen=True, slots=True)
class FakeParentChannel:
    id: int
    threads: tuple[FakeThread, ...] = ()
    archived: tuple[FakeThread, ...] = ()

    async def archived_threads(self, *, limit: int) -> AsyncIterator[FakeThread]:
        del limit
        for thread in self.archived:
            yield thread


def _empty_channels() -> Mapping[int, FakeThread | FakeParentChannel]:
    return MappingProxyType({})


def _empty_errors() -> Mapping[int, BaseException]:
    return MappingProxyType({})


@dataclass(frozen=True, slots=True)
class FakeMirrorAccessBot:
    cached: Mapping[int, FakeThread | FakeParentChannel] = field(default_factory=_empty_channels)
    fetched: Mapping[int, FakeThread | FakeParentChannel] = field(default_factory=_empty_channels)
    errors: Mapping[int, BaseException] = field(default_factory=_empty_errors)

    def get_channel(self, channel_id: int) -> FakeThread | FakeParentChannel | None:
        return self.cached.get(channel_id)

    async def fetch_channel(self, channel_id: int) -> FakeThread | FakeParentChannel:
        exc = self.errors.get(channel_id)
        if exc is not None:
            raise exc
        return self.fetched[channel_id]


class MirrorAccessNotFound(Exception):
    pass


class MirrorAccessAbort(BaseException):
    pass


class MirrorAccessInspectionTests(unittest.IsolatedAsyncioTestCase):
    _original_discord_module: ModuleType | None = None

    @override
    def setUp(self) -> None:
        self._original_discord_module = sys.modules.get("discord")
        fake_discord = ModuleType("discord")
        setattr(fake_discord, "DiscordException", MirrorAccessNotFound)
        setattr(fake_discord, "HTTPException", MirrorAccessNotFound)
        setattr(fake_discord, "Forbidden", MirrorAccessNotFound)
        setattr(fake_discord, "NotFound", MirrorAccessNotFound)
        sys.modules["discord"] = fake_discord

    @override
    def tearDown(self) -> None:
        if self._original_discord_module is None:
            _ = sys.modules.pop("discord", None)
            return
        sys.modules["discord"] = self._original_discord_module

    async def test_inspect_thread_access_accepts_direct_fetch_match(self) -> None:
        # Given
        bot = FakeMirrorAccessBot(fetched={333: FakeThread(333)})

        # When
        status = await mirror_access.inspect_thread_access(
            bot,
            parent_channel_id=111,
            discord_thread_id=333,
        )

        # Then
        self.assertEqual(status.accessible, mirror_access.ACCESS_TRUE)
        self.assertEqual(status.archived, mirror_access.ARCHIVED_FALSE)
        self.assertFalse(status.stale)
        self.assertEqual(status.reason, mirror_access.ACTIVE_MAPPING_REASON)

    async def test_inspect_thread_access_uses_active_parent_threads_after_fetch_miss(self) -> None:
        # Given
        bot = FakeMirrorAccessBot(
            cached={111: FakeParentChannel(111, threads=(FakeThread(333),))},
            errors={333: MirrorAccessNotFound("missing")},
        )

        # When
        status = await mirror_access.inspect_thread_access(
            bot,
            parent_channel_id=111,
            discord_thread_id=333,
        )

        # Then
        self.assertEqual(status.accessible, mirror_access.ACCESS_TRUE)
        self.assertEqual(status.archived, mirror_access.ARCHIVED_FALSE)
        self.assertFalse(status.stale)
        self.assertEqual(status.reason, mirror_access.ACTIVE_MAPPING_REASON)

    async def test_inspect_thread_access_uses_archived_parent_threads_after_fetch_miss(self) -> None:
        # Given
        bot = FakeMirrorAccessBot(
            cached={111: FakeParentChannel(111, archived=(FakeThread(333, archived=True),))},
            errors={333: MirrorAccessNotFound("missing")},
        )

        # When
        status = await mirror_access.inspect_thread_access(
            bot,
            parent_channel_id=111,
            discord_thread_id=333,
        )

        # Then
        self.assertEqual(status.accessible, mirror_access.ACCESS_TRUE)
        self.assertEqual(status.archived, mirror_access.ARCHIVED_TRUE)
        self.assertFalse(status.stale)
        self.assertEqual(status.reason, mirror_access.ACTIVE_MAPPING_REASON)

    async def test_inspect_thread_access_marks_unknown_channel_stale_when_absent(self) -> None:
        # Given
        bot = FakeMirrorAccessBot(
            cached={111: FakeParentChannel(111)},
            errors={333: MirrorAccessNotFound("404 Not Found (error code: 10003): Unknown Channel")},
        )

        # When
        status = await mirror_access.inspect_thread_access(
            bot,
            parent_channel_id=111,
            discord_thread_id=333,
        )

        # Then
        self.assertEqual(status.accessible, mirror_access.ACCESS_FALSE)
        self.assertEqual(status.archived, mirror_access.ARCHIVED_UNKNOWN)
        self.assertTrue(status.stale)
        self.assertEqual(status.reason, mirror_access.UNKNOWN_CHANNEL_REASON)

    async def test_inspect_thread_access_does_not_swallow_runtime_abort(self) -> None:
        # Given
        abort = MirrorAccessAbort("stop now")
        bot = FakeMirrorAccessBot(cached={111: FakeParentChannel(111)}, errors={333: abort})

        # When / Then
        with self.assertRaises(MirrorAccessAbort):
            _ = await mirror_access.inspect_thread_access(
                bot,
                parent_channel_id=111,
                discord_thread_id=333,
            )


if __name__ == "__main__":
    _ = unittest.main()
