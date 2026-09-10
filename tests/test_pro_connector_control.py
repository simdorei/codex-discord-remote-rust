from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path
from typing import cast


CONTROL_PATH = Path(
    "plugins/codex-discord-remote/skills/ask-chatgpt-pro/scripts/"
    "pro_connector_control.mjs"
).resolve()


def _run_control(
    *,
    attached: bool,
    chat: bool,
    historical_attached: bool = False,
    near_prefix_attached: bool = False,
    chat_control_count: int = 1,
    work_control_count: int = 1,
    pro_control_count: int = 1,
    pro_control_name: str = "Pro",
    work_click_changes_state: bool = True,
    work_selection_delayed_until_wait: bool = False,
    chat_click_changes_state: bool = True,
    pill_survives_chat: bool = True,
    composer_text: str = "",
    pill_text_in_composer: bool = False,
    korean_plugin_button_count: int = 1,
    english_plugin_button_count: int = 0,
    plugin_button_delayed_until_wait: bool = False,
    plugin_button_wait_times_out: bool = False,
    plugin_duplicate_after_first_observation: bool = False,
    plugin_button_has_menu: bool = True,
    plugin_menu_attribute_delayed_until_second_read: bool = False,
    menu_count: int = 1,
    menu_name: str = "Simdorei Local Project Oauth",
    menu_checked: str | None = "false",
    menu_selector_matches: bool = True,
    menu_click_detaches: bool = False,
    menu_wait_auto_attaches: bool = False,
    composer_evaluate_fails: bool = False,
    pill_appears_after_first_composer_text_read: bool = False,
    draft_after_surface_count: bool = False,
    draft_after_work_click: bool = False,
    draft_during_plugin_wait: bool = False,
    draft_after_plugin_button_click: bool = False,
    draft_on_pro_count: bool = False,
    modern_composer: bool = False,
    catalog_page: bool = False,
    catalog_path: str = "/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c",
    catalog_heading_count: int = 1,
    catalog_button_count: int = 1,
    catalog_click_attaches: bool = True,
    catalog_pill_delayed_until_wait: bool = False,
    catalog_draft_after_click: bool = False,
    catalog_result_url: str = "https://chatgpt.com/",
) -> dict[str, object]:
    script = f"""
const calls = [];
const delays = [];
const waits = [];
let nowMs = 0;
Date.now = () => nowMs;
const selectors = [];
const scopedSelectors = [];
const connectorName = "Simdorei Local Project Oauth";
const exactPillSelector =
  'a[href="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c?"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c#"]';
const legacyPrefixPillSelector =
  'a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"]';
const exactMenuSelector =
  '[data-composer-plugin-impression-id="plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"][role="menuitemcheckbox"]';
const state = {{
  attached: {str(attached).lower()},
  historicalAttached: {str(historical_attached).lower()},
  nearPrefixAttached: {str(near_prefix_attached).lower()},
  chat: {str(chat).lower()},
  work: false,
  workPending: false,
  generation: 0,
  catalog: {str(catalog_page).lower()},
  catalogLaunched: false,
  catalogPillPending: false,
}};
const chatControlCount = {chat_control_count};
const workControlCount = {work_control_count};
const proControlCount = {pro_control_count};
const proControlName = {json.dumps(pro_control_name)};
const workClickChangesState = {str(work_click_changes_state).lower()};
const workSelectionDelayedUntilWait =
  {str(work_selection_delayed_until_wait).lower()};
const chatClickChangesState = {str(chat_click_changes_state).lower()};
const pillSurvivesChat = {str(pill_survives_chat).lower()};
const koreanPluginButtonCount = {korean_plugin_button_count};
const englishPluginButtonCount = {english_plugin_button_count};
const pluginButtonDelayedUntilWait =
  {str(plugin_button_delayed_until_wait).lower()};
const pluginButtonWaitTimesOut = {str(plugin_button_wait_times_out).lower()};
const pluginDuplicateAfterFirstObservation =
  {str(plugin_duplicate_after_first_observation).lower()};
const pluginButtonHasMenu = {str(plugin_button_has_menu).lower()};
const pluginMenuAttributeDelayedUntilSecondRead =
  {str(plugin_menu_attribute_delayed_until_second_read).lower()};
const menuCount = {menu_count};
const menuName = {json.dumps(menu_name)};
const menuChecked = {json.dumps(menu_checked)};
const menuSelectorMatches = {str(menu_selector_matches).lower()};
const menuClickDetaches = {str(menu_click_detaches).lower()};
const menuWaitAutoAttaches = {str(menu_wait_auto_attaches).lower()};
const composerEvaluateFails = {str(composer_evaluate_fails).lower()};
const pillAppearsAfterFirstComposerTextRead =
  {str(pill_appears_after_first_composer_text_read).lower()};
const draftAfterSurfaceCount = {str(draft_after_surface_count).lower()};
const draftAfterWorkClick = {str(draft_after_work_click).lower()};
const draftDuringPluginWait = {str(draft_during_plugin_wait).lower()};
const draftAfterPluginButtonClick =
  {str(draft_after_plugin_button_click).lower()};
const draftOnProCount = {str(draft_on_pro_count).lower()};
const modernComposer = {str(modern_composer).lower()};
const catalogPath = {json.dumps(catalog_path)};
const catalogHeadingCount = {catalog_heading_count};
const catalogButtonCount = {catalog_button_count};
const catalogClickAttaches = {str(catalog_click_attaches).lower()};
const catalogPillDelayedUntilWait = {str(catalog_pill_delayed_until_wait).lower()};
const catalogDraftAfterClick = {str(catalog_draft_after_click).lower()};
const catalogResultUrl = {json.dumps(catalog_result_url)};
const pillTextInComposer = {str(pill_text_in_composer).lower()};
let currentPlainComposerText = {json.dumps(composer_text)};
let composerTextReads = 0;
let pluginButtonReady = !pluginButtonDelayedUntilWait;
let pluginObservationPass = 0;
let pluginMenuAttributeReads = 0;

const locator = (kind, generation = state.generation) => ({{
  count: async () => {{
    if (["composer", "surface"].includes(kind) && state.catalog) return 0;
    if (["composer", "surface", "pill", "near-pill"].includes(kind) &&
        generation !== state.generation) {{
      throw new Error("stale locator");
    }}
    if (kind === "surface") {{
      if (draftAfterSurfaceCount) currentPlainComposerText = "user draft";
      return 1;
    }}
    if (kind === "pro") {{
      if (draftOnProCount) currentPlainComposerText = "user draft";
      return proControlCount;
    }}
    if (kind === "composer") return 1;
    if (kind === "catalog-heading") return state.catalog ? catalogHeadingCount : 0;
    if (kind === "catalog-use") return state.catalog ? catalogButtonCount : 0;
    if (kind === "pill") return state.attached ? 1 : 0;
    if (kind === "near-pill") return state.nearPrefixAttached ? 1 : 0;
    if (kind === "global-pill") {{
      return state.attached || state.historicalAttached ? 1 : 0;
    }}
    if (kind === "chat") return chatControlCount;
    if (kind === "work") return workControlCount;
    if (kind === "plus") return modernComposer ? 1 : 0;
    if (kind === "plugin-ko") {{
      return pluginButtonReady ? koreanPluginButtonCount : 0;
    }}
    if (kind === "plugin-en") {{
      const lateDuplicate =
        pluginDuplicateAfterFirstObservation && pluginObservationPass >= 1 ? 1 : 0;
      const count = pluginButtonReady ? englishPluginButtonCount + lateDuplicate : 0;
      pluginObservationPass += 1;
      return count;
    }}
    if (kind === "menu") return menuCount;
    if (kind === "missing") return 0;
    return 1;
  }},
  textContent: async () => {{
    if (["composer", "surface", "pill", "near-pill"].includes(kind) &&
        generation !== state.generation) {{
      throw new Error("stale locator");
    }}
    if (kind === "composer") {{
      composerTextReads += 1;
      const plainText =
        pillAppearsAfterFirstComposerTextRead && state.attached
          ? ""
          : currentPlainComposerText;
      const text = plainText +
        (pillTextInComposer && state.attached ? connectorName : "");
      if (pillAppearsAfterFirstComposerTextRead && composerTextReads === 1) {{
        state.attached = true;
      }}
      return text;
    }}
    if (kind === "pill" && state.attached) return connectorName;
    if (kind === "near-pill" && state.nearPrefixAttached) return connectorName;
    if (kind === "menu") return menuName;
    return "";
  }},
  evaluate: async () => {{
    if (kind === "composer" && composerEvaluateFails) {{
      throw new Error("evaluate timeout");
    }}
    return "";
  }},
  click: async () => {{
    calls.push(`click:${{kind}}`);
    if (kind === "catalog-use") {{
      state.catalog = false;
      state.catalogLaunched = true;
      state.catalogPillPending = catalogClickAttaches && catalogPillDelayedUntilWait;
      state.attached = catalogClickAttaches && !state.catalogPillPending;
      state.work = true;
      state.generation += 1;
      currentPlainComposerText = catalogDraftAfterClick ? "user draft" : "";
      if (state.catalogPillPending) currentPlainComposerText = "\\ufeff" + connectorName + " ";
    }}
    if (kind === "work" && workClickChangesState) {{
      state.work = !workSelectionDelayedUntilWait;
      state.workPending = workSelectionDelayedUntilWait;
      state.chat = false;
      state.generation += 1;
      if (draftAfterWorkClick) currentPlainComposerText = "user draft";
    }}
    if (kind.startsWith("plugin-") && draftAfterPluginButtonClick) {{
      currentPlainComposerText = "user draft";
    }}
    if (kind === "menu") {{
      state.attached = true;
      state.generation += 1;
      if (menuClickDetaches) throw new Error("menu detached after selection");
    }}
    if (kind === "chat" && chatClickChangesState) {{
      state.chat = true;
      state.work = false;
      if (!pillSurvivesChat) state.attached = false;
      state.generation += 1;
    }}
  }},
  type: async (value) => {{
    calls.push(`type:${{value}}`);
    if (kind !== "composer") throw new Error("wrong typing target");
    currentPlainComposerText = value;
  }},
  isVisible: async () => {{
    if (kind === "plugin-ko") {{
      return pluginButtonReady && koreanPluginButtonCount === 1;
    }}
    if (kind === "plugin-en") {{
      const lateDuplicate =
        pluginDuplicateAfterFirstObservation && pluginObservationPass >= 1 ? 1 : 0;
      return pluginButtonReady && englishPluginButtonCount + lateDuplicate === 1;
    }}
    return true;
  }},
  waitFor: async (options) => {{
    waits.push({{ kind, options }});
    if (kind === "pill" && state.catalogPillPending) {{
      state.catalogPillPending = false;
      state.attached = true;
      state.generation += 1;
      currentPlainComposerText = "\\ufeff ";
    }}
    if (kind.startsWith("plugin-")) {{
      if (pluginButtonWaitTimesOut) throw new Error("plugin wait timeout");
      pluginButtonReady = true;
      if (draftDuringPluginWait) currentPlainComposerText = "user draft";
    }}
    if (kind === "menu") {{
      if (menuWaitAutoAttaches) {{
        state.attached = true;
        state.generation += 1;
        throw new Error("menu detached after automatic attachment");
      }}
      if (menuCount === 0) throw new Error("menu not found");
    }}
    if (kind === "pill" && !state.attached) throw new Error("pill not found");
  }},
  getAttribute: async (name) => {{
    if (name === "aria-label" && kind === "composer" && modernComposer) {{
      return "ChatGPT와 채팅";
    }}
    if (name === "aria-haspopup" && kind === "plus") return "menu";
    if (name === "aria-checked" && kind === "chat") {{
      return state.chat ? "true" : "false";
    }}
    if (name === "aria-checked" && kind === "work") {{
      return state.work ? "true" : "false";
    }}
    if (name === "aria-checked" && kind === "menu") return menuChecked;
    if (name === "aria-haspopup" && kind.startsWith("plugin-")) {{
      pluginMenuAttributeReads += 1;
      if (
        pluginMenuAttributeDelayedUntilSecondRead &&
        pluginMenuAttributeReads === 1
      ) {{
        return null;
      }}
      return pluginButtonHasMenu ? "menu" : null;
    }}
    return null;
  }},
  filter: () => locator(kind),
  locator: (selector) => {{
    scopedSelectors.push(selector);
    if (selector === exactPillSelector) return locator("pill", generation);
    if (selector === legacyPrefixPillSelector && state.nearPrefixAttached) {{
      return locator("near-pill", generation);
    }}
    return locator("missing", generation);
  }},
}});

globalThis.proConversationTab = {{
  url: async () => state.catalog ? "https://chatgpt.com" + catalogPath :
    (state.catalogLaunched ? catalogResultUrl : "https://chatgpt.com/c/fixture"),
  playwright: {{
  locator: (selector) => {{
    selectors.push(selector);
    if (selector === '[id="prompt-textarea"]') return locator("composer");
    if (selector === '[data-composer-surface="true"]') return locator("surface");
    if (selector === '[data-testid="composer-plus-btn"]') return locator("plus");
    if (selector.startsWith("a[href")) return locator("global-pill");
    if (selector === exactMenuSelector && menuSelectorMatches) return locator("menu");
    return locator("missing");
  }},
  getByRole: (role, options) => {{
    if (Object.keys(options).some((key) => !["exact", "name"].includes(key))) {{
      throw new Error("unsupported getByRole option");
    }}
    if (options.exact !== true) return locator("missing");
    if (role === "heading" && options.name === connectorName) return locator("catalog-heading");
    if (role === "button" && options.name === "채팅에서 사용해 보기") return locator("catalog-use");
    if (role === "radio" && options.name === "Chat") return locator("chat");
    if (role === "radio" && options.name === "Work") return locator("work");
    if (role === "button" && options.name === "플러그인") {{
      return locator("plugin-ko");
    }}
    if (role === "button" && options.name === "Plugins") {{
      return locator("plugin-en");
    }}
    const actualProName = state.catalogLaunched && state.work ? "GPT-5.6 Sol Light" : proControlName;
    if (role === "button" &&
        (options.name instanceof RegExp
          ? options.name.test(actualProName)
          : options.name === actualProName)) return locator("pro");
    return locator("missing");
  }},
  waitForTimeout: async (timeoutMs) => {{
    delays.push(timeoutMs);
    nowMs += timeoutMs;
    if (state.workPending) {{
      state.work = true;
      state.workPending = false;
    }}
  }},
}} }};

const control = await import({json.dumps(CONTROL_PATH.as_uri())});
const evidence = await control.prepareProConnector(globalThis);
process.stdout.write(JSON.stringify({{
  evidence,
  calls,
  delays,
  waits,
  selectors,
  scopedSelectors,
}}));
"""
    completed = subprocess.run(
        ["node", "--input-type=module", "--eval", script],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    raw = cast(object, json.loads(completed.stdout))
    if not isinstance(raw, dict):
        raise AssertionError("expected JSON object")
    return {str(key): value for key, value in cast(dict[object, object], raw).items()}


class ProConnectorControlTests(unittest.TestCase):
    def test_catalog_launch_requires_actual_exact_attachment(self) -> None:
        result = _run_control(attached=False, chat=False,
                              catalog_page=True)
        self.assertEqual(result["evidence"]["status"], "verified")
        self.assertEqual(result["evidence"]["action"], "attached")
        self.assertEqual(result["evidence"]["click_result"], "verified_catalog_launch")
        self.assertEqual(result["calls"], ["click:catalog-use", "click:chat"])

    def test_catalog_waits_for_delayed_pill_before_classifying_text_as_draft(self) -> None:
        result = _run_control(attached=False, chat=False, catalog_page=True,
                              catalog_pill_delayed_until_wait=True,
                              pill_text_in_composer=True,
                              catalog_result_url="https://chatgpt.com/?surface=work")
        self.assertEqual(result["evidence"]["status"], "verified")
        self.assertEqual(result["evidence"]["chat_mode"], "chat")
        self.assertIs(result["evidence"]["pro_mode"], True)
        self.assertEqual(result["calls"], ["click:catalog-use", "click:chat"])

    def test_catalog_wrong_identity_or_ambiguous_controls_never_click(self) -> None:
        for changes in (
            {"catalog_path": "/plugins/unrelated-plugin"},
            {"catalog_heading_count": 0},
            {"catalog_heading_count": 2},
            {"catalog_button_count": 0},
            {"catalog_button_count": 2},
        ):
            with self.subTest(changes=changes):
                result = _run_control(attached=False, chat=False, catalog_page=True,
                                      **changes)
                self.assertEqual(result["evidence"]["status"], "failed")
                self.assertEqual(result["calls"], [])

    def test_catalog_launch_is_not_proof_without_pill_empty_draft_and_pro(self) -> None:
        for changes, stage, switched_chat in (
            ({"catalog_click_attaches": False}, "catalog_attach", False),
            ({"catalog_draft_after_click": True}, "composer_not_empty", False),
            ({"pro_control_name": "6 Thinking"}, "pro_mode", True),
            ({"catalog_result_url": "https://unrelated.invalid/"}, "catalog_navigation", False),
            ({"catalog_result_url": "https://chatgpt.com/plugins/another-plugin"}, "catalog_navigation", False),
        ):
            with self.subTest(changes=changes):
                result = _run_control(attached=False, chat=False, catalog_page=True,
                                      **changes)
                self.assertEqual(result["evidence"]["status"], "failed")
                self.assertEqual(result["evidence"]["failed_stage"], stage)
                expected_calls = ["click:catalog-use"]
                if switched_chat:
                    expected_calls.append("click:chat")
                self.assertEqual(result["calls"], expected_calls)

    def test_modern_chat_without_picker_does_not_invent_attachment(self) -> None:
        result = _run_control(attached=False, chat=False, chat_control_count=0,
                              work_control_count=0, modern_composer=True)
        self.assertEqual(result["evidence"]["failed_stage"], "connector_picker_unavailable")
        self.assertEqual(result["calls"], [])

    def test_modern_chat_does_not_type_with_wrong_model(self) -> None:
        result = _run_control(attached=False, chat=False, chat_control_count=0,
                              work_control_count=0, modern_composer=True,
                              pro_control_name="6 Thinking")
        self.assertEqual(result["evidence"]["failed_stage"], "pro_mode")
        self.assertEqual(result["calls"], [])

    def test_modern_unattached_mention_is_not_verified(self) -> None:
        result = _run_control(attached=False, chat=False, chat_control_count=0,
                              work_control_count=0, modern_composer=True,
                              composer_text="@Simdorei Local Project Oauth")
        self.assertEqual(result["evidence"]["status"], "failed")
        self.assertEqual(result["evidence"]["failed_stage"], "composer_not_empty")
        self.assertEqual(result["calls"], [])

    def test_modern_attachment_does_not_overwrite_concurrent_draft(self) -> None:
        result = _run_control(attached=False, chat=False, chat_control_count=0,
                              work_control_count=0, modern_composer=True,
                              draft_on_pro_count=True)
        self.assertEqual(result["evidence"]["failed_stage"], "composer_not_empty")

    def test_attaches_exact_oauth_connector_through_work_picker(self) -> None:
        result = _run_control(attached=False, chat=True)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "verified")
        self.assertEqual(evidence["action"], "attached")
        self.assertEqual(evidence["chat_mode"], "chat")
        self.assertIs(evidence["pro_mode"], True)
        self.assertEqual(
            result["calls"],
            ["click:work", "click:plugin-ko", "click:menu", "click:chat"],
        )
        selectors = cast(list[str], result["selectors"])
        self.assertIn(
            '[data-composer-plugin-impression-id="plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"][role="menuitemcheckbox"]',
            selectors,
        )
        self.assertFalse(any("> .__menu-item" in item for item in selectors))
        scoped_selectors = cast(list[str], result["scopedSelectors"])
        self.assertIn(
            'a[href="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c?"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c#"]',
            scoped_selectors,
        )
        self.assertNotIn(
            'a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"]',
            scoped_selectors,
        )

    def test_english_plugin_button_is_an_explicit_supported_label(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            korean_plugin_button_count=0,
            english_plugin_button_count=1,
        )

        self.assertEqual(cast(dict[str, object], result["evidence"])["status"], "verified")
        self.assertIn("click:plugin-en", cast(list[str], result["calls"]))

    def test_waits_for_delayed_plugin_button_after_entering_work_mode(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_button_delayed_until_wait=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "verified")
        plugin_waits = [
            cast(dict[str, object], wait)
            for wait in cast(list[object], result["waits"])
            if cast(dict[str, object], wait)["kind"] == "plugin-ko"
        ]
        self.assertGreaterEqual(len(plugin_waits), 1)
        for wait in plugin_waits:
            options = cast(dict[str, object], wait["options"])
            self.assertEqual(options["state"], "visible")
            self.assertGreater(cast(int, options["timeoutMs"]), 0)
            self.assertLessEqual(cast(int, options["timeoutMs"]), 10000)
        self.assertIn("click:plugin-ko", cast(list[str], result["calls"]))

    def test_waits_for_work_selection_before_resolving_delayed_picker(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            work_selection_delayed_until_wait=True,
            plugin_button_delayed_until_wait=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "verified")
        wait_kinds = [
            cast(dict[str, object], wait)["kind"]
            for wait in cast(list[object], result["waits"])
        ]
        self.assertIn("plugin-ko", wait_kinds)
        delays = cast(list[int], result["delays"])
        self.assertTrue(delays)
        self.assertTrue(all(0 < delay <= 100 for delay in delays))

    def test_late_duplicate_plugin_button_fails_closed_before_click(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_duplicate_after_first_observation=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "plugin_picker")
        self.assertNotIn("click:plugin-ko", cast(list[str], result["calls"]))
        self.assertNotIn("click:plugin-en", cast(list[str], result["calls"]))

    def test_plugin_button_wait_timeout_fails_closed_without_click(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_button_wait_times_out=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "plugin_picker")
        self.assertNotIn("click:plugin-ko", cast(list[str], result["calls"]))
        self.assertNotIn("click:plugin-en", cast(list[str], result["calls"]))

    def test_delayed_menu_attribute_is_stable_before_picker_click(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_menu_attribute_delayed_until_second_read=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "verified")
        self.assertIn("click:plugin-ko", cast(list[str], result["calls"]))

    def test_draft_during_plugin_wait_stops_before_picker_click(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_button_delayed_until_wait=True,
            draft_during_plugin_wait=True,
        )

        evidence = cast(dict[str, object], result["evidence"])
        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")
        self.assertNotIn("click:plugin-ko", cast(list[str], result["calls"]))
        self.assertNotIn("click:plugin-en", cast(list[str], result["calls"]))
        self.assertNotIn("click:menu", cast(list[str], result["calls"]))

    def test_already_attached_connector_is_only_verified(self) -> None:
        result = _run_control(attached=True, chat=True)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "verified")
        self.assertEqual(evidence["action"], "already_attached")
        self.assertEqual(result["calls"], [])

    def test_historical_pill_is_not_current_composer_evidence(self) -> None:
        result = _run_control(attached=False, historical_attached=True, chat=True)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "verified")
        self.assertEqual(evidence["action"], "attached")
        self.assertIn("click:menu", cast(list[str], result["calls"]))

    def test_missing_legacy_chat_radio_is_allowed_for_existing_pill(self) -> None:
        result = _run_control(
            attached=True,
            chat=False,
            chat_control_count=0,
            work_control_count=0,
        )

        self.assertEqual(cast(dict[str, object], result["evidence"])["status"], "verified")

    def test_missing_chat_after_work_attachment_fails_closed(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            chat_control_count=0,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "chat_mode")
        self.assertNotIn("click:chat", cast(list[str], result["calls"]))

    def test_missing_chat_with_visible_work_is_not_legacy_mode(self) -> None:
        result = _run_control(
            attached=True,
            chat=False,
            chat_control_count=0,
            work_control_count=1,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "chat_mode")

    def test_connector_pill_label_is_not_treated_as_draft(self) -> None:
        result = _run_control(
            attached=True,
            chat=True,
            pill_text_in_composer=True,
        )

        self.assertEqual(cast(dict[str, object], result["evidence"])["status"], "verified")

    def test_stale_composer_text_fails_before_any_click(self) -> None:
        result = _run_control(attached=True, chat=True, composer_text="stale draft")
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")
        self.assertEqual(result["calls"], [])

    def test_draft_appearing_after_surface_check_fails_closed(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            draft_after_surface_count=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")
        self.assertEqual(result["calls"], [])

    def test_draft_appearing_during_work_switch_stops_before_picker(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            draft_after_work_click=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")
        self.assertEqual(result["calls"], ["click:work"])

    def test_draft_appearing_before_success_is_not_verified(self) -> None:
        result = _run_control(attached=True, chat=True, draft_on_pro_count=True)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")

    def test_connector_appearing_during_composer_check_fails_closed(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            composer_text="Simdorei Local Project Oauth",
            pill_text_in_composer=True,
            pill_appears_after_first_composer_text_read=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_changed")

    def test_composer_check_does_not_depend_on_page_evaluate(self) -> None:
        result = _run_control(
            attached=True,
            chat=True,
            composer_evaluate_fails=True,
        )

        self.assertEqual(cast(dict[str, object], result["evidence"])["status"], "verified")

    def test_delayed_auto_attachment_survives_disappearing_menu(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            menu_wait_auto_attaches=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "verified")
        self.assertEqual(evidence["click_result"], "verified_without_menu_click")

    def test_detached_menu_click_is_verified_from_resulting_pill(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            menu_click_detaches=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "verified")
        self.assertEqual(evidence["click_result"], "verified_after_error")

    def test_wrong_connector_id_fails_closed(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            menu_selector_matches=False,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "connector_match")
        self.assertNotIn("click:menu", cast(list[str], result["calls"]))

    def test_near_prefix_pill_is_not_accepted_as_exact_connector(self) -> None:
        result = _run_control(
            attached=False,
            near_prefix_attached=True,
            chat=True,
            work_control_count=0,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "work_mode")

    def test_duplicate_connector_id_fails_closed(self) -> None:
        result = _run_control(attached=False, chat=True, menu_count=2)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "connector_match")

    def test_wrong_connector_name_fails_closed(self) -> None:
        result = _run_control(attached=False, chat=True, menu_name="Wrong connector")
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "connector_match")
        self.assertNotIn("click:menu", cast(list[str], result["calls"]))

    def test_missing_menu_checkbox_state_fails_closed(self) -> None:
        result = _run_control(attached=False, chat=True, menu_checked=None)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "connector_match")

    def test_missing_or_duplicate_work_control_fails_closed(self) -> None:
        for count in (0, 2):
            with self.subTest(count=count):
                result = _run_control(
                    attached=False,
                    chat=True,
                    work_control_count=count,
                )
                evidence = cast(dict[str, object], result["evidence"])
                self.assertEqual(evidence["status"], "failed")
                self.assertEqual(evidence["failed_stage"], "work_mode")

    def test_missing_or_ambiguous_plugin_button_fails_closed(self) -> None:
        cases = ((0, 0), (2, 0), (1, 1))
        for korean_count, english_count in cases:
            with self.subTest(
                korean_count=korean_count,
                english_count=english_count,
            ):
                result = _run_control(
                    attached=False,
                    chat=True,
                    korean_plugin_button_count=korean_count,
                    english_plugin_button_count=english_count,
                )
                evidence = cast(dict[str, object], result["evidence"])
                self.assertEqual(evidence["status"], "failed")
                self.assertEqual(evidence["failed_stage"], "plugin_picker")
                self.assertNotIn("click:plugin-ko", cast(list[str], result["calls"]))
                self.assertNotIn("click:plugin-en", cast(list[str], result["calls"]))

    def test_plugin_button_must_open_a_menu(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            plugin_button_has_menu=False,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "plugin_picker")
        self.assertNotIn("click:plugin-ko", cast(list[str], result["calls"]))

    def test_draft_after_plugin_picker_opens_stops_before_attachment(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            draft_after_plugin_button_click=True,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "composer_not_empty")
        self.assertNotIn("click:menu", cast(list[str], result["calls"]))

    def test_work_click_must_actually_switch_modes(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            work_click_changes_state=False,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "work_mode")
        self.assertEqual(result["calls"], ["click:work"])

    def test_chat_click_must_actually_switch_modes(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            chat_click_changes_state=False,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "chat_mode")

    def test_connector_must_survive_chat_transition(self) -> None:
        result = _run_control(
            attached=False,
            chat=True,
            pill_survives_chat=False,
        )
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "connector_after_chat")

    def test_duplicate_chat_control_fails_closed(self) -> None:
        result = _run_control(attached=True, chat=True, chat_control_count=2)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "chat_mode")

    def test_explicitly_allowed_pro_names(self) -> None:
        for name in ("Pro", "6 Pro"):
            with self.subTest(name=name):
                result = _run_control(attached=True, chat=True, pro_control_name=name)
                self.assertEqual(result["evidence"]["status"], "verified")

    def test_other_model_names_are_rejected(self) -> None:
        for name in ("6 Thinking", "Pro settings", "16 Pro", "6 Pro extra"):
            with self.subTest(name=name):
                result = _run_control(attached=True, chat=True, pro_control_name=name)
                self.assertEqual(result["evidence"]["failed_stage"], "pro_mode")

    def test_duplicate_pro_control_fails_closed(self) -> None:
        result = _run_control(attached=True, chat=True, pro_control_count=2)
        evidence = cast(dict[str, object], result["evidence"])

        self.assertEqual(evidence["status"], "failed")
        self.assertEqual(evidence["failed_stage"], "pro_mode")


if __name__ == "__main__":
    _ = unittest.main()
