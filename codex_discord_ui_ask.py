from __future__ import annotations


def should_retry_ask_with_ui(exit_code: int, output: str) -> bool:
    if exit_code == 0:
        return False
    text = (output or "").lower()
    return (
        "local sidecar could not attach" in text
        or "ipc owner client for the selected thread was not discovered" in text
        or "winerror 2" in text
        or "winerror 5" in text
    )


def build_ui_ask_argv(
    prompt: str,
    *,
    target_thread_id: str | None,
    force_while_busy: bool,
    wait: bool,
    timeout_sec: float | None = None,
) -> list[str]:
    timeout_value = "0" if timeout_sec is None else str(max(1, int(timeout_sec)))
    argv = [
        "ask",
        "--ui",
        "--switch-thread",
        "--foreground",
        "--timeout",
        timeout_value,
    ]
    if target_thread_id:
        argv.extend(["--thread-id", target_thread_id])
    if force_while_busy:
        argv.append("--force-while-busy")
    if not wait:
        argv.append("--no-wait")
    argv.append(prompt)
    return argv
