// All catalog/modern-composer cases from test_pro_connector_control.py, baseline 3b359b4.
import { run, check, contract, failed, test } from './support/connector_runner.mjs';
const catalog = { attached: false, chat: false, catalog_page: true };
const modern = { attached: false, chat: false, chat_control_count: 0, work_control_count: 0, modern_composer: true };
contract('test_catalog_launch_requires_actual_exact_attachment', catalog,
  { status: 'verified', action: 'attached', click_result: 'verified_catalog_launch' }, ['click:catalog-use', 'click:chat']);
contract('test_catalog_waits_for_delayed_pill_before_classifying_text_as_draft',
  { ...catalog, catalog_pill_delayed_until_wait: true, pill_text_in_composer: true, catalog_result_url: 'https://chatgpt.com/?surface=work' },
  { status: 'verified', chat_mode: 'chat', pro_mode: true }, ['click:catalog-use', 'click:chat']);
test('test_catalog_wrong_identity_or_ambiguous_controls_never_click', () => {
  for (const change of [{ catalog_path: '/plugins/unrelated-plugin' }, { catalog_heading_count: 0 },
    { catalog_heading_count: 2 }, { catalog_button_count: 0 }, { catalog_button_count: 2 }]) {
    check(run({ ...catalog, ...change }), { status: 'failed' }, []);
  }
});
test('test_catalog_launch_is_not_proof_without_pill_empty_draft_and_pro', () => {
  for (const [change, stage, switched] of [
    [{ catalog_click_attaches: false }, 'catalog_attach', false],
    [{ catalog_draft_after_click: true }, 'composer_not_empty', false],
    [{ pro_control_name: '6 Thinking' }, 'pro_mode', true],
    [{ catalog_result_url: 'https://unrelated.invalid/' }, 'catalog_navigation', false],
    [{ catalog_result_url: 'https://chatgpt.com/plugins/another-plugin' }, 'catalog_navigation', false],
  ]) {
    check(run({ ...catalog, ...change }), { status: 'failed', failed_stage: stage },
      switched ? ['click:catalog-use', 'click:chat'] : ['click:catalog-use']);
  }
});
failed('test_modern_chat_without_picker_does_not_invent_attachment', modern, 'connector_picker_unavailable', []);
failed('test_modern_chat_does_not_type_with_wrong_model', { ...modern, pro_control_name: '6 Thinking' }, 'pro_mode', []);
failed('test_modern_unattached_mention_is_not_verified', { ...modern, composer_text: '@Simdorei Local Project Oauth' }, 'composer_not_empty', []);
failed('test_modern_attachment_does_not_overwrite_concurrent_draft', { ...modern, draft_on_pro_count: true }, 'composer_not_empty');
