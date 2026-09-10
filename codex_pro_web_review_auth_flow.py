from __future__ import annotations
from collections.abc import Callable, Sequence
import argparse
from dataclasses import dataclass
import time
from codex_pro_web_review_login import BrowserName, ProWebReviewEnvReport, build_env_report, ensure_browser, format_env_report
@dataclass(frozen=True, slots=True)
class AuthFlowOptions:
    browser: BrowserName = "chrome"
    wait_seconds: int = 300
    poll_seconds: int = 5
@dataclass(frozen=True, slots=True)
class AuthFlowDeps:
    ensure_browser: Callable[[BrowserName, Callable[[str], None]], bool]
    build_report: Callable[[], ProWebReviewEnvReport]
    monotonic: Callable[[], float]
    sleep: Callable[[float], None]
def run_auth_flow(options: AuthFlowOptions, deps: AuthFlowDeps, emit: Callable[[str], None] = print) -> int:
    if not deps.ensure_browser(options.browser, emit):
        return 2
    deadline = deps.monotonic() + options.wait_seconds
    while True:
        report = deps.build_report()
        emit(format_env_report(report))
        if report.login == "ok":
            emit("ChatGPT login confirmed. Pro web review can use this browser session.")
            return 0
        if deps.monotonic() >= deadline:
            emit("Timed out waiting for ChatGPT login. Keep the CDP Chrome open and rerun this command.")
            return 1
        emit("Sign in on the opened ChatGPT page, then leave this command running.")
        deps.sleep(options.poll_seconds)
def main(argv: Sequence[str] | None = None) -> int:
    options = _parse_args(argv)
    deps = AuthFlowDeps(
        ensure_browser=_ensure_browser,
        build_report=build_env_report,
        monotonic=time.monotonic,
        sleep=time.sleep,
    )
    return run_auth_flow(options, deps)
def _parse_args(argv: Sequence[str] | None) -> AuthFlowOptions:
    parser = argparse.ArgumentParser(description="Open CDP Chrome and wait for a manual ChatGPT login.")
    parser.add_argument("--browser", choices=("chrome", "comet"), default="chrome")
    parser.add_argument("--wait-seconds", type=int, default=300)
    parser.add_argument("--poll-seconds", type=int, default=5)
    parsed = parser.parse_args(argv)
    browser: BrowserName = "comet" if parsed.browser == "comet" else "chrome"
    return AuthFlowOptions(
        browser=browser,
        wait_seconds=max(0, parsed.wait_seconds),
        poll_seconds=max(1, parsed.poll_seconds),
    )
def _ensure_browser(browser: BrowserName, emit: Callable[[str], None]) -> bool:
    return ensure_browser(browser, emit=emit)
if __name__ == "__main__":
    raise SystemExit(main())
