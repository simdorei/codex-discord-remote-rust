// Independent fake-browser adapter ported from baseline 3b359b4 test_pro_connector_control.py.
// One responsibility: simulated composer/locator state. No real browser is controlled.
import defaults from './connector_config.mjs';
const config = { ...defaults, ...JSON.parse(process.argv[2]) };

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
const state = {
  attached: config.attached,
  historicalAttached: config.historical_attached,
  nearPrefixAttached: config.near_prefix_attached,
  chat: config.chat,
  work: false,
  workPending: false,
  generation: 0,
  catalog: config.catalog_page,
  catalogLaunched: false,
  catalogPillPending: false,
};
const chatControlCount = config.chat_control_count;
const workControlCount = config.work_control_count;
const proControlCount = config.pro_control_count;
const proControlName = config.pro_control_name;
const workClickChangesState = config.work_click_changes_state;
const workSelectionDelayedUntilWait =
  config.work_selection_delayed_until_wait;
const chatClickChangesState = config.chat_click_changes_state;
const pillSurvivesChat = config.pill_survives_chat;
const koreanPluginButtonCount = config.korean_plugin_button_count;
const englishPluginButtonCount = config.english_plugin_button_count;
const pluginButtonDelayedUntilWait =
  config.plugin_button_delayed_until_wait;
const pluginButtonWaitTimesOut = config.plugin_button_wait_times_out;
const pluginDuplicateAfterFirstObservation =
  config.plugin_duplicate_after_first_observation;
const pluginButtonHasMenu = config.plugin_button_has_menu;
const pluginMenuAttributeDelayedUntilSecondRead =
  config.plugin_menu_attribute_delayed_until_second_read;
const menuCount = config.menu_count;
const menuName = config.menu_name;
const menuChecked = config.menu_checked;
const menuSelectorMatches = config.menu_selector_matches;
const menuClickDetaches = config.menu_click_detaches;
const menuWaitAutoAttaches = config.menu_wait_auto_attaches;
const composerEvaluateFails = config.composer_evaluate_fails;
const pillAppearsAfterFirstComposerTextRead =
  config.pill_appears_after_first_composer_text_read;
const draftAfterSurfaceCount = config.draft_after_surface_count;
const draftAfterWorkClick = config.draft_after_work_click;
const draftDuringPluginWait = config.draft_during_plugin_wait;
const draftAfterPluginButtonClick =
  config.draft_after_plugin_button_click;
const draftOnProCount = config.draft_on_pro_count;
const modernComposer = config.modern_composer;
const catalogPath = config.catalog_path;
const catalogHeadingCount = config.catalog_heading_count;
const catalogButtonCount = config.catalog_button_count;
const catalogClickAttaches = config.catalog_click_attaches;
const catalogPillDelayedUntilWait = config.catalog_pill_delayed_until_wait;
const catalogDraftAfterClick = config.catalog_draft_after_click;
const catalogResultUrl = config.catalog_result_url;
const pillTextInComposer = config.pill_text_in_composer;
let currentPlainComposerText = config.composer_text;
let composerTextReads = 0;
let pluginButtonReady = !pluginButtonDelayedUntilWait;
let pluginObservationPass = 0;
let pluginMenuAttributeReads = 0;

