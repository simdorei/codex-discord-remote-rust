from __future__ import annotations

import unittest

import codex_desktop_bridge_prompt_sender as prompt_sender
from codex_thread_models import WindowInfo


class PromptSenderTests(unittest.TestCase):
    def test_clipboard_mismatch_raises_typed_error_after_retry(self) -> None:
        events: list[str] = []
        window = WindowInfo(hwnd=1, title="Codex", left=0, top=0, right=100, bottom=100)
        clipboard_reads = ["wrong", "still wrong"]

        def record_hotkey(*keys: int) -> None:
            events.append("hotkey:" + "+".join(str(key) for key in keys))

        def get_clipboard_text() -> str | None:
            return clipboard_reads.pop(0)

        deps = prompt_sender.PromptSenderDeps(
            find_codex_window=lambda: window,
            focus_window=lambda focused_window: events.append(f"focus:{focused_window.hwnd}"),
            ensure_codex_composer_focus=lambda: True,
            click_window=lambda click_window, x_ratio, y_offset: (click_window.left, y_offset),
            send_hotkey=record_hotkey,
            send_key_event=lambda vk, keyup: events.append(f"key:{vk}:{keyup}"),
            set_clipboard_text=lambda text: events.append(f"clipboard:{text}"),
            get_clipboard_text=get_clipboard_text,
            sleep=lambda seconds: events.append(f"sleep:{seconds}"),
            print_line=events.append,
            vk_control=17,
            vk_a=65,
            vk_back=8,
            vk_v=86,
            vk_return=13,
        )

        with self.assertRaises(prompt_sender.PromptClipboardMismatchError) as raised:
            _ = prompt_sender.send_prompt_to_codex(
                "hello",
                click_x_ratio=0.5,
                click_y_offset=10,
                skip_click=True,
                deps=deps,
            )

        self.assertEqual(str(raised.exception), "Clipboard did not contain the prompt after setting it.")
        self.assertIn("sleep:0.05", events)
        self.assertNotIn("hotkey:17+86", events)
