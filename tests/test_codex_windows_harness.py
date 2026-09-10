import json

import pytest

from codex_windows_harness import print_json
from codex_windows_harness_types import HarnessRuntime


def test_print_json_serializes_dataclass(
    capsys: pytest.CaptureFixture[str],
) -> None:
    # Given
    runtime = HarnessRuntime(
        version="1",
        platform="windows-local",
        codex_cli_path="codex.exe",
        codex_cli_status="available",
        codex_desktop_status="running",
    )

    # When
    print_json(runtime)

    # Then
    assert json.loads(capsys.readouterr().out) == {
        "version": "1",
        "platform": "windows-local",
        "codex_cli_path": "codex.exe",
        "codex_cli_status": "available",
        "codex_desktop_status": "running",
    }
