from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol


LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]
RelayStalePredicate = Callable[[str | None, int], bool]


class StreamSuppressionRelay(Protocol):
    @property
    def suppressed_after_steering(self) -> bool: ...

    @property
    def relay_generation(self) -> int: ...

    @property
    def sent_live(self) -> bool: ...


@dataclass(frozen=True, slots=True)
class StreamSuppressionDeps:
    is_discord_relay_stale: RelayStalePredicate
    format_log_text_len: TextLenFunc
    log: LogFunc


def handle_stream_suppressed_after_steering(
    *,
    relay: StreamSuppressionRelay,
    target_thread_id: str | None,
    output: str,
    deps: StreamSuppressionDeps,
) -> bool:
    if not relay.suppressed_after_steering:
        return False
    target_ref = target_thread_id or "-"
    output_len = deps.format_log_text_len(output)
    if deps.is_discord_relay_stale(target_thread_id, relay.relay_generation):
        deps.log(
            f"ask_stream_suppressed_after_newer_relay target={target_ref} "
            + f"sent_live={relay.sent_live} output_len={output_len}"
        )
        return True
    deps.log(
        f"ask_stream_suppressed_after_steering target={target_ref} "
        + f"sent_live={relay.sent_live} output_len={output_len}"
    )
    return True
