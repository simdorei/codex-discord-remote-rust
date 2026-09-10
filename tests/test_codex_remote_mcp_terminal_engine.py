from __future__ import annotations

import os
import shlex
import sys
import threading
import time
from pathlib import Path

import pytest

from codex_remote_mcp_terminal_engine import (
    TerminalExecutionEngine,
    TerminalExecutionError,
)
from simdorei_mcp_common.terminal_protocol import TerminalExecOutput, TerminalExecRequest


def _python(code: str) -> str:
    if os.name != "nt":
        return shlex.join([sys.executable, "-c", code])
    executable = sys.executable.replace("'", "''")
    powershell_code = code.replace("'", "''")
    return f"& '{executable}' -c '{powershell_code}'"


def _wait_for(path: Path) -> None:
    deadline = time.monotonic() + 5
    while not path.exists() and time.monotonic() < deadline:
        time.sleep(0.02)
    assert path.exists()


def test_terminal_reuses_external_cwd_and_environment_with_secret_safe_receipt(
    tmp_path: Path,
) -> None:
    root = tmp_path / "project"
    external = tmp_path / "external"
    root.mkdir()
    external.mkdir()
    engine = TerminalExecutionEngine(root, session_id="session-a")

    first = engine.execute(
        TerminalExecRequest(
            command=_python(
                "import os; print(os.getcwd()); print(os.environ['TERMINAL_QA'])"
            ),
            cwd=str(external),
            environment={"TERMINAL_QA": "persisted"},
        )
    )
    second = engine.execute(
        TerminalExecRequest(
            terminal_id=first.terminal_id,
            command=_python("import os; print(os.environ['TERMINAL_QA'])"),
        )
    )

    assert str(external) in first.stdout
    assert "persisted" in first.stdout
    assert first.receipt.cwd_scope == "external_absolute"
    assert second.cwd == str(external)
    assert "persisted" in second.stdout
    assert first.receipt.command_digest not in first.stdout
    assert "command" not in first.receipt.model_dump()


def test_terminal_ids_are_owned_by_one_engine_session(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    owner = TerminalExecutionEngine(root, session_id="owner")
    stranger = TerminalExecutionEngine(root, session_id="stranger")
    created = owner.execute(TerminalExecRequest(command="echo owned"))

    with pytest.raises(TerminalExecutionError, match="does not belong"):
        _ = stranger.execute(
            TerminalExecRequest(
                terminal_id=created.terminal_id,
                command="echo escaped",
            )
        )


def test_terminal_timeout_returns_a_receipted_outcome(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    engine = TerminalExecutionEngine(root, session_id="timeout")

    result = engine.execute(
        TerminalExecRequest(
            command=_python("import time; print('started', flush=True); time.sleep(30)"),
            timeout_seconds=1,
        )
    )

    assert result.exit_code is None
    assert result.timed_out is True
    assert result.cancelled is False
    assert "started" in result.stdout
    assert result.receipt.timed_out is True
    assert result.process_id > 0


def test_pre_cancelled_request_never_creates_a_terminal(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    engine = TerminalExecutionEngine(root, session_id="pre-cancel")
    cancelled = threading.Event()
    cancelled.set()

    with pytest.raises(TerminalExecutionError, match="cancelled"):
        _ = engine.execute(
            TerminalExecRequest(command="echo no"),
            cancel_event=cancelled,
        )

    assert engine.list_terminal_ids() == ()


def test_cancel_previous_replaces_only_the_same_terminal(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    engine = TerminalExecutionEngine(root, session_id="replace")
    seeded = engine.execute(TerminalExecRequest(command="echo seeded"))
    marker = tmp_path / "long-started"
    outcomes: list[TerminalExecOutput] = []

    def run_long() -> None:
        outcomes.append(
            engine.execute(
                TerminalExecRequest(
                    terminal_id=seeded.terminal_id,
                    command=_python(
                        "from pathlib import Path; import time; "
                        + f"Path({str(marker)!r}).write_text('x'); time.sleep(30)"
                    ),
                    timeout_seconds=60,
                )
            )
        )

    worker = threading.Thread(target=run_long)
    worker.start()
    _wait_for(marker)
    with pytest.raises(TerminalExecutionError, match="active command"):
        _ = engine.execute(
            TerminalExecRequest(
                terminal_id=seeded.terminal_id,
                command="echo must-not-run",
            )
        )

    replacement = engine.execute(
        TerminalExecRequest(
            terminal_id=seeded.terminal_id,
            command="echo replacement",
            cancel_previous=True,
        )
    )
    worker.join(timeout=10)

    assert not worker.is_alive()
    assert len(outcomes) == 1
    assert outcomes[0].cancelled is True
    assert outcomes[0].timed_out is False
    assert replacement.exit_code == 0
    assert "replacement" in replacement.stdout


def test_close_cancels_active_work_and_rejects_reuse(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    engine = TerminalExecutionEngine(root, session_id="close")
    seeded = engine.execute(TerminalExecRequest(command="echo seeded"))
    marker = tmp_path / "close-started"
    outcomes: list[TerminalExecOutput] = []

    worker = threading.Thread(
        target=lambda: outcomes.append(
            engine.execute(
                TerminalExecRequest(
                    terminal_id=seeded.terminal_id,
                    command=_python(
                        "from pathlib import Path; import time; "
                        + f"Path({str(marker)!r}).write_text('x'); time.sleep(30)"
                    ),
                    timeout_seconds=60,
                )
            )
        )
    )
    worker.start()
    _wait_for(marker)

    engine.close()
    worker.join(timeout=10)

    assert not worker.is_alive()
    assert outcomes[0].cancelled is True
    assert engine.list_terminal_ids() == ()
    with pytest.raises(TerminalExecutionError, match="closed"):
        _ = engine.execute(TerminalExecRequest(command="echo no"))


def test_terminal_strips_inherited_secrets_and_redacts_output(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "project"
    root.mkdir()
    monkeypatch.setenv("TERMINAL_QA_SECRET_TOKEN", "NeverExpose1234567890")
    engine = TerminalExecutionEngine(root, session_id="redaction")

    result = engine.execute(
        TerminalExecRequest(
            command=_python(
                "import os; print(os.environ.get('TERMINAL_QA_SECRET_TOKEN', 'missing')); "
                + "print('api_key=Abcd1234Efgh5678Ijkl')"
            )
        )
    )

    assert "NeverExpose" not in result.stdout
    assert "missing" in result.stdout
    assert "api_key=[REDACTED]" in result.stdout
