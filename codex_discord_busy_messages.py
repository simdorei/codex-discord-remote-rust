from __future__ import annotations

from collections.abc import Callable
from typing import Protocol, TypeVar

FitSingleMessage = Callable[[str], str]
FormatLogTextLen = Callable[[str], str]
LogLine = Callable[[str], None]
SourceMessageT = TypeVar("SourceMessageT")
SourceMessageT_contra = TypeVar("SourceMessageT_contra", contravariant=True)
ViewT = TypeVar("ViewT")
ViewT_co = TypeVar("ViewT_co", covariant=True)


class BusyChoiceViewFactory(Protocol[SourceMessageT_contra, ViewT_co]):
    def __call__(
        self,
        source_message: SourceMessageT_contra,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool,
    ) -> ViewT_co: ...


def build_busy_choice_message(
    prompt: str,
    target_thread_id: str | None,
    *,
    discord_max_len: int,
    fit_single_message_func: FitSingleMessage,
) -> str:
    _ = target_thread_id
    lines = ["Codex app is still processing this mapped thread.", ""]
    footer = "\n\nChoose the Discord action for this message."
    prefix = "\n".join(lines)
    prompt_text = str(prompt or "")
    prompt_budget = max(0, discord_max_len - len(prefix) - len(footer))
    if len(prompt_text) > prompt_budget:
        suffix = "\n\n[prompt truncated for Discord]"
        prompt_text = prompt_text[: max(0, prompt_budget - len(suffix))].rstrip() + suffix
    return fit_single_message_func(prefix + prompt_text + footer)


def build_stale_busy_steer_block_message(
    prompt: str,
    *,
    target_ref: str,
    age_seconds: float,
    fit_single_message_func: FitSingleMessage,
) -> str:
    age_minutes = max(1, int(age_seconds // 60))
    prompt_text = str(prompt or "").strip()
    return fit_single_message_func(
        "\n".join(
            [
                "This Codex thread is busy but has not produced new output recently.",
                f"thread: {target_ref or 'selected'}",
                f"last Codex activity: about {age_minutes} min ago",
                "",
                "Steering will still be sent. This warning means the previous Codex turn may be stuck or slow.",
                "",
                f"message: {prompt_text}",
                "",
                "If the Codex app spinner is stuck, use the `Stop reply` button or `!stop` (`!stop <ref>` for another thread). Use `!new <prompt>` only when you want a fresh mirrored thread.",
            ]
        )
    )


def make_busy_choice_payload(
    source_message: SourceMessageT,
    prompt: str,
    *,
    target_thread_id: str | None,
    allow_steer: bool,
    build_busy_choice_message_func: Callable[[str, str | None], str],
    make_busy_choice_view_func: BusyChoiceViewFactory[SourceMessageT, ViewT],
) -> tuple[str, ViewT]:
    return (
        build_busy_choice_message_func(prompt, target_thread_id),
        make_busy_choice_view_func(
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
        ),
    )


def log_busy_choice_sent(
    reason: str,
    target_thread_id: str | None,
    prompt: str,
    *,
    log_func: LogLine,
    format_log_text_len_func: FormatLogTextLen,
) -> None:
    safe_reason = reason.replace("\n", " ")[:80]
    log_func(
        f"busy_choice_sent reason={safe_reason} target={target_thread_id or '-'} prompt_len={format_log_text_len_func(prompt)}"
    )


__all__ = [
    "BusyChoiceViewFactory",
    "FitSingleMessage",
    "FormatLogTextLen",
    "LogLine",
    "SourceMessageT",
    "SourceMessageT_contra",
    "ViewT",
    "ViewT_co",
    "build_busy_choice_message",
    "build_stale_busy_steer_block_message",
    "log_busy_choice_sent",
    "make_busy_choice_payload",
]
