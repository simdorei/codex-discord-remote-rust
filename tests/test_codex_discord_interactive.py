from __future__ import annotations

import unittest

import codex_discord_interactive as interactive

STATE_NONE = ""
STATE_INPUT = "waiting-input"
STATE_APPROVAL = "waiting-approval"


class InteractiveTests(unittest.TestCase):
    def test_infer_interactive_state_from_error_detects_approval(self) -> None:
        self.assertEqual(
            interactive.infer_interactive_state_from_error(
                "Codex is waiting for approval.",
                state_none=STATE_NONE,
                state_input=STATE_INPUT,
                state_approval=STATE_APPROVAL,
            ),
            STATE_APPROVAL,
        )

    def test_infer_interactive_state_from_error_detects_input(self) -> None:
        self.assertEqual(
            interactive.infer_interactive_state_from_error(
                "Thread is waiting on user input.",
                state_none=STATE_NONE,
                state_input=STATE_INPUT,
                state_approval=STATE_APPROVAL,
            ),
            STATE_INPUT,
        )

    def test_infer_interactive_state_from_error_uses_none_for_other_errors(self) -> None:
        self.assertEqual(
            interactive.infer_interactive_state_from_error(
                "unrelated failure",
                state_none=STATE_NONE,
                state_input=STATE_INPUT,
                state_approval=STATE_APPROVAL,
            ),
            STATE_NONE,
        )


if __name__ == "__main__":
    _ = unittest.main()
