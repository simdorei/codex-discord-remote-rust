// Independent behavioral assertions retained from baseline 3b359b4, not generated from implementation output.
import { run, check, contract, failed, assert, test } from './support/connector_runner.mjs';
const attached = { attached: true, chat: true };
test('test_attaches_exact_oauth_connector_through_work_picker', () => {
  const result = run();
  check(result, { status: 'verified', action: 'attached', chat_mode: 'chat', pro_mode: true },
    ['click:work', 'click:plugin-ko', 'click:menu', 'click:chat']);
  assert.ok(result.selectors.includes('[data-composer-plugin-impression-id="plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"][role="menuitemcheckbox"]'));
  assert.ok(!result.selectors.some(item => item.includes('> .__menu-item')));
  assert.ok(result.scopedSelectors.includes('a[href="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c?"], a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c#"]'));
  assert.ok(!result.scopedSelectors.includes('a[href^="/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"]'));
});
contract('test_english_plugin_button_is_an_explicit_supported_label',
  { korean_plugin_button_count: 0, english_plugin_button_count: 1 }, { status: 'verified' }, undefined, ['click:plugin-en']);
test('test_waits_for_delayed_plugin_button_after_entering_work_mode', () => {
  const result = run({ plugin_button_delayed_until_wait: true });
  check(result, { status: 'verified' }, undefined, ['click:plugin-ko']);
  const waits = result.waits.filter(wait => wait.kind === 'plugin-ko');
  assert.ok(waits.length >= 1);
  for (const { options } of waits) {
    assert.equal(options.state, 'visible'); assert.ok(options.timeoutMs > 0); assert.ok(options.timeoutMs <= 10000);
  }
});
test('test_waits_for_work_selection_before_resolving_delayed_picker', () => {
  const result = run({ work_selection_delayed_until_wait: true, plugin_button_delayed_until_wait: true });
  check(result, { status: 'verified' });
  assert.ok(result.waits.some(wait => wait.kind === 'plugin-ko'));
  assert.ok(result.delays.length > 0); assert.ok(result.delays.every(delay => delay > 0 && delay <= 100));
});
failed('test_late_duplicate_plugin_button_fails_closed_before_click',
  { plugin_duplicate_after_first_observation: true }, 'plugin_picker', undefined, ['click:plugin-ko', 'click:plugin-en']);
failed('test_plugin_button_wait_timeout_fails_closed_without_click',
  { plugin_button_wait_times_out: true }, 'plugin_picker', undefined, ['click:plugin-ko', 'click:plugin-en']);
contract('test_delayed_menu_attribute_is_stable_before_picker_click',
  { plugin_menu_attribute_delayed_until_second_read: true }, { status: 'verified' }, undefined, ['click:plugin-ko']);
failed('test_draft_during_plugin_wait_stops_before_picker_click',
  { plugin_button_delayed_until_wait: true, draft_during_plugin_wait: true }, 'composer_not_empty', undefined,
  ['click:plugin-ko', 'click:plugin-en', 'click:menu']);
contract('test_already_attached_connector_is_only_verified', attached, { status: 'verified', action: 'already_attached' }, []);
contract('test_historical_pill_is_not_current_composer_evidence', { historical_attached: true },
  { status: 'verified', action: 'attached' }, undefined, ['click:menu']);
contract('test_missing_legacy_chat_radio_is_allowed_for_existing_pill',
  { attached: true, chat: false, chat_control_count: 0, work_control_count: 0 }, { status: 'verified' });
failed('test_missing_chat_after_work_attachment_fails_closed', { chat_control_count: 0 }, 'chat_mode', undefined, ['click:chat']);
failed('test_missing_chat_with_visible_work_is_not_legacy_mode',
  { attached: true, chat: false, chat_control_count: 0, work_control_count: 1 }, 'chat_mode');
