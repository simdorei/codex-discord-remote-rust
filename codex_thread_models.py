from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ThreadInfo:
    id: str
    title: str
    cwd: str
    updated_at: int
    rollout_path: str
    model: str
    reasoning_effort: str
    tokens_used: int
    archived_at: int = 0


@dataclass(slots=True)  # noqa: MUTABLE_OK
class ThreadContextUsage:
    """Mutable accumulator updated while scanning thread context events."""

    model_context_window: int
    last_input_tokens: int
    last_total_tokens: int
    peak_input_tokens: int
    peak_total_tokens: int
    usage_ratio: float
    inferred_compactions: int = 0
    last_compaction_before_input_tokens: int = 0
    last_compaction_after_input_tokens: int = 0


@dataclass(frozen=True, slots=True)
class WindowInfo:
    hwnd: int
    title: str
    left: int
    top: int
    right: int
    bottom: int

    @property
    def width(self) -> int:
        return self.right - self.left

    @property
    def height(self) -> int:
        return self.bottom - self.top
