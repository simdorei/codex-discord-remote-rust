from __future__ import annotations

import pytest

import codex_pro_runtime_preflight as pro_preflight
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    ProRuntimePreflightError,
    preflight_error,
)


class ResidentRefreshTestError(Exception):
    pass


def _preflight_failure(code: ProDiagnosticCode) -> ProRuntimePreflightError:
    return preflight_error(
        stage=ProDiagnosticStage.RESIDENT_APP_SERVER,
        code=code,
        public_message="test failure",
        recovery_action="test recovery",
        internal_detail="test detail",
    )


def test_stale_preflight_restarts_resident_and_retries_once() -> None:
    # Given one stale result followed by a healthy resident snapshot.
    calls: list[str] = []
    expected = pro_preflight.ProRuntimeStatus(
        remote_plugin_version="1.2.3",
        browser_plugin_version="9.8.7",
        resident_generation=8,
    )

    def check() -> pro_preflight.ProRuntimeStatus:
        calls.append("check")
        if calls.count("check") == 1:
            raise _preflight_failure(ProDiagnosticCode.RESIDENT_STALE)
        return expected

    def refresh() -> bool:
        calls.append("refresh")
        return True

    # When stale recovery is attempted.
    result = pro_preflight.recover_stale_pro_runtime(check, refresh)

    # Then the resident is refreshed once before one successful retry.
    assert result == expected
    assert calls == ["check", "refresh", "check"]


def test_stale_preflight_stays_blocked_when_resident_is_busy() -> None:
    # Given a stale result and a resident that cannot safely restart yet.
    calls: list[str] = []
    failure = _preflight_failure(ProDiagnosticCode.RESIDENT_STALE)

    def check() -> pro_preflight.ProRuntimeStatus:
        calls.append("check")
        raise failure

    def refresh() -> bool:
        calls.append("refresh")
        return False

    # When stale recovery is attempted.
    with pytest.raises(ProRuntimePreflightError) as caught:
        pro_preflight.recover_stale_pro_runtime(check, refresh)

    # Then the original diagnostic is preserved without an unsafe retry.
    assert caught.value is failure
    assert calls == ["check", "refresh"]


def test_non_stale_preflight_failure_does_not_restart_resident() -> None:
    # Given a failure that a resident restart cannot repair.
    calls: list[str] = []
    failure = _preflight_failure(ProDiagnosticCode.PLUGIN_INVENTORY_INVALID)

    def check() -> pro_preflight.ProRuntimeStatus:
        calls.append("check")
        raise failure

    def refresh() -> bool:
        calls.append("refresh")
        return True

    # When runtime recovery checks the failure type.
    with pytest.raises(ProRuntimePreflightError) as caught:
        pro_preflight.recover_stale_pro_runtime(check, refresh)

    # Then it surfaces the failure without touching the resident.
    assert caught.value is failure
    assert calls == ["check"]


def test_stale_preflight_retries_only_once() -> None:
    # Given a successful restart followed by a different preflight failure.
    calls: list[str] = []
    stale_failure = _preflight_failure(ProDiagnosticCode.RESIDENT_STALE)
    retry_failure = _preflight_failure(ProDiagnosticCode.RESIDENT_UNHEALTHY)

    def check() -> pro_preflight.ProRuntimeStatus:
        calls.append("check")
        if calls.count("check") == 1:
            raise stale_failure
        raise retry_failure

    def refresh() -> bool:
        calls.append("refresh")
        return True

    # When stale recovery performs its single retry.
    with pytest.raises(ProRuntimePreflightError) as caught:
        pro_preflight.recover_stale_pro_runtime(check, refresh)

    # Then no restart loop is possible.
    assert caught.value is retry_failure
    assert calls == ["check", "refresh", "check"]


def test_refresh_transport_failure_preserves_stale_diagnostic() -> None:
    # Given a stale preflight and an unexpected resident refresh failure.
    calls: list[str] = []
    failure = _preflight_failure(ProDiagnosticCode.RESIDENT_STALE)

    def check() -> pro_preflight.ProRuntimeStatus:
        calls.append("check")
        raise failure

    def refresh() -> bool:
        calls.append("refresh")
        raise ResidentRefreshTestError("resident refresh failed")

    # When automatic recovery cannot restart the resident.
    with pytest.raises(ProRuntimePreflightError) as caught:
        pro_preflight.recover_stale_pro_runtime(check, refresh)

    # Then the stale guidance remains while the internal cause is retained.
    assert caught.value.diagnostic.stage is failure.diagnostic.stage
    assert caught.value.diagnostic.code is failure.diagnostic.code
    assert caught.value.diagnostic.public_message == failure.diagnostic.public_message
    assert caught.value.diagnostic.recovery_action == failure.diagnostic.recovery_action
    assert caught.value.diagnostic.internal_detail == (
        "test detail; automatic resident refresh failed "
        "error_type=ResidentRefreshTestError error=resident refresh failed"
    )
    assert caught.value.__cause__ is None
    assert calls == ["check", "refresh"]