contract('test_connector_pill_label_is_not_treated_as_draft', { ...attached, pill_text_in_composer: true }, { status: 'verified' });
failed('test_stale_composer_text_fails_before_any_click', { ...attached, composer_text: 'stale draft' }, 'composer_not_empty', []);
failed('test_draft_appearing_after_surface_check_fails_closed', { draft_after_surface_count: true }, 'composer_not_empty', []);
failed('test_draft_appearing_during_work_switch_stops_before_picker', { draft_after_work_click: true }, 'composer_not_empty', ['click:work']);
failed('test_draft_appearing_before_success_is_not_verified', { ...attached, draft_on_pro_count: true }, 'composer_not_empty');
failed('test_connector_appearing_during_composer_check_fails_closed',
  { composer_text: 'Simdorei Local Project Oauth', pill_text_in_composer: true, pill_appears_after_first_composer_text_read: true }, 'composer_changed');
contract('test_composer_check_does_not_depend_on_page_evaluate', { ...attached, composer_evaluate_fails: true }, { status: 'verified' });
contract('test_delayed_auto_attachment_survives_disappearing_menu', { menu_wait_auto_attaches: true },
  { status: 'verified', click_result: 'verified_without_menu_click' });
contract('test_detached_menu_click_is_verified_from_resulting_pill', { menu_click_detaches: true },
  { status: 'verified', click_result: 'verified_after_error' });
failed('test_wrong_connector_id_fails_closed', { menu_selector_matches: false }, 'connector_match', undefined, ['click:menu']);
failed('test_near_prefix_pill_is_not_accepted_as_exact_connector', { near_prefix_attached: true, work_control_count: 0 }, 'work_mode');
failed('test_duplicate_connector_id_fails_closed', { menu_count: 2 }, 'connector_match');
failed('test_wrong_connector_name_fails_closed', { menu_name: 'Wrong connector' }, 'connector_match', undefined, ['click:menu']);
failed('test_missing_menu_checkbox_state_fails_closed', { menu_checked: null }, 'connector_match');
test('test_missing_or_duplicate_work_control_fails_closed', () => {
  for (const work_control_count of [0, 2]) check(run({ work_control_count }), { status: 'failed', failed_stage: 'work_mode' });
});
test('test_missing_or_ambiguous_plugin_button_fails_closed', () => {
  for (const [korean_plugin_button_count, english_plugin_button_count] of [[0, 0], [2, 0], [1, 1]]) {
    check(run({ korean_plugin_button_count, english_plugin_button_count }), { status: 'failed', failed_stage: 'plugin_picker' },
      undefined, [], ['click:plugin-ko', 'click:plugin-en']);
  }
});
failed('test_plugin_button_must_open_a_menu', { plugin_button_has_menu: false }, 'plugin_picker', undefined, ['click:plugin-ko']);
failed('test_draft_after_plugin_picker_opens_stops_before_attachment', { draft_after_plugin_button_click: true }, 'composer_not_empty', undefined, ['click:menu']);
failed('test_work_click_must_actually_switch_modes', { work_click_changes_state: false }, 'work_mode', ['click:work']);
failed('test_chat_click_must_actually_switch_modes', { chat_click_changes_state: false }, 'chat_mode');
failed('test_connector_must_survive_chat_transition', { pill_survives_chat: false }, 'connector_after_chat');
failed('test_duplicate_chat_control_fails_closed', { ...attached, chat_control_count: 2 }, 'chat_mode');
test('test_explicitly_allowed_pro_names', () => {
  for (const pro_control_name of ['Pro', '6 Pro']) check(run({ ...attached, pro_control_name }), { status: 'verified' });
});
test('test_other_model_names_are_rejected', () => {
  for (const pro_control_name of ['6 Thinking', 'Pro settings', '16 Pro', '6 Pro extra'])
    check(run({ ...attached, pro_control_name }), { failed_stage: 'pro_mode' });
});
failed('test_duplicate_pro_control_fails_closed', { ...attached, pro_control_count: 2 }, 'pro_mode');
