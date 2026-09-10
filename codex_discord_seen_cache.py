from __future__ import annotations

import time
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

SeenCacheKey: TypeAlias = int | str
SeenCacheMap: TypeAlias = dict[SeenCacheKey, float]


class SeenCacheOwner(Protocol):
    pass


def get_or_create_seen_map(owner: SeenCacheOwner, attr_name: str) -> SeenCacheMap | None:
    raw_seen = getattr(owner, attr_name, None)
    if isinstance(raw_seen, dict):
        return cast(SeenCacheMap, raw_seen)
    seen: SeenCacheMap = {}
    try:
        setattr(owner, attr_name, seen)
    except (AttributeError, TypeError):
        return None
    return seen


def remember_limited_seen_key(
    seen: SeenCacheMap,
    key: SeenCacheKey,
    *,
    limit: int,
    monotonic: Callable[[], float] = time.monotonic,
) -> None:
    seen[key] = monotonic()
    if len(seen) <= limit:
        return
    for stale_key, _seen_at in sorted(seen.items(), key=lambda item: item[1])[: len(seen) - limit]:
        _ = seen.pop(stale_key, None)
