from __future__ import annotations

from datetime import datetime
import threading
from typing import final


@final
class RenewableExpiry:
    __slots__ = ("_expires_at", "_lock")

    def __init__(self, expires_at: datetime) -> None:
        self._expires_at = expires_at
        self._lock = threading.Lock()

    @property
    def value(self) -> datetime:
        with self._lock:
            return self._expires_at

    def extend(self, expires_at: datetime) -> None:
        with self._lock:
            if expires_at > self._expires_at:
                self._expires_at = expires_at
