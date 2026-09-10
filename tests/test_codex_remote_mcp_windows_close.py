from __future__ import annotations

import pytest

import codex_remote_mcp_windows_close as windows_close
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_platform_lifecycle import OwnedLaunch, OwnedWindow
from codex_remote_mcp_windows_windows import ResolvedWindow
from tests.remote_mcp_computer_fakes import FakeOwnedProcess, computer_identity
from tests.remote_mcp_computer_fakes import computer_window as fake_window


def _owned_launch(process: FakeOwnedProcess) -> OwnedLaunch:
    window = fake_window()
    owner = OwnedWindow(
        process_id=process.pid,
        process_path=r"c:\windows\system32\notepad.exe",
        process_name="notepad.exe",
        process=process,
    )
    return OwnedLaunch(
        window_id=window.window_id,
        owner=owner,
        process=process,
        temporary_profile=None,
    )


def _matching_window(identity: ComputerWindowIdentity) -> ResolvedWindow:
    return ResolvedWindow(entry=fake_window(), identity=identity)


def test_close_validates_the_visible_window_then_stops_the_retained_process(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = FakeOwnedProcess(process_id=1234)
    identity = computer_identity(fake_window())
    monkeypatch.setattr(
        windows_close,
        "require_matching_active_window",
        _matching_window,
    )

    windows_close.close_owned_window(identity, _owned_launch(process))

    assert process.exit_code == 0


def test_close_does_nothing_when_the_retained_process_already_exited(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = FakeOwnedProcess(process_id=1234)
    process.terminate()
    monkeypatch.setattr(
        windows_close,
        "require_matching_active_window",
        lambda _: pytest.fail("an exited process needs no window validation"),
    )

    windows_close.close_owned_window(
        computer_identity(fake_window()),
        _owned_launch(process),
    )


def test_close_rejects_a_changed_window_without_stopping_the_process(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = FakeOwnedProcess(process_id=1234)

    def reject_changed_window(_: ComputerWindowIdentity) -> ResolvedWindow:
        raise ComputerControlError("The window changed after the screenshot.")

    monkeypatch.setattr(
        windows_close,
        "require_matching_active_window",
        reject_changed_window,
    )

    with pytest.raises(ComputerControlError, match="window changed"):
        windows_close.close_owned_window(
            computer_identity(fake_window()),
            _owned_launch(process),
        )

    assert process.exit_code is None


def test_close_rechecks_process_identity_after_window_validation(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = FakeOwnedProcess(process_id=1234)
    launch = _owned_launch(process)

    def replace_process_identity(
        identity: ComputerWindowIdentity,
    ) -> ResolvedWindow:
        process.replace_process_id_for_test(9999)
        return _matching_window(identity)

    monkeypatch.setattr(
        windows_close,
        "require_matching_active_window",
        replace_process_identity,
    )

    with pytest.raises(ComputerControlError, match="process identity changed"):
        windows_close.close_owned_window(computer_identity(fake_window()), launch)

    assert process.exit_code is None
