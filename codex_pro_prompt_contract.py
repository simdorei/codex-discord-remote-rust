from __future__ import annotations

from pathlib import Path
from typing import Final

from codex_app_server_transport_reply_types import JsonArray


PRO_SKILL_NAME: Final = "ask-chatgpt-pro"
CHROME_MENTION_NAME: Final = "Chrome"
CHROME_PLUGIN_URI: Final = "plugin://chrome@openai-bundled"
PRO_SKILL_CALL: Final = f"${PRO_SKILL_NAME} [@{CHROME_MENTION_NAME}]({CHROME_PLUGIN_URI})"
PRO_SKILL_PATH: Final = str(
    (Path(__file__).resolve().parent / ".agents/skills/ask-chatgpt-pro/SKILL.md").resolve()
)


def is_pro_skill_prompt(prompt: str) -> bool:
    if not prompt.startswith(PRO_SKILL_CALL):
        return False
    return len(prompt) == len(PRO_SKILL_CALL) or prompt[len(PRO_SKILL_CALL)].isspace()


def build_turn_input(prompt: str) -> JsonArray:
    turn_input: JsonArray = [{"type": "text", "text": prompt, "text_elements": []}]
    if is_pro_skill_prompt(prompt):
        turn_input.extend(
            (
                {"type": "skill", "name": PRO_SKILL_NAME, "path": PRO_SKILL_PATH},
                {"type": "mention", "name": CHROME_MENTION_NAME, "path": CHROME_PLUGIN_URI},
            )
        )
    return turn_input
