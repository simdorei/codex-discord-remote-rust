from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_approval_view as discord_approval_view
import codex_discord_busy_choice_view as discord_busy_choice_view
import codex_discord_input_choice_view as discord_input_choice_view


@dataclass(frozen=True, slots=True)
class BotViewClassesRuntime:
    module: ModuleType

    def make_approval_view_class(self) -> type[discord_approval_view.ApprovalView]:
        runtime = self

        class ApprovalView(discord_approval_view.ApprovalView):
            def __init__(self, target_thread_id: str) -> None:
                super().__init__(target_thread_id, deps=runtime.make_approval_view_deps())

        return ApprovalView

    def make_input_choice_classes(
        self,
    ) -> tuple[type[discord_input_choice_view.InputChoiceButton], type[discord_input_choice_view.InputChoiceView]]:
        runtime = self

        class InputChoiceButton(discord_input_choice_view.InputChoiceButton):
            def __init__(self, target_thread_id: str, value: str, label: str) -> None:
                super().__init__(target_thread_id, value, label, deps=runtime.make_input_choice_view_deps())

        class InputChoiceView(discord_input_choice_view.InputChoiceView):
            def __init__(self, target_thread_id: str, options: list[tuple[str, str]]) -> None:
                super().__init__(target_thread_id, options, deps=runtime.make_input_choice_view_deps())

        return InputChoiceButton, InputChoiceView

    def make_busy_choice_view_class(self) -> type[discord_busy_choice_view.BusyChoiceView]:
        runtime = self

        class BusyChoiceView(discord_busy_choice_view.BusyChoiceView):
            def __init__(
                self,
                message: discord_busy_choice_view.BusyChoiceViewMessage,
                prompt: str,
                *,
                target_thread_id: str | None = None,
                allow_steer: bool = True,
                choice_id: str | None = None,
            ) -> None:
                super().__init__(
                    message,
                    prompt,
                    deps=runtime.make_busy_choice_view_deps(),
                    target_thread_id=target_thread_id,
                    allow_steer=allow_steer,
                    choice_id=choice_id,
                )

        return BusyChoiceView

    def make_approval_view_deps(self) -> discord_approval_view.ApprovalViewDeps:
        return cast(
            Callable[[], discord_approval_view.ApprovalViewDeps],
            getattr(self.module, "_make_approval_view_deps"),
        )()

    def make_input_choice_view_deps(self) -> discord_input_choice_view.InputChoiceViewDeps:
        return cast(
            Callable[[], discord_input_choice_view.InputChoiceViewDeps],
            getattr(self.module, "_make_input_choice_view_deps"),
        )()

    def make_busy_choice_view_deps(self) -> discord_busy_choice_view.BusyChoiceViewDeps:
        return cast(
            Callable[[], discord_busy_choice_view.BusyChoiceViewDeps],
            getattr(self.module, "_make_busy_choice_view_deps"),
        )()