const locator = (kind, generation = state.generation) => ({
  count: async () => {
    if (["composer", "surface"].includes(kind) && state.catalog) return 0;
    if (["composer", "surface", "pill", "near-pill"].includes(kind) &&
        generation !== state.generation) {
      throw new Error("stale locator");
    }
    if (kind === "surface") {
      if (draftAfterSurfaceCount) currentPlainComposerText = "user draft";
      return 1;
    }
    if (kind === "pro") {
      if (draftOnProCount) currentPlainComposerText = "user draft";
      return proControlCount;
    }
    if (kind === "composer") return 1;
    if (kind === "catalog-heading") return state.catalog ? catalogHeadingCount : 0;
    if (kind === "catalog-use") return state.catalog ? catalogButtonCount : 0;
    if (kind === "pill") return state.attached ? 1 : 0;
    if (kind === "near-pill") return state.nearPrefixAttached ? 1 : 0;
    if (kind === "global-pill") {
      return state.attached || state.historicalAttached ? 1 : 0;
    }
    if (kind === "chat") return chatControlCount;
    if (kind === "work") return workControlCount;
    if (kind === "plus") return modernComposer ? 1 : 0;
    if (kind === "plugin-ko") {
      return pluginButtonReady ? koreanPluginButtonCount : 0;
    }
    if (kind === "plugin-en") {
      const lateDuplicate =
        pluginDuplicateAfterFirstObservation && pluginObservationPass >= 1 ? 1 : 0;
      const count = pluginButtonReady ? englishPluginButtonCount + lateDuplicate : 0;
      pluginObservationPass += 1;
      return count;
    }
    if (kind === "menu") return menuCount;
    if (kind === "missing") return 0;
    return 1;
  },
  textContent: async () => {
    if (["composer", "surface", "pill", "near-pill"].includes(kind) &&
        generation !== state.generation) {
      throw new Error("stale locator");
    }
    if (kind === "composer") {
      composerTextReads += 1;
      const plainText =
        pillAppearsAfterFirstComposerTextRead && state.attached
          ? ""
          : currentPlainComposerText;
      const text = plainText +
        (pillTextInComposer && state.attached ? connectorName : "");
      if (pillAppearsAfterFirstComposerTextRead && composerTextReads === 1) {
        state.attached = true;
      }
      return text;
    }
    if (kind === "pill" && state.attached) return connectorName;
    if (kind === "near-pill" && state.nearPrefixAttached) return connectorName;
    if (kind === "menu") return menuName;
    return "";
  },
  evaluate: async () => {
    if (kind === "composer" && composerEvaluateFails) {
      throw new Error("evaluate timeout");
    }
    return "";
  },
  click: async () => {
    calls.push(`click:${kind}`);
    if (kind === "catalog-use") {
      state.catalog = false;
      state.catalogLaunched = true;
      state.catalogPillPending = catalogClickAttaches && catalogPillDelayedUntilWait;
      state.attached = catalogClickAttaches && !state.catalogPillPending;
      state.work = true;
      state.generation += 1;
      currentPlainComposerText = catalogDraftAfterClick ? "user draft" : "";
      if (state.catalogPillPending) currentPlainComposerText = "\ufeff" + connectorName + " ";
    }
    if (kind === "work" && workClickChangesState) {
      state.work = !workSelectionDelayedUntilWait;
      state.workPending = workSelectionDelayedUntilWait;
      state.chat = false;
      state.generation += 1;
      if (draftAfterWorkClick) currentPlainComposerText = "user draft";
    }
    if (kind.startsWith("plugin-") && draftAfterPluginButtonClick) {
      currentPlainComposerText = "user draft";
    }
    if (kind === "menu") {
      state.attached = true;
      state.generation += 1;
      if (menuClickDetaches) throw new Error("menu detached after selection");
    }
    if (kind === "chat" && chatClickChangesState) {
      state.chat = true;
      state.work = false;
      if (!pillSurvivesChat) state.attached = false;
      state.generation += 1;
    }
  },
  type: async (value) => {
    calls.push(`type:${value}`);
    if (kind !== "composer") throw new Error("wrong typing target");
    currentPlainComposerText = value;
  },
  isVisible: async () => {
    if (kind === "plugin-ko") {
      return pluginButtonReady && koreanPluginButtonCount === 1;
    }
    if (kind === "plugin-en") {
      const lateDuplicate =
        pluginDuplicateAfterFirstObservation && pluginObservationPass >= 1 ? 1 : 0;
      return pluginButtonReady && englishPluginButtonCount + lateDuplicate === 1;
    }
    return true;
  },
  waitFor: async (options) => {
    waits.push({ kind, options });
    if (kind === "pill" && state.catalogPillPending) {
      state.catalogPillPending = false;
      state.attached = true;
      state.generation += 1;
      currentPlainComposerText = "\ufeff ";
    }
    if (kind.startsWith("plugin-")) {
      if (pluginButtonWaitTimesOut) throw new Error("plugin wait timeout");
      pluginButtonReady = true;
      if (draftDuringPluginWait) currentPlainComposerText = "user draft";
    }
    if (kind === "menu") {
      if (menuWaitAutoAttaches) {
        state.attached = true;
        state.generation += 1;
        throw new Error("menu detached after automatic attachment");
      }
      if (menuCount === 0) throw new Error("menu not found");
    }
    if (kind === "pill" && !state.attached) throw new Error("pill not found");
  },
  getAttribute: async (name) => {
    if (name === "aria-label" && kind === "composer" && modernComposer) {
      return "ChatGPT와 채팅";
    }
    if (name === "aria-haspopup" && kind === "plus") return "menu";
    if (name === "aria-checked" && kind === "chat") {
      return state.chat ? "true" : "false";
    }
    if (name === "aria-checked" && kind === "work") {
      return state.work ? "true" : "false";
    }
    if (name === "aria-checked" && kind === "menu") return menuChecked;
    if (name === "aria-haspopup" && kind.startsWith("plugin-")) {
      pluginMenuAttributeReads += 1;
      if (
        pluginMenuAttributeDelayedUntilSecondRead &&
        pluginMenuAttributeReads === 1
      ) {
        return null;
      }
      return pluginButtonHasMenu ? "menu" : null;
    }
    return null;
  },
  filter: () => locator(kind),
  locator: (selector) => {
    scopedSelectors.push(selector);
    if (selector === exactPillSelector) return locator("pill", generation);
    if (selector === legacyPrefixPillSelector && state.nearPrefixAttached) {
      return locator("near-pill", generation);
    }
    return locator("missing", generation);
  },
});

