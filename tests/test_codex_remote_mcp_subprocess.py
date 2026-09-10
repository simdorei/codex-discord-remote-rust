from __future__ import annotations

import os
import sys
from pathlib import Path

from codex_remote_mcp_subprocess import (
    execute_owned_bounded_process,
    run_owned_bounded_process,
)


def test_normal_process_output_is_preserved_exactly(tmp_path: Path) -> None:
    result = run_owned_bounded_process(
        (
            sys.executable,
            "-c",
            "import sys; print('out'); print('err', file=sys.stderr)",
        ),
        cwd=tmp_path,
        env=os.environ.copy(),
        timeout_seconds=5,
        max_stream_bytes=4_096,
    )

    assert result.returncode == 0
    expected_stdout = b"out\r\n" if os.name == "nt" else b"out\n"
    expected_stderr = b"err\r\n" if os.name == "nt" else b"err\n"
    assert result.stdout == expected_stdout
    assert result.stderr == expected_stderr
    assert result.stdout_truncated is False
    assert result.stderr_truncated is False


def test_capture_retention_limit_preserves_natural_success(tmp_path: Path) -> None:
    result = run_owned_bounded_process(
        (sys.executable, "-c", "import sys; sys.stdout.write('a' * 10000)"),
        cwd=tmp_path,
        env=os.environ.copy(),
        timeout_seconds=5,
        max_stream_bytes=4_096,
    )

    assert result.returncode == 0
    assert len(result.stdout) == 4_096
    assert b"...[output truncated]..." in result.stdout
    assert result.stdout_truncated is True
    assert result.stderr == b""


def test_outcome_api_returns_timeout_without_losing_diagnostics(tmp_path: Path) -> None:
    result = execute_owned_bounded_process(
        (
            sys.executable,
            "-c",
            "import time; print('started', flush=True); time.sleep(30)",
        ),
        cwd=tmp_path,
        env=os.environ.copy(),
        timeout_seconds=0.2,
        max_stream_bytes=4_096,
    )

    assert result.process_id > 0
    assert result.exit_code is None
    assert result.stdout in {b"started\n", b"started\r\n"}
    assert result.stdout_bytes == len(result.stdout)
    assert result.timed_out is True
    assert result.cancelled is False
