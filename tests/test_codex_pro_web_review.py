from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import codex_pro_web_review as pro_review
from codex_pro_web_review_cdp import CdpChatGptReviewer


class ProWebReviewTests(unittest.TestCase):
    def test_pack_review_request_writes_numbered_text_pack(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "bot.py"
            source.write_text("print('hello')\n", encoding="utf-8")

            pack = pro_review.pack_pro_web_review(
                pro_review.ProWebReviewRequest(
                    root=root,
                    target="bot.py",
                    prompt="Review this file.",
                ),
                now=_fixed_now,
            )

            self.assertEqual(pack.target, "bot.py")
            self.assertEqual(pack.packed_files, (source.resolve(),))
            body = pack.pack_path.read_text(encoding="utf-8")
            self.assertIn("## File: bot.py", body)
            self.assertIn("1\tprint('hello')", body)
            self.assertIn("Use the attached code pack", pack.prompt_text)

    def test_missing_target_raises_typed_error(self) -> None:
        with TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(pro_review.ProWebReviewTargetError, "does not exist"):
                _ = pro_review.pack_pro_web_review(
                    pro_review.ProWebReviewRequest(
                        root=Path(tmp),
                        target="missing.py",
                        prompt="Review it.",
                    ),
                    now=_fixed_now,
                )

    def test_run_review_saves_response_from_reviewer(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "bot.py"
            source.write_text("print('hello')\n", encoding="utf-8")

            result = pro_review.run_pro_web_review(
                pro_review.ProWebReviewRequest(
                    root=root,
                    target="bot.py",
                    prompt="Find bugs.",
                ),
                reviewer=_FakeReviewer("No findings."),
                now=_fixed_now,
            )

            response = result.response_path.read_text(encoding="utf-8")
            self.assertIn("- model: GPT-5.5 Pro", response)
            self.assertIn("No findings.", response)
            self.assertEqual(result.pack.packed_files, (source.resolve(),))

    def test_empty_review_response_fails_closed(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bot.py").write_text("print('hello')\n", encoding="utf-8")

            with self.assertRaises(pro_review.ProWebReviewEmptyResponseError):
                _ = pro_review.run_pro_web_review(
                    pro_review.ProWebReviewRequest(root=root, target="bot.py", prompt="Review."),
                    reviewer=_FakeReviewer("   "),
                    now=_fixed_now,
                )

    def test_cdp_login_help_explains_separate_login(self) -> None:
        help_text = pro_review.cdp_login_help()

        self.assertIn("Codex login and ChatGPT browser login are separate", help_text)
        self.assertIn("chatgpt.com", help_text)

    def test_cdp_reviewer_defaults_to_local_debug_port(self) -> None:
        reviewer = CdpChatGptReviewer()

        self.assertEqual(reviewer.cdp_url, "http://127.0.0.1:9222")


class _FakeReviewer:
    def __init__(self, response: str) -> None:
        self.response = response

    def submit_review(self, pack: pro_review.ProWebReviewPack) -> str:
        _ = pack
        return self.response


def _fixed_now() -> datetime:
    return datetime(2026, 6, 22, 12, 0, 0, tzinfo=timezone.utc)


if __name__ == "__main__":
    _ = unittest.main()
