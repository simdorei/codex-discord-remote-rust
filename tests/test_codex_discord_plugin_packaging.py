from __future__ import annotations

import json
import unittest
from pathlib import Path

from pydantic import BaseModel, Field


class ManifestInterface(BaseModel):
    long_description: str = Field(alias="longDescription")


class PluginManifest(BaseModel):
    skills: str
    version: str
    interface: ManifestInterface


def _parse_manifest(path: Path) -> PluginManifest:
    return PluginManifest.model_validate_json(path.read_text(encoding="utf-8"))


class DiscordPluginPackagingTests(unittest.TestCase):
    def test_plugin_packages_workflow_skills(self) -> None:
        plugin_root = Path("plugins/codex-discord-remote")
        manifest_path = plugin_root / ".codex-plugin/plugin.json"
        manifest = _parse_manifest(manifest_path)
        manifest_json = json.loads(manifest_path.read_text(encoding="utf-8"))
        skill_text = (plugin_root / "skills/deep-interview/SKILL.md").read_text(
            encoding="utf-8"
        )
        auto_research = (
            plugin_root / "skills/deep-interview/auto-research-greenfield.md"
        )
        auto_answer = plugin_root / "skills/deep-interview/auto-answer-uncertain.md"
        notice = plugin_root / "skills/deep-interview/NOTICE.md"
        removed_skill_dirs = [
            plugin_root / "skills/github-project-triage",
            plugin_root / "skills/maintainer-orchestrator",
        ]
        ask_skill = (plugin_root / "skills/ask-chatgpt-pro/SKILL.md").read_text(
            encoding="utf-8"
        )
        source_skill = Path(".agents/skills/ask-chatgpt-pro/SKILL.md").read_text(
            encoding="utf-8"
        )
        conversation_map = (
            plugin_root / "skills/ask-chatgpt-pro/scripts/conversation_map.py"
        )
        source_conversation_map = Path(
            ".agents/skills/ask-chatgpt-pro/scripts/conversation_map.py"
        )
        browser_evidence = (
            plugin_root
            / "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
        )
        source_browser_evidence = Path(
            ".agents/skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
        )
        connector_control = (
            plugin_root
            / "skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs"
        )
        source_connector_control = Path(
            ".agents/skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs"
        )
        browser_hook = plugin_root / "hooks/browser_evidence_hook.py"
        browser_hook_manifest = plugin_root / "hooks/browser-evidence.json"
        connector_hook = plugin_root / "hooks/pro_connector_evidence_hook.py"
        connector_hook_manifest = plugin_root / "hooks/pro-connector-evidence.json"
        ask_metadata = plugin_root / "skills/ask-chatgpt-pro/agents/openai.yaml"
        source_metadata = Path(
            ".agents/skills/ask-chatgpt-pro/agents/openai.yaml"
        )
        intent_qa_skill = plugin_root / "skills/intent-driven-qa/SKILL.md"
        intent_qa_metadata = plugin_root / "skills/intent-driven-qa/agents/openai.yaml"
        skills_doc = Path("docs/plugin-skills.md")

        self.assertEqual(manifest.skills, "./skills/")
        self.assertRegex(manifest.version, r"^0\.1\.0\+codex\.\d{14}$")
        self.assertEqual(ask_skill, source_skill)
        self.assertIn("select_project", ask_skill)
        self.assertIn("file_apply_patch", ask_skill)
        self.assertIn("git_push", ask_skill)
        self.assertIn("conversation_map.py", ask_skill)
        self.assertIn(
            "restart --scope <conversation_scope> --failed-url "
            "<failed-canonical-url>",
            ask_skill,
        )
        self.assertIn("thinking_failure_restart_used", ask_skill)
        self.assertIn("status --scope <conversation_scope>", ask_skill)
        self.assertIn("complete-restart --scope <conversation_scope>", ask_skill)
        self.assertIn("restore-stalled --scope <conversation_scope>", ask_skill)
        self.assertIn("never poll with `acquire`", ask_skill)
        self.assertIn("Never run `restore-stalled` automatically", ask_skill)
        self.assertIn("`status: protected`", ask_skill)
        self.assertIn("must not equal the failed canonical conversation", ask_skill)
        self.assertIn("Expiry marks the owner as stalled; it does not revoke", ask_skill)
        self.assertIn("persists across Codex process restarts", ask_skill)
        self.assertIn("immutable copy of the exact initial consultation request", ask_skill)
        self.assertIn("Never delete or archive the failed ChatGPT conversation", ask_skill)
        self.assertIn("A spinner or active Stop control, elapsed time, partial output", ask_skill)
        self.assertIn("never accept that partial text as the completed answer", ask_skill)
        self.assertIn("Do not replay a request with possible external side effects", ask_skill)
        self.assertIn("[@Chrome](plugin://chrome@openai-bundled)", ask_skill)
        self.assertIn("browser_evidence_hook.py print-probe-code", ask_skill)
        self.assertIn("pro_connector_evidence_hook.py print-probe-code", ask_skill)
        self.assertIn("pro_connector_evidence_hook.py print-retry-probe-code", ask_skill)
        self.assertIn("use `type()` instead of `fill()`", ask_skill)
        self.assertIn("both literal MCP block tags remain", ask_skill)
        normalized_ask_skill = " ".join(ask_skill.split())
        self.assertIn(
            "Keep a turn-local `connector_retry_used` flag initialized to false. "
            "Immediately before every retry helper invocation, including fresh-chat "
            "recovery below, set it to true; if it is already true, stop without sending.",
            normalized_ask_skill,
        )
        request_entry_start = normalized_ask_skill.index(
            "For a contenteditable ChatGPT composer"
        )
        request_entry_end = normalized_ask_skill.index(
            "After selection, prefer the dedicated file tools"
        )
        self.assertEqual(
            normalized_ask_skill[request_entry_start:request_entry_end].strip(),
            "For a contenteditable ChatGPT composer, use `type()` instead of `fill()` "
            "when entering this request because `fill()` may parse the angle-bracket "
            "block as markup and remove it. Before sending, verify that both literal "
            "MCP block tags remain. Reacquire the composer locator after every click, "
            "clear, or typing operation before reading it; ChatGPT may replace the "
            "contenteditable node while preserving what is visibly typed. Validate "
            "Windows paths before typing and construct backslashes with "
            "`String.fromCharCode(92)` when the request passes through nested "
            "JavaScript strings. If either tag is missing, do not send. Clear the "
            "composer first, reacquire its locator, and verify that no non-pill request "
            "text remains; clearing the contenteditable also removes the connector "
            "pill. If `connector_retry_used` is already true, stop without sending. "
            "Otherwise set it to true immediately before invoking the retry helper, "
            "and continue only when it returns `status: verified`. Reacquire the "
            "composer after the retry, enter the corrected request with `type()`, "
            "reacquire it again, and verify the literal tags before sending. Never "
            "invoke either connector helper while the composer contains request text. "
            "`composer_not_empty` is a fail-closed guard against overwriting or sending "
            "user draft text, not a retry signal.",
        )
        self.assertTrue(conversation_map.is_file())
        self.assertEqual(
            conversation_map.read_bytes(),
            source_conversation_map.read_bytes(),
        )
        self.assertTrue(browser_evidence.is_file())
        self.assertEqual(
            browser_evidence.read_text(encoding="utf-8"),
            source_browser_evidence.read_text(encoding="utf-8"),
        )
        self.assertEqual(
            connector_control.read_text(encoding="utf-8"),
            source_connector_control.read_text(encoding="utf-8"),
        )
        self.assertNotIn("hooks", manifest_json)
        self.assertTrue(browser_hook.is_file())
        self.assertTrue(browser_hook_manifest.is_file())
        self.assertTrue(connector_hook.is_file())
        self.assertTrue(connector_hook_manifest.is_file())
        self.assertTrue(ask_metadata.is_file())
        self.assertEqual(
            ask_metadata.read_text(encoding="utf-8"),
            source_metadata.read_text(encoding="utf-8"),
        )
        self.assertTrue(intent_qa_skill.is_file())
        self.assertTrue(intent_qa_metadata.is_file())
        self.assertIn(
            "name: intent-driven-qa",
            intent_qa_skill.read_text(encoding="utf-8"),
        )
        self.assertIn(
            "$intent-driven-qa",
            intent_qa_metadata.read_text(encoding="utf-8"),
        )
        self.assertIn(
            "$codex-discord-remote:intent-driven-qa",
            skills_doc.read_text(encoding="utf-8"),
        )
        self.assertIn("name: deep-interview", skill_text)
        self.assertTrue(auto_research.is_file())
        self.assertTrue(auto_answer.is_file())
        self.assertIn("MIT License", notice.read_text(encoding="utf-8"))
        for removed_skill_dir in removed_skill_dirs:
            self.assertFalse(removed_skill_dir.exists())


if __name__ == "__main__":
    _ = unittest.main()
