from __future__ import annotations

from pathlib import Path
from typing import Final

from remote_mcp_server.simdorei_mcp.capability_inventory import (
    EXPECTED_TOOL_NAMES,
)


ROOT: Final = Path(__file__).resolve().parents[1]
FULL_TERMINAL_TOOLS: Final = frozenset(
    {
        "terminal_exec",
        "terminal_window_activate",
        "terminal_window_capture",
        "terminal_window_close",
        "terminal_window_interrupt",
        "terminal_window_keys",
        "terminal_window_list",
        "terminal_window_open",
        "terminal_window_type",
    }
)


def test_release_manifest_preserves_full_terminal_surface() -> None:
    missing = FULL_TERMINAL_TOOLS.difference(EXPECTED_TOOL_NAMES)

    assert missing == set()


def test_gateway_guidance_names_full_terminal_surface() -> None:
    guidance_path = ROOT / "remote_mcp_server/simdorei_mcp/mcp_instructions.py"

    assert guidance_path.is_file()
    guidance = guidance_path.read_text(encoding="utf-8")
    missing = FULL_TERMINAL_TOOLS.difference(guidance.split("`"))

    assert missing == set()


def test_pro_skill_guidance_names_full_terminal_surface() -> None:
    skill_paths = (
        ROOT / ".agents/skills/ask-chatgpt-pro/SKILL.md",
        ROOT / "plugins/codex-discord-remote/skills/ask-chatgpt-pro/SKILL.md",
    )

    for skill_path in skill_paths:
        guidance = skill_path.read_text(encoding="utf-8")
        missing = FULL_TERMINAL_TOOLS.difference(guidance.split("`"))
        assert missing == set(), skill_path


def test_pro_transport_and_skill_are_chrome_only() -> None:
    prompt_contract = (ROOT / "codex_pro_prompt_contract.py").read_text(encoding="utf-8")
    skill_paths = (
        ROOT / ".agents/skills/ask-chatgpt-pro/SKILL.md",
        ROOT / "plugins/codex-discord-remote/skills/ask-chatgpt-pro/SKILL.md",
    )

    assert "plugin://chrome@openai-bundled" in prompt_contract
    assert "plugin://browser@openai-bundled" not in prompt_contract
    for skill_path in skill_paths:
        guidance = skill_path.read_text(encoding="utf-8")
        assert "Use Chrome only" in guidance, skill_path
        assert "in-app Browser" not in guidance, skill_path
