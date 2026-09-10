from __future__ import annotations
from collections.abc import Callable
import unittest
import codex_pro_web_review_auth_flow as auth_flow
from codex_pro_web_review_login import (
    BrowserName,
    EnvIssue,
    LoginState,
    ProWebReviewEnvReport,
)
class ProWebReviewAuthFlowTests(unittest.TestCase):
    def test_run_auth_flow_returns_zero_when_login_ok(self) -> None:
        out: list[str] = []
        result = auth_flow.run_auth_flow(
            auth_flow.AuthFlowOptions(wait_seconds=0),
            _deps(_report("ok")),
            out.append,
        )
        self.assertEqual(result, 0)
        self.assertTrue(any("login confirmed" in line for line in out))
    def test_run_auth_flow_times_out_when_login_missing(self) -> None:
        out: list[str] = []
        result = auth_flow.run_auth_flow(
            auth_flow.AuthFlowOptions(wait_seconds=0),
            _deps(_report("no")),
            out.append,
        )
        self.assertEqual(result, 1)
        self.assertTrue(any("Timed out" in line for line in out))
    def test_run_auth_flow_returns_two_when_browser_cannot_open(self) -> None:
        def fail_browser(_browser: BrowserName, _emit: Callable[[str], None]) -> bool:
            return False
        result = auth_flow.run_auth_flow(
            auth_flow.AuthFlowOptions(wait_seconds=0),
            auth_flow.AuthFlowDeps(
                ensure_browser=fail_browser,
                build_report=lambda: _report("unknown"),
                monotonic=lambda: 0.0,
                sleep=lambda _seconds: None,
            ),
        )
        self.assertEqual(result, 2)
    def test_parse_args_keeps_wait_non_negative(self) -> None:
        options = auth_flow._parse_args(["--browser", "comet", "--wait-seconds", "-1", "--poll-seconds", "0"])
        self.assertEqual(options.browser, "comet")
        self.assertEqual(options.wait_seconds, 0)
        self.assertEqual(options.poll_seconds, 1)
def _deps(report: ProWebReviewEnvReport) -> auth_flow.AuthFlowDeps:
    return auth_flow.AuthFlowDeps(
        ensure_browser=lambda _browser, _emit: True,
        build_report=lambda: report,
        monotonic=lambda: 0.0,
        sleep=lambda _seconds: None,
    )
def _report(login: LoginState) -> ProWebReviewEnvReport:
    issues = () if login == "ok" else (EnvIssue("ChatGPT login missing", "Sign in."),)
    return ProWebReviewEnvReport(deps="ok", browser="ok", login=login, ok=(), issues=issues)
if __name__ == "__main__":
    _ = unittest.main()
