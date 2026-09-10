from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from functools import wraps
from typing import ParamSpec, TypeVar
from weakref import WeakKeyDictionary

P = ParamSpec("P")
ResultT = TypeVar("ResultT")

_LOOP_LOCKS: WeakKeyDictionary[asyncio.AbstractEventLoop, asyncio.Lock] = WeakKeyDictionary()


def _current_loop_lock() -> asyncio.Lock:
    loop = asyncio.get_running_loop()
    lock = _LOOP_LOCKS.get(loop)
    if lock is None:
        lock = asyncio.Lock()
        _LOOP_LOCKS[loop] = lock
    return lock


def serialize_mirror_mutation(
    operation: Callable[P, Awaitable[ResultT]],
) -> Callable[P, Awaitable[ResultT]]:
    @wraps(operation)
    async def serialized(*args: P.args, **kwargs: P.kwargs) -> ResultT:
        async with _current_loop_lock():
            return await operation(*args, **kwargs)

    return serialized
