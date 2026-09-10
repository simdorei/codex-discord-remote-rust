from __future__ import annotations

from dataclasses import dataclass
from typing import Final

TRUNCATION_MARKER: Final = b"\n...[output truncated]...\n"


@dataclass(slots=True)
class BoundedProcessCapture:
    """Retain complete small output or deterministic head and tail bytes."""

    limit: int
    _complete: bytearray | None = None
    _head: bytearray | None = None
    _tail: bytearray | None = None
    total_bytes: int = 0

    def __post_init__(self) -> None:
        self._complete = bytearray()

    @property
    def truncated(self) -> bool:
        return self._complete is None

    def append(self, data: bytes) -> None:
        self.total_bytes += len(data)
        if self._complete is not None:
            if len(self._complete) + len(data) <= self.limit:
                self._complete.extend(data)
                return
            combined = bytes(self._complete) + data
            retained = self.limit - len(TRUNCATION_MARKER)
            head_limit = retained * 2 // 3
            tail_limit = retained - head_limit
            self._head = bytearray(combined[:head_limit])
            self._tail = bytearray(combined[-tail_limit:])
            self._complete = None
            return
        assert self._tail is not None
        tail_limit = self.limit - len(TRUNCATION_MARKER) - len(self._head or b"")
        self._tail.extend(data)
        if len(self._tail) > tail_limit:
            del self._tail[:-tail_limit]

    def value(self) -> bytes:
        if self._complete is not None:
            return bytes(self._complete)
        return bytes(self._head or b"") + TRUNCATION_MARKER + bytes(self._tail or b"")


__all__ = ["BoundedProcessCapture", "TRUNCATION_MARKER"]
