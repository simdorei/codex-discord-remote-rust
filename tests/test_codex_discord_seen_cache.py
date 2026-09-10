from __future__ import annotations

import unittest
from typing import ClassVar

import codex_discord_seen_cache as seen_cache


class _Owner:
    __slots__: ClassVar[tuple[str, ...]] = ("_seen",)
    _seen: seen_cache.SeenCacheMap | None

    def __init__(self, seen: seen_cache.SeenCacheMap | None = None) -> None:
        self._seen = seen

    @property
    def seen(self) -> seen_cache.SeenCacheMap | None:
        return self._seen


class _FrozenOwner:
    __slots__: ClassVar[tuple[str, ...]] = ()


class DiscordSeenCacheTests(unittest.TestCase):
    def test_get_or_create_seen_map_reuses_existing_map(self) -> None:
        existing: seen_cache.SeenCacheMap = {1: 1.0}
        owner = _Owner(existing)

        self.assertIs(seen_cache.get_or_create_seen_map(owner, "_seen"), existing)

    def test_get_or_create_seen_map_creates_missing_map(self) -> None:
        owner = _Owner()

        seen = seen_cache.get_or_create_seen_map(owner, "_seen")

        self.assertEqual(seen, {})
        self.assertIs(owner.seen, seen)

    def test_get_or_create_seen_map_returns_none_when_owner_rejects_attr(self) -> None:
        self.assertIsNone(seen_cache.get_or_create_seen_map(_FrozenOwner(), "_seen"))

    def test_remember_limited_seen_key_prunes_oldest_entries(self) -> None:
        ticks = iter([10.0, 20.0, 30.0])

        def monotonic() -> float:
            return next(ticks)

        seen: seen_cache.SeenCacheMap = {}
        seen_cache.remember_limited_seen_key(seen, "oldest", limit=2, monotonic=monotonic)
        seen_cache.remember_limited_seen_key(seen, "middle", limit=2, monotonic=monotonic)
        seen_cache.remember_limited_seen_key(seen, "newest", limit=2, monotonic=monotonic)

        self.assertEqual(set(seen), {"middle", "newest"})


if __name__ == "__main__":
    _ = unittest.main()
