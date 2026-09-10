from __future__ import annotations

from contextlib import contextmanager
from contextvars import ContextVar
from collections.abc import Generator


_DISCORD_REMOTE_STOP: ContextVar[bool] = ContextVar("discord_remote_stop", default=False)


@contextmanager
def discord_remote_stop_scope() -> Generator[None]:
    token = _DISCORD_REMOTE_STOP.set(True)
    try:
        yield
    finally:
        _DISCORD_REMOTE_STOP.reset(token)


def is_discord_remote_stop() -> bool:
    return _DISCORD_REMOTE_STOP.get()
