from __future__ import annotations

import unittest
from pathlib import Path

import codex_desktop_bridge_prompt_delivery as prompt_delivery
from codex_bridge_state import JsonObject
from codex_pro_prompt_contract import CHROME_PLUGIN_URI, PRO_SKILL_NAME
from codex_thread_models import ThreadInfo


class PromptDeliveryTests(unittest.TestCase):
    def test_pro_delivery_matches_localized_browser_label_by_plugin_uri(self) -> None:
        thread = ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:/repo",
            updated_at=1,
            rollout_path="C:/repo/session.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )
        expected = f"${PRO_SKILL_NAME} [@Chrome]({CHROME_PLUGIN_URI}) review"
        recorded = f"${PRO_SKILL_NAME} [@크롬]({CHROME_PLUGIN_URI}) review"
        event: JsonObject = {
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": recorded}],
            },
        }
        now = [0.0]

        def read_events(_path: Path, cursor: int) -> tuple[list[JsonObject], int]:
            return ([event] if cursor == 0 else []), cursor + 1

        def extract_text(payload: JsonObject) -> str:
            content = payload.get("content")
            if not isinstance(content, list) or not content or not isinstance(content[0], dict):
                return ""
            text = content[0].get("text")
            return text if isinstance(text, str) else ""

        result = prompt_delivery.wait_for_prompt_delivery(
            {thread.id: (thread, Path(thread.rollout_path), 0)},
            expected,
            timeout_sec=0.5,
            read_new_session_events=read_events,
            extract_message_text=extract_text,
            time_now=lambda: now[0],
            sleep=lambda seconds: now.__setitem__(0, now[0] + seconds),
        )

        self.assertIs(result, thread)


if __name__ == "__main__":
    unittest.main()
