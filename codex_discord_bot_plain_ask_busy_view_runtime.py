from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_store as discord_store
ModuleValue: TypeAlias = object


BusyChoiceMessage: TypeAlias = discord_bot_shapes.BusyChoiceSourceMessage
BusyChoiceViewValue: TypeAlias = object


class BusyChoiceViewFactoryClass(Protocol):
    def __call__(
        self,
        message: BusyChoiceMessage,
        prompt: str,
        *,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
        choice_id: str | None = None,
    ) -> BusyChoiceViewValue: ...


@dataclass(frozen=True, slots=True)
class BotPlainAskBusyViewRuntime:
    module: ModuleType

    def make_busy_choice_view(
        self,
        source_message: BusyChoiceMessage,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool = True,
    ) -> BusyChoiceViewValue:
        choice_id = discord_store.create_busy_choice_record(
            cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
            cast(
                Callable[[BusyChoiceMessage], discord_bot_shapes.BusyChoiceStoreMessageLike],
                self._module_func("require_busy_choice_store_message"),
            )(source_message),
            prompt,
            target_thread_id,
            allow_steer=allow_steer,
            ttl_seconds=cast(float, getattr(self.module, "BUSY_CHOICE_TTL_SECONDS")),
        )
        return cast(BusyChoiceViewFactoryClass, getattr(self.module, "BusyChoiceView"))(
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
            choice_id=choice_id,
        )

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
