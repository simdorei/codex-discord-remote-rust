from __future__ import annotations

import unittest

import codex_model_catalog as catalog
import codex_thread_settings as settings


class CodexModelCatalogTests(unittest.TestCase):
    def test_extracts_app_model_options_and_speed_tiers(self) -> None:
        payload: catalog.JsonObject = {
            "data": [
                {
                    "model": "gpt-5.5",
                    "hidden": False,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "medium"},
                        {"reasoningEffort": "xhigh"},
                    ],
                    "additionalSpeedTiers": ["fast"],
                    "serviceTiers": [{"id": "priority", "name": "Fast"}],
                },
                {
                    "model": "gpt-5.4-mini",
                    "hidden": False,
                    "supportedReasoningEfforts": [{"reasoningEffort": "low"}],
                    "additionalSpeedTiers": [],
                    "serviceTiers": [],
                },
                {"model": "hidden-model", "hidden": True},
            ]
        }

        self.assertEqual(catalog.available_model_ids(payload), ("gpt-5.5", "gpt-5.4-mini"))
        self.assertEqual(catalog.available_reasoning_efforts(payload), ("medium", "xhigh", "low"))
        self.assertEqual(catalog.available_speeds(payload), ("standard", "fast"))
        self.assertEqual(catalog.speed_service_tiers(payload)["fast"], "priority")

    def test_thread_settings_update_uses_app_catalog_validation(self) -> None:
        payload: catalog.JsonObject = {
            "data": [
                {
                    "model": "gpt-5.4-mini",
                    "hidden": False,
                    "supportedReasoningEfforts": [{"reasoningEffort": "medium"}],
                    "additionalSpeedTiers": [],
                    "serviceTiers": [],
                }
            ]
        }

        self.assertEqual(
            settings.build_thread_settings_update(
                "gpt-5.4-mini",
                "medium",
                "standard",
                model_catalog=payload,
            ),
            {"model": "gpt-5.4-mini", "effort": "medium", "serviceTier": None},
        )
        with self.assertRaises(settings.UnsupportedThreadSettingError):
            _ = settings.build_thread_settings_update("gpt-5.5", None, None, model_catalog=payload)
