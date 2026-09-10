from __future__ import annotations

from pathlib import Path

import anyio

from codex_discord_stop_marker import StopMarkerLoopDeps, stop_marker_loop


def test_restart_marker_prepares_remote_mcp_handoff_before_bot_close(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        stop_path = tmp_path / "stop"
        heartbeat_path = tmp_path / "heartbeat"
        _ = stop_path.write_text(
            "identity=123|456\nmode=restart",
            encoding="utf-8",
        )
        closed = False
        events: list[str] = []

        async def close() -> None:
            nonlocal closed
            events.append("close")
            closed = True

        async def drain(*, timeout_seconds: float, reason: str) -> bool:
            _ = timeout_seconds, reason
            events.append("drain")
            return True

        def record_exit(exit_code: int, *, reason: str) -> None:
            events.append(f"exit:{exit_code}:{reason}")

        await stop_marker_loop(
            StopMarkerLoopDeps(
                stop_request_path=stop_path,
                heartbeat_path=heartbeat_path,
                process_identity="123|456",
                poll_seconds=0.01,
                drain_timeout_seconds=1,
                close_timeout_seconds=1,
                is_closed=lambda: closed,
                set_delivery_stopping=lambda reason: events.append(reason),
                wait_for_delivery_drain=drain,
                close_with_timeout=close,
                sleep=anyio.sleep,
                exit_bot_process=record_exit,
                log=lambda value: None,
                prepare_restart_handoff=lambda: events.append("handoff") or True,
            )
        )

        assert events == ["stop_marker", "drain", "handoff", "close"]

    anyio.run(scenario)


def test_ordinary_stop_does_not_prepare_remote_mcp_handoff(tmp_path: Path) -> None:
    async def scenario() -> None:
        stop_path = tmp_path / "stop"
        heartbeat_path = tmp_path / "heartbeat"
        _ = stop_path.write_text("identity=123|456", encoding="utf-8")
        closed = False
        prepare_calls = 0

        async def close() -> None:
            nonlocal closed
            closed = True

        async def drain(*, timeout_seconds: float, reason: str) -> bool:
            _ = timeout_seconds, reason
            return True

        def prepare() -> bool:
            nonlocal prepare_calls
            prepare_calls += 1
            return True

        def ignore_exit(exit_code: int, *, reason: str) -> None:
            _ = exit_code, reason

        await stop_marker_loop(
            StopMarkerLoopDeps(
                stop_request_path=stop_path,
                heartbeat_path=heartbeat_path,
                process_identity="123|456",
                poll_seconds=0.01,
                drain_timeout_seconds=1,
                close_timeout_seconds=1,
                is_closed=lambda: closed,
                set_delivery_stopping=lambda reason: None,
                wait_for_delivery_drain=drain,
                close_with_timeout=close,
                sleep=anyio.sleep,
                exit_bot_process=ignore_exit,
                log=lambda value: None,
                prepare_restart_handoff=prepare,
            )
        )

        assert prepare_calls == 0

    anyio.run(scenario)
