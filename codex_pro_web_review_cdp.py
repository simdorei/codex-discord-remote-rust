from __future__ import annotations

from dataclasses import dataclass
import time

from codex_pro_web_review import (
    DEFAULT_CDP_URL,
    ProWebReviewBrowserError,
    ProWebReviewError,
    ProWebReviewLoginRequiredError,
    ProWebReviewPack,
    cdp_login_help,
)
from codex_pro_web_review_login import looks_logged_in, pick_context


@dataclass(frozen=True, slots=True)
class CdpChatGptReviewer:
    cdp_url: str = DEFAULT_CDP_URL
    chatgpt_url: str = "https://chatgpt.com/"
    wait_seconds: int = 1200
    stable_seconds: int = 8

    def submit_review(self, pack: ProWebReviewPack) -> str:
        try:
            from playwright.sync_api import Error as PlaywrightError
            from playwright.sync_api import sync_playwright
        except ImportError as exc:
            raise ProWebReviewBrowserError(
                "Playwright is not installed. Install it before using the CDP ChatGPT runner."
            ) from exc

        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.connect_over_cdp(self.cdp_url)
                context = pick_context(browser)
                if context is None:
                    raise ProWebReviewLoginRequiredError(cdp_login_help(self.cdp_url))
                page = context.new_page()
                try:
                    page.goto(self.chatgpt_url, wait_until="load", timeout=60_000)
                    if not looks_logged_in(page):
                        raise ProWebReviewLoginRequiredError(cdp_login_help(self.cdp_url))
                    prompt_box = _wait_for_prompt_box(page, self.cdp_url)
                    base_assistant_count = page.locator('[data-message-author-role="assistant"]').count()
                    _attach_pack_file(page, pack.pack_path)
                    prompt_box.fill(pack.prompt_text)
                    _click_send(page)
                    return _wait_for_new_assistant_text(
                        page,
                        base_assistant_count=base_assistant_count,
                        wait_seconds=self.wait_seconds,
                        stable_seconds=self.stable_seconds,
                    )
                finally:
                    page.close()
        except ProWebReviewError:
            raise
        except PlaywrightError as exc:
            raise ProWebReviewBrowserError(f"ChatGPT CDP review failed: {exc}") from exc


def _wait_for_prompt_box(page, cdp_url: str):
    deadline = time.monotonic() + 30
    selectors = ("#prompt-textarea", 'textarea[data-testid="prompt-textarea"]', 'div[contenteditable="true"]')
    while time.monotonic() < deadline:
        for selector in selectors:
            locator = page.locator(selector).first
            if locator.count() > 0:
                return locator
        page.wait_for_timeout(500)
    raise ProWebReviewLoginRequiredError(cdp_login_help(cdp_url))


def _attach_pack_file(page, pack_path) -> None:
    file_inputs = page.locator('input[type="file"]')
    if file_inputs.count() == 0:
        raise ProWebReviewBrowserError("ChatGPT file upload input was not found.")
    file_inputs.first.set_input_files(str(pack_path))
    page.wait_for_timeout(2_000)


def _click_send(page) -> None:
    selectors = ('button[data-testid="send-button"]', 'button[aria-label*="Send"]', 'button[aria-label*="send"]')
    for selector in selectors:
        button = page.locator(selector).first
        if button.count() > 0:
            button.click()
            return
    raise ProWebReviewBrowserError("ChatGPT send button was not found.")


def _wait_for_new_assistant_text(page, *, base_assistant_count: int, wait_seconds: int, stable_seconds: int) -> str:
    deadline = time.monotonic() + wait_seconds
    last_text = ""
    stable_since = time.monotonic()
    while time.monotonic() < deadline:
        if page.locator('button[data-testid="stop-button"]').count() > 0:
            stable_since = time.monotonic()
            page.wait_for_timeout(1_000)
            continue
        assistant = page.locator('[data-message-author-role="assistant"]')
        if assistant.count() <= base_assistant_count:
            page.wait_for_timeout(1_000)
            continue
        text = assistant.last.inner_text().strip()
        if text != last_text:
            last_text = text
            stable_since = time.monotonic()
        elif text and time.monotonic() - stable_since >= stable_seconds:
            return text
        page.wait_for_timeout(1_000)
    raise ProWebReviewBrowserError(f"Timed out waiting for ChatGPT response after {wait_seconds}s.")
