from __future__ import annotations

import base64
import threading
from dataclasses import dataclass, field
from typing import override

import pytest

from codex_remote_mcp_computer import (
    ComputerController,
    execute_computer_operation,
)
from codex_remote_mcp_computer_contracts import ComputerActionPermit, ComputerCapture
from codex_remote_mcp_computer_errors import ComputerControlError
from simdorei_mcp_common.operation_outputs import (
    ComputerActionOutput,
    ComputerScreenshotOutput,
)
from simdorei_mcp_common.operation_requests import (
    ComputerClickRequest,
    ComputerScreenshotRequest,
    ComputerSetClipboardRequest,
)
from tests.remote_mcp_computer_fakes import (
    PNG_BYTES,
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


@dataclass(frozen=True, slots=True)
class BlockingClickPlatform(FakeComputerPlatform):
    started: threading.Event = field(default_factory=threading.Event)
    release: threading.Event = field(default_factory=threading.Event)

    @override
    def click(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        button: str,
        click_count: int,
    ) -> None:
        _ = permit, x, y, button, click_count
        self.started.set()
        if not self.release.wait(timeout=2):
            raise AssertionError("test click was not released")


@dataclass(frozen=True, slots=True)
class BlockingScreenshotPlatform(FakeComputerPlatform):
    started: threading.Event = field(default_factory=threading.Event)
    release: threading.Event = field(default_factory=threading.Event)

    @override
    def screenshot(self, window_id: int) -> ComputerCapture:
        self.started.set()
        if not self.release.wait(timeout=2):
            raise AssertionError("test screenshot was not released")
        return FakeComputerPlatform.screenshot(self, window_id)


def test_controller_requires_one_fresh_screenshot_per_input_action() -> None:
    platform = FakeComputerPlatform(computer_window())
    controller = make_controller(platform)
    screenshot = execute_computer_operation(
        ComputerScreenshotRequest(window_id=42),
        controller=controller,
    )
    assert isinstance(screenshot, ComputerScreenshotOutput)
    assert base64.b64decode(screenshot.data_base64) == PNG_BYTES

    result = execute_computer_operation(
        ComputerClickRequest(
            window_id=42,
            observation_id=screenshot.observation_id,
            x=120,
            y=80,
        ),
        controller=controller,
    )
    assert isinstance(result, ComputerActionOutput)
    assert platform.clicks == [(42, 120, 80, "left", 1)]

    with pytest.raises(ComputerControlError, match="fresh screenshot"):
        _ = execute_computer_operation(
            ComputerClickRequest(
                window_id=42,
                observation_id=screenshot.observation_id,
                x=120,
                y=80,
            ),
            controller=controller,
        )


def test_controller_rejects_coordinates_outside_observed_window() -> None:
    platform = FakeComputerPlatform(computer_window())
    controller = make_controller(platform)
    screenshot = execute_computer_operation(
        ComputerScreenshotRequest(window_id=42),
        controller=controller,
    )
    assert isinstance(screenshot, ComputerScreenshotOutput)

    with pytest.raises(ComputerControlError, match="outside"):
        _ = execute_computer_operation(
            ComputerClickRequest(
                window_id=42,
                observation_id=screenshot.observation_id,
                x=801,
                y=80,
            ),
            controller=controller,
        )


def test_clipboard_write_consumes_a_window_observation() -> None:
    platform = FakeComputerPlatform(computer_window())
    controller = make_controller(platform)
    screenshot = execute_computer_operation(
        ComputerScreenshotRequest(window_id=42),
        controller=controller,
    )
    assert isinstance(screenshot, ComputerScreenshotOutput)

    result = execute_computer_operation(
        ComputerSetClipboardRequest(
            window_id=42,
            observation_id=screenshot.observation_id,
            text="bound clipboard",
        ),
        controller=controller,
    )

    assert isinstance(result, ComputerActionOutput)
    assert platform.clipboards == [(42, "bound clipboard")]


def test_clipboard_request_without_observation_is_rejected() -> None:
    with pytest.raises(ValueError, match="window_id"):
        _ = ComputerSetClipboardRequest.model_validate({"text": "unbound"})


def test_observation_expires_at_exact_ttl_boundary() -> None:
    now = [100.0]
    platform = FakeComputerPlatform(computer_window())
    controller = ComputerController(
        platform,
        clock=lambda: now[0],
        token_factory=lambda: "observation-expiry-token",
    )
    screenshot = execute_computer_operation(
        ComputerScreenshotRequest(window_id=42),
        controller=controller,
    )
    assert isinstance(screenshot, ComputerScreenshotOutput)
    now[0] = 130.0

    with pytest.raises(ComputerControlError, match="fresh screenshot"):
        _ = execute_computer_operation(
            ComputerClickRequest(
                window_id=42,
                observation_id=screenshot.observation_id,
                x=10,
                y=10,
            ),
            controller=controller,
        )


def test_new_screenshot_revokes_prior_observation_and_stop_revokes_permit() -> None:
    platform = FakeComputerPlatform(computer_window())
    tokens = iter(("observation-token-one", "observation-token-two"))
    controller = ComputerController(
        platform,
        clock=lambda: 100.0,
        token_factory=lambda: next(tokens),
    )
    first = controller.screenshot(42)
    second = controller.screenshot(42)
    with pytest.raises(ComputerControlError, match="fresh screenshot"):
        _ = controller.consume_observation(first.observation_id, 42)

    permit = controller.consume_observation(second.observation_id, 42)
    controller.stop()
    with pytest.raises(ComputerControlError, match="stopped or rebound"):
        permit.require_active()


def test_stop_during_action_prevents_successful_completion() -> None:
    platform = BlockingClickPlatform(computer_window())
    controller = make_controller(platform)
    screenshot = controller.screenshot(42)
    outcome: list[ComputerActionOutput | ComputerControlError] = []

    def click() -> None:
        try:
            result = execute_computer_operation(
                ComputerClickRequest(
                    window_id=42,
                    observation_id=screenshot.observation_id,
                    x=10,
                    y=10,
                ),
                controller=controller,
            )
            assert isinstance(result, ComputerActionOutput)
            outcome.append(result)
        except ComputerControlError as exc:
            outcome.append(exc)

    worker = threading.Thread(target=click)
    worker.start()
    assert platform.started.wait(timeout=2)
    stopped = threading.Event()

    def stop() -> None:
        controller.stop()
        stopped.set()

    stopper = threading.Thread(target=stop)
    stopper.start()
    assert not stopped.wait(timeout=0.1)
    platform.release.set()
    worker.join(timeout=2)
    stopper.join(timeout=2)

    assert stopped.is_set()
    assert len(outcome) == 1
    assert isinstance(outcome[0], ComputerControlError)


def test_screenshots_are_serialized_across_controllers() -> None:
    first_platform = BlockingScreenshotPlatform(computer_window())
    second_platform = BlockingScreenshotPlatform(computer_window())
    first = make_controller(first_platform)
    second = make_controller(second_platform)
    outcomes: list[ComputerScreenshotOutput] = []

    def capture(controller: ComputerController) -> None:
        result = execute_computer_operation(
            ComputerScreenshotRequest(window_id=42),
            controller=controller,
        )
        assert isinstance(result, ComputerScreenshotOutput)
        outcomes.append(result)

    first_worker = threading.Thread(target=capture, args=(first,))
    second_worker = threading.Thread(target=capture, args=(second,))
    first_worker.start()
    assert first_platform.started.wait(timeout=2)
    second_worker.start()
    assert not second_platform.started.wait(timeout=0.1)
    first_platform.release.set()
    assert second_platform.started.wait(timeout=2)
    second_platform.release.set()
    first_worker.join(timeout=2)
    second_worker.join(timeout=2)

    assert len(outcomes) == 2


def test_controller_stop_closes_every_session_owned_process() -> None:
    platform = FakeComputerPlatform(computer_window())
    controller = make_controller(platform)

    controller.stop()
    controller.stop()

    assert platform.stops == [True]


def test_controller_retries_platform_cleanup_after_a_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    platform = FakeComputerPlatform(computer_window())
    controller = make_controller(platform)
    attempts: list[bool] = []

    def flaky_stop(_: FakeComputerPlatform) -> None:
        attempts.append(True)
        if len(attempts) == 1:
            raise ComputerControlError("temporary cleanup failure")

    monkeypatch.setattr(FakeComputerPlatform, "stop", flaky_stop)

    with pytest.raises(ComputerControlError, match="temporary cleanup failure"):
        controller.stop()
    controller.stop()

    assert attempts == [True, True]