globalThis.proConversationTab = {
  url: async () => state.catalog ? "https://chatgpt.com" + catalogPath :
    (state.catalogLaunched ? catalogResultUrl : "https://chatgpt.com/c/fixture"),
  playwright: {
  locator: (selector) => {
    selectors.push(selector);
    if (selector === '[id="prompt-textarea"]') return locator("composer");
    if (selector === '[data-composer-surface="true"]') return locator("surface");
    if (selector === '[data-testid="composer-plus-btn"]') return locator("plus");
    if (selector.startsWith("a[href")) return locator("global-pill");
    if (selector === exactMenuSelector && menuSelectorMatches) return locator("menu");
    return locator("missing");
  },
  getByRole: (role, options) => {
    if (Object.keys(options).some((key) => !["exact", "name"].includes(key))) {
      throw new Error("unsupported getByRole option");
    }
    if (options.exact !== true) return locator("missing");
    if (role === "heading" && options.name === connectorName) return locator("catalog-heading");
    if (role === "button" && options.name === "채팅에서 사용해 보기") return locator("catalog-use");
    if (role === "radio" && options.name === "Chat") return locator("chat");
    if (role === "radio" && options.name === "Work") return locator("work");
    if (role === "button" && options.name === "플러그인") {
      return locator("plugin-ko");
    }
    if (role === "button" && options.name === "Plugins") {
      return locator("plugin-en");
    }
    const actualProName = state.catalogLaunched && state.work ? "GPT-5.6 Sol Light" : proControlName;
    if (role === "button" &&
        (options.name instanceof RegExp
          ? options.name.test(actualProName)
          : options.name === actualProName)) return locator("pro");
    return locator("missing");
  },
  waitForTimeout: async (timeoutMs) => {
    delays.push(timeoutMs);
    nowMs += timeoutMs;
    if (state.workPending) {
      state.work = true;
      state.workPending = false;
    }
  },
} };

const control = await import(process.argv[3]);
const evidence = await control.prepareProConnector(globalThis);
process.stdout.write(JSON.stringify({
  evidence,
  calls,
  delays,
  waits,
  selectors,
  scopedSelectors,
}));
