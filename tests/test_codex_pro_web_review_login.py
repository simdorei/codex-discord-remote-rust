from __future__ import annotations

import unittest

import codex_pro_web_review_login as login


class ProWebReviewLoginTests(unittest.TestCase):
    def test_pick_context_prefers_secure_auth_cookie(self) -> None:
        anonymous = _FakeContext([{"name": "oai-sc"}])
        authenticated = _FakeContext([{"name": "__Secure-next-auth.session-token"}])
        browser = _FakeBrowser([anonymous, authenticated])

        self.assertIs(login.pick_context(browser), authenticated)

    def test_pick_context_falls_back_to_chatgpt_cookie(self) -> None:
        empty = _FakeContext([])
        chatgpt = _FakeContext([{"name": "oai-sc"}])
        browser = _FakeBrowser([empty, chatgpt])

        self.assertIs(login.pick_context(browser), chatgpt)

    def test_looks_logged_in_requires_composer_signal(self) -> None:
        page = _FakePage({"#prompt-textarea", login.FILE_INPUT_SELECTOR})

        self.assertTrue(login.looks_logged_in(page))

    def test_looks_logged_in_rejects_login_wall(self) -> None:
        page = _FakePage({"#prompt-textarea", login.FILE_INPUT_SELECTOR, 'button[data-testid="login-button"]'})

        self.assertFalse(login.looks_logged_in(page))

    def test_browser_info_detector_matches_chrome_family(self) -> None:
        self.assertTrue(login.browser_info_is_cdp_browser({"Browser": "Chrome/126.0"}))
        self.assertFalse(login.browser_info_is_cdp_browser({"Browser": "Firefox/126.0"}))

    def test_format_env_report_includes_machine_status_line(self) -> None:
        report = login.ProWebReviewEnvReport(
            deps="ok",
            browser="down",
            login="unknown",
            ok=("python playwright installed.",),
            issues=(login.EnvIssue("browser CDP port 9222 is closed", "Start Chrome."),),
        )

        text = login.format_env_report(report)

        self.assertIn("STATUS deps=ok browser=down login=unknown", text)
        self.assertIn("browser CDP port 9222 is closed", text)


class _FakeBrowser:
    def __init__(self, contexts: list["_FakeContext"]) -> None:
        self.contexts = contexts


class _FakeContext:
    def __init__(self, cookies: list[dict[str, str]]) -> None:
        self._cookies = cookies

    def cookies(self, url: str) -> list[dict[str, str]]:
        _ = url
        return self._cookies


class _FakePage:
    def __init__(self, selectors: set[str]) -> None:
        self.selectors = selectors

    def query_selector(self, selector: str) -> str | None:
        return "node" if selector in self.selectors else None


if __name__ == "__main__":
    _ = unittest.main()
