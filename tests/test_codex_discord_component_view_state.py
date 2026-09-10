from __future__ import annotations

import unittest

import codex_discord_component_view_state as component_view_state


class FakeButton:
    def __init__(self, label: str) -> None:
        self.label: str = label
        self.custom_id: str = ""
        self.disabled: bool = False


class FakeNonButton:
    def __init__(self, label: str) -> None:
        self.label: str = label
        self.custom_id: str = ""
        self.disabled: bool = False


def is_fake_button(item: component_view_state.ComponentViewChild) -> bool:
    return isinstance(item, FakeButton)


class ComponentViewStateTests(unittest.TestCase):
    def test_assign_approval_button_custom_ids_by_label(self) -> None:
        children: list[component_view_state.ComponentViewChild] = [
            FakeButton("Approve"),
            FakeButton("Approve session"),
            FakeButton("Reject"),
            FakeButton("Cancel"),
        ]

        component_view_state.assign_approval_button_custom_ids(
            children,
            "thread-1",
            is_button=is_fake_button,
        )

        self.assertEqual(getattr(children[0], "custom_id"), "codex_approval:thread-1:1")
        self.assertEqual(getattr(children[1], "custom_id"), "codex_approval:thread-1:2")
        self.assertEqual(getattr(children[2], "custom_id"), "codex_approval:thread-1:3")
        self.assertEqual(getattr(children[3], "custom_id"), "codex_approval:thread-1:cancel")

    def test_configure_busy_choice_buttons_assigns_ids_and_disables_steer(self) -> None:
        children: list[component_view_state.ComponentViewChild] = [
            FakeButton("Steer now"),
            FakeButton("Queue next"),
            FakeButton("Ignore"),
        ]

        component_view_state.configure_busy_choice_buttons(
            children,
            "0123456789abcdef01234567",
            allow_steer=False,
            is_button=is_fake_button,
        )

        self.assertEqual(getattr(children[0], "custom_id"), "codex_busy:0123456789abcdef01234567:steer")
        self.assertEqual(getattr(children[1], "custom_id"), "codex_busy:0123456789abcdef01234567:queue")
        self.assertEqual(getattr(children[2], "custom_id"), "codex_busy:0123456789abcdef01234567:ignore")
        self.assertTrue(getattr(children[0], "disabled"))
        self.assertFalse(getattr(children[1], "disabled"))
        self.assertFalse(getattr(children[2], "disabled"))

    def test_busy_choice_assignment_ignores_non_buttons_and_missing_choice_id(self) -> None:
        button = FakeButton("Queue next")
        non_button = FakeNonButton("Queue next")
        children: list[component_view_state.ComponentViewChild] = [button, non_button]

        component_view_state.configure_busy_choice_buttons(
            children,
            None,
            allow_steer=True,
            is_button=is_fake_button,
        )

        self.assertEqual(button.custom_id, "")
        self.assertEqual(non_button.custom_id, "")

    def test_disable_busy_choice_steer_button_only_changes_steer_button(self) -> None:
        steer = FakeButton("Steer now")
        queue = FakeButton("Queue next")
        non_button = FakeNonButton("Steer now")

        component_view_state.disable_busy_choice_steer_button(
            [steer, queue, non_button],
            is_button=is_fake_button,
        )

        self.assertTrue(steer.disabled)
        self.assertFalse(queue.disabled)
        self.assertFalse(non_button.disabled)
