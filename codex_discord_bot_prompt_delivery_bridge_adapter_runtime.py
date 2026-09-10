from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

import codex_discord_prompt_busy_result as discord_prompt_busy_result
from codex_thread_models import ThreadInfo


class SessionOffsetsPromptDeliveryBridge(Protocol):
    def wait_for_prompt_delivery(
        self,
        session_offsets: discord_prompt_busy_result.RecentOffsets,
        prompt: str,
        timeout_sec: float = 4,
    ) -> ThreadInfo | None: ...

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str | None: ...

    def get_thread_label(self, thread: ThreadInfo) -> str: ...


@dataclass(frozen=True, slots=True)
class PromptDeliveryBridgeAdapter:
    bridge: SessionOffsetsPromptDeliveryBridge

    def wait_for_prompt_delivery(
        self,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        prompt: str,
        *,
        timeout_sec: float,
    ) -> ThreadInfo | None:
        return self.bridge.wait_for_prompt_delivery(
            recent_offsets,
            prompt,
            timeout_sec=timeout_sec,
        )

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str | None:
        return self.bridge.get_thread_workspace_ref(thread)

    def get_thread_label(self, thread: ThreadInfo) -> str:
        return self.bridge.get_thread_label(thread)
