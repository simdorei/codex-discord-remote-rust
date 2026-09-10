from __future__ import annotations

import re
from typing import Final

MASK: Final = "[REDACTED]"

_PEM_BLOCK = re.compile(
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?"
    r"-----END [A-Z ]*PRIVATE KEY-----"
)
_AWS_ACCESS_KEY = re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")
_AWS_SECRET = re.compile(
    r"\b((?:aws_secret_access_key|AWS_SECRET_ACCESS_KEY)\s*[:=]\s*)"
    r"([A-Za-z0-9/+=]{40})\b"
)
_GCP_API_KEY = re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")
_GCP_JSON_FIELD = re.compile(
    r'("(?:private_key_id|client_secret|client_email)"\s*:\s*")([^"]*)(")'
)
_DATABASE_URL = re.compile(
    r"\b((?:DATABASE_URL|DB_URL|POSTGRES_URL|MYSQL_URL|MONGO(?:DB)?_URI)"
    r"\s*=\s*)(\S+)",
    re.IGNORECASE,
)
_CONNECTION_CREDENTIALS = re.compile(
    r"\b([A-Za-z][\w+.-]*://)([^:/\s]+):([^@/\s]+)@"
)
_URL_USERINFO = re.compile(r"\b(https?://)[^/@\s]+@", re.IGNORECASE)
_QUERY_SECRET = re.compile(
    r"([?&](?:access_token|api_key|apikey|auth|key|password|secret|token)=)"
    r"([^&#\s]+)",
    re.IGNORECASE,
)
_GENERIC_SECRET = re.compile(
    r"\b((?:api[_-]?key|apikey|access[_-]?token|auth[_-]?token|secret|"
    r"password|passwd|pwd)\s*[:=]\s*)(['\"]?)([A-Za-z0-9\-_.\/+=]{8,})\2",
    re.IGNORECASE,
)
_BEARER = re.compile(
    r"\b(Bearer\s+)([A-Za-z0-9\-_.~+/]{10,}=*)",
    re.IGNORECASE,
)
_HIGH_ENTROPY = re.compile(r"\b[A-Za-z0-9+/_-]{32,}={0,2}\b")


def redact(text: str) -> str:
    """Mask likely credential material before text crosses the MCP boundary."""
    value = _PEM_BLOCK.sub(MASK, text)
    value = _AWS_ACCESS_KEY.sub(MASK, value)
    value = _AWS_SECRET.sub(lambda match: f"{match.group(1)}{MASK}", value)
    value = _GCP_API_KEY.sub(MASK, value)
    value = _GCP_JSON_FIELD.sub(
        lambda match: f"{match.group(1)}{MASK}{match.group(3)}",
        value,
    )
    value = _DATABASE_URL.sub(lambda match: f"{match.group(1)}{MASK}", value)
    value = _CONNECTION_CREDENTIALS.sub(
        lambda match: f"{match.group(1)}{match.group(2)}:{MASK}@",
        value,
    )
    value = _URL_USERINFO.sub(lambda match: f"{match.group(1)}{MASK}@", value)
    value = _QUERY_SECRET.sub(
        lambda match: f"{match.group(1)}{MASK}",
        value,
    )
    value = _BEARER.sub(lambda match: f"{match.group(1)}{MASK}", value)
    value = _GENERIC_SECRET.sub(
        lambda match: f"{match.group(1)}{match.group(2)}{MASK}{match.group(2)}",
        value,
    )
    return _HIGH_ENTROPY.sub(_mask_high_entropy, value)


def _mask_high_entropy(match: re.Match[str]) -> str:
    token = match.group(0)
    variety = sum(
        (
            any(character.isdigit() for character in token),
            any(character.isupper() for character in token),
            any(character.islower() for character in token),
        )
    )
    if variety < 2:
        return token
    if "/" in token and not any(character.isdigit() for character in token):
        return token
    return MASK
