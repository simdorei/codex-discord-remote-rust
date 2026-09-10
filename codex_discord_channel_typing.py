from __future__ import annotations

import sys
from collections.abc import Callable
from contextlib import asynccontextmanager
from types import TracebackType
from typing import Protocol, TypeGuard

type LogFunc = Callable[[str], None]


class AsyncTypingManager(Protocol):
    async def __aenter__(self) -> None: ...

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...


class TypingTarget(Protocol):
    def typing(self) -> AsyncTypingManager: ...


def _is_async_typing_manager(value: object) -> TypeGuard[AsyncTypingManager]:  # noqa: OBJECT_OK
    return callable(getattr(value, "__aenter__", None)) and callable(getattr(value, "__aexit__", None))


@asynccontextmanager
async def channel_typing(
    target: object,  # noqa: OBJECT_OK - Discord channels are duck-typed at this boundary.
    *,
    context: str = "",
    log_func: LogFunc,
    raise_start_error: bool = False,
):
    typing_factory = getattr(target, "typing", None)
    if not callable(typing_factory):
        yield
        return

    try:
        candidate = typing_factory()
        if not _is_async_typing_manager(candidate):
            yield
            return
        manager = candidate
        await manager.__aenter__()
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - Discord typing manager boundary.
        log_func(f"typing_start_failed context={context or '-'} error_type={type(exc).__name__}")
        if raise_start_error:
            raise
        yield
        return

    exc_info: tuple[type[BaseException] | None, BaseException | None, TracebackType | None] = (
        None,
        None,
        None,
    )
    try:
        yield
    except BaseException:  # noqa: BROAD_EXCEPT_OK - preserve body exception for __aexit__.
        exc_info = sys.exc_info()
        raise
    finally:
        try:
            await manager.__aexit__(*exc_info)
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK - Discord typing manager boundary.
            log_func(f"typing_stop_failed context={context or '-'} error_type={type(exc).__name__}")
