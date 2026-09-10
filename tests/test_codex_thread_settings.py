import unittest

import codex_thread_settings as settings


class ThreadSettingsTests(unittest.TestCase):
    def test_saved_settings_round_trip_filters_empty_values(self) -> None:
        state: settings.JsonObject = {}

        settings.remember_thread_settings(
            state,
            "thread-1",
            model="gpt-5.5",
            reasoning="xhigh",
            speed="fast",
        )
        state[settings.THREAD_SETTINGS_STATE_KEY] = {
            "thread-1": {
                "model": "gpt-5.5",
                "reasoning": "xhigh",
                "speed": "fast",
                "empty": "",
            },
            "thread-2": "bad",
        }

        self.assertEqual(
            settings.saved_thread_settings(state, "thread-1"),
            {"model": "gpt-5.5", "reasoning": "xhigh", "speed": "fast"},
        )
        self.assertEqual(settings.saved_thread_settings(state, "thread-2"), {})

    def test_session_events_provide_mode_and_speed(self) -> None:
        events: list[settings.JsonObject] = [
            {
                "type": "event_msg",
                "payload": {"type": "task_started", "collaboration_mode_kind": "plan"},
            },
            {
                "type": "turn_context",
                "payload": {
                    "collaboration_mode": {"mode": "default"},
                    "service_tier": "priority",
                },
            },
        ]

        self.assertEqual(settings.collaboration_mode_from_events(events), "default")
        self.assertEqual(settings.service_tier_from_events(events), "fast")

    def test_build_update_maps_standard_speed_to_null_tier(self) -> None:
        self.assertEqual(
            settings.build_thread_settings_update("gpt-5.4", "high", "standard"),
            {"model": "gpt-5.4", "effort": "high", "serviceTier": None},
        )

    def test_format_display_uses_saved_values(self) -> None:
        self.assertEqual(
            settings.format_thread_model_display(
                model="gpt-5.4",
                reasoning="high",
                mode="default",
                speed="",
                saved_settings={"model": "gpt-5.5", "reasoning": "xhigh", "speed": "fast"},
            ),
            "gpt-5.5/xhigh/default/fast",
        )
