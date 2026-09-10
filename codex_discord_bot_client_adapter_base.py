from __future__ import annotations

from collections.abc import Awaitable, Callable
from types import ModuleType
from typing import TypeAlias, cast

ModuleValue: TypeAlias = object


class BotClientAdapterBase:
    module: ModuleType

    def __init__(self, module: ModuleType) -> None:
        self.module = module

    def _runtime_func(self, runtime_name: str, method_name: str) -> Callable[..., ModuleValue]:
        runtime = cast(ModuleValue, getattr(self.module, runtime_name))
        return cast(Callable[..., ModuleValue], getattr(runtime, method_name))

    async def _await_runtime(
        self,
        runtime_name: str,
        method_name: str,
        *args: ModuleValue,
        **kwargs: ModuleValue,
    ) -> None:
        await cast(Awaitable[None], self._runtime_func(runtime_name, method_name)(*args, **kwargs))

    async def _await_runtime_value(
        self,
        runtime_name: str,
        method_name: str,
        *args: ModuleValue,
        **kwargs: ModuleValue,
    ) -> ModuleValue | None:
        return await cast(
            Awaitable[ModuleValue | None],
            self._runtime_func(runtime_name, method_name)(*args, **kwargs),
        )

    def _module_func(self, name: str) -> Callable[..., ModuleValue]:
        return cast(Callable[..., ModuleValue], getattr(self.module, name))

    def _log(self, message: str) -> None:
        cast(Callable[[str], None], getattr(self.module, "log_line"))(message)
