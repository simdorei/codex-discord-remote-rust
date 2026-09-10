from __future__ import annotations

from dataclasses import replace

import pytest

import codex_remote_mcp_windows_windows as windows_windows
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_native import USER32
from codex_remote_mcp_windows_windows import ResolvedWindow
from tests.remote_mcp_computer_fakes import computer_identity, computer_window


def _resolved() -> ResolvedWindow:
    window = computer_window()
    identity = replace(
        computer_identity(window),
        surface_window_id=84,
        surface_digest="a" * 64,
        surface_left=20,
        surface_top=50,
        surface_width=760,
        surface_height=520,
    )
    return ResolvedWindow(entry=window, identity=identity)


def test_post_mutation_allows_only_title_and_content_digest_changes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    expected = _resolved()
    mutated = ResolvedWindow(
        entry=expected.entry,
        identity=replace(
            expected.identity,
            title_digest="b" * 64,
            surface_digest="c" * 64,
        ),
    )
    monkeypatch.setattr(windows_windows, "resolve_allowed_window", lambda _: mutated)
    monkeypatch.setattr(
        USER32,
        "GetForegroundWindow",
        lambda: expected.identity.window_id,
    )

    current = windows_windows.require_same_active_window_after_mutation(
        expected.identity
    )

    assert current == mutated


@pytest.mark.parametrize(
    "changed_identity",
    (
        {"process_id": 999},
        {"width": 799},
        {"surface_window_id": 999},
        {"surface_left": 21},
    ),
)
def test_post_mutation_rejects_structural_identity_changes(
    monkeypatch: pytest.MonkeyPatch,
    changed_identity: dict[str, int],
) -> None:
    expected = _resolved()
    changed = ResolvedWindow(
        entry=expected.entry,
        identity=replace(expected.identity, **changed_identity),
    )
    monkeypatch.setattr(windows_windows, "resolve_allowed_window", lambda _: changed)

    with pytest.raises(ComputerControlError, match="window changed"):
        _ = windows_windows.require_same_active_window_after_mutation(expected.identity)


def test_post_mutation_rejects_a_foreground_change(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    expected = _resolved()
    monkeypatch.setattr(
        windows_windows,
        "resolve_allowed_window",
        lambda _: expected,
    )
    monkeypatch.setattr(
        USER32,
        "GetForegroundWindow",
        lambda: 999,
    )

    with pytest.raises(ComputerControlError, match="active window changed"):
        _ = windows_windows.require_same_active_window_after_mutation(expected.identity)
