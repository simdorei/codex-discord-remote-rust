from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import ClassVar, Final, Protocol, TypeVar
from uuid import uuid4

from mcp.server.auth.middleware.auth_context import get_access_token
from mcp.server.fastmcp import Context
from mcp.server.fastmcp.exceptions import ToolError
from mcp.server.session import ServerSession
from mcp.types import ToolAnnotations
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import BrokerError
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_REQUIRED_SCOPES,
    COMPUTER_OBSERVE_REQUIRED_SCOPES,
    READ_SCOPE,
    TERMINAL_EXECUTE_REQUIRED_SCOPES,
    TERMINAL_INTERACT_REQUIRED_SCOPES,
    WRITE_SCOPE,
)
from simdorei_mcp_common.operation_base import OperationOutput
from simdorei_mcp_common.operation_requests import ProjectOperation
from simdorei_mcp_common.messages import RequestId

READ_AUTH_META: Final = {
    "securitySchemes": [{"type": "oauth2", "scopes": [READ_SCOPE]}]
}
WRITE_AUTH_META: Final = {
    "securitySchemes": [{"type": "oauth2", "scopes": [READ_SCOPE, WRITE_SCOPE]}]
}
COMPUTER_OBSERVE_AUTH_META: Final = {
    "securitySchemes": [
        {
            "type": "oauth2",
            "scopes": list(COMPUTER_OBSERVE_REQUIRED_SCOPES),
        }
    ]
}
COMPUTER_CONTROL_AUTH_META: Final = {
    "securitySchemes": [
        {
            "type": "oauth2",
            "scopes": list(COMPUTER_CONTROL_REQUIRED_SCOPES),
        }
    ]
}
TERMINAL_EXECUTE_AUTH_META: Final = {
    "securitySchemes": [
        {
            "type": "oauth2",
            "scopes": list(TERMINAL_EXECUTE_REQUIRED_SCOPES),
        }
    ]
}
TERMINAL_INTERACT_AUTH_META: Final = {
    "securitySchemes": [
        {
            "type": "oauth2",
            "scopes": list(TERMINAL_INTERACT_REQUIRED_SCOPES),
        }
    ]
}
ToolContext = Context[ServerSession, None, object]
OutputT = TypeVar("OutputT", bound=OperationOutput)
READ_ONLY_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=True,
    destructiveHint=False,
    openWorldHint=False,
)
WRITE_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=True,
    openWorldHint=False,
)
OPEN_WORLD_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=True,
    openWorldHint=True,
)
TERMINAL_OBSERVE_ANNOTATIONS: Final = READ_ONLY_ANNOTATIONS
TERMINAL_LOCAL_STATE_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=False,
    openWorldHint=False,
)
TERMINAL_LOCAL_DESTRUCTIVE_ANNOTATIONS: Final = WRITE_ANNOTATIONS
COMPUTER_OBSERVE_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=True,
    destructiveHint=False,
    openWorldHint=True,
)
COMPUTER_CONTROL_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=True,
    openWorldHint=True,
)
COMPUTER_STOP_ANNOTATIONS: Final = ToolAnnotations(
    readOnlyHint=False,
    destructiveHint=False,
    openWorldHint=True,
)
SELECT_ANNOTATIONS: Final = READ_ONLY_ANNOTATIONS


class OpenAiToolSession(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        frozen=True,
        extra="ignore",
        populate_by_name=True,
    )

    session: str = Field(alias="openai/session", min_length=1)


@dataclass(frozen=True, slots=True)
class ToolIdentity:
    session: str
    subject: str


class ToolIdentityRequestContext(Protocol):
    @property
    def meta(self) -> BaseModel | None: ...


class ToolIdentityContext(Protocol):
    @property
    def request_context(self) -> ToolIdentityRequestContext: ...


class ToolRequestIdentityContext(ToolIdentityContext, Protocol):
    @property
    def request_id(self) -> str: ...


def tool_identity(
    ctx: ToolIdentityContext,
    required_scope: str | tuple[str, ...],
) -> ToolIdentity:
    meta = ctx.request_context.meta
    if meta is None:
        raise ToolError("ChatGPT did not provide conversation identity metadata.")
    try:
        session = OpenAiToolSession.model_validate_json(
            meta.model_dump_json(by_alias=True)
        )
    except ValidationError as exc:
        raise ToolError(
            "ChatGPT did not provide conversation session metadata."
        ) from exc
    access_token = get_access_token()
    if access_token is None or access_token.subject is None:
        raise ToolError("ChatGPT OAuth identity is unavailable.")
    required_scopes = (
        (required_scope,) if isinstance(required_scope, str) else required_scope
    )
    missing_scopes = tuple(
        scope for scope in required_scopes if scope not in access_token.scopes
    )
    if missing_scopes:
        raise ToolError(f"OAuth scope {missing_scopes[0]} is required.")
    principal_source = (
        f"{len(access_token.subject)}:{access_token.subject}{access_token.client_id}"
    )
    principal = hashlib.sha256(principal_source.encode("utf-8")).hexdigest()
    return ToolIdentity(session=session.session, subject=principal)


def tool_request_id(
    ctx: ToolRequestIdentityContext,
    identity: ToolIdentity,
) -> RequestId:
    source = (
        f"{len(identity.session)}:{identity.session}"
        + f"{identity.subject}:{ctx.request_id}:{uuid4().hex}"
    )
    return RequestId(hashlib.sha256(source.encode("utf-8")).hexdigest())


async def execute_operation(
    ctx: ToolContext,
    broker: BindingBroker,
    operation: ProjectOperation,
    output_type: type[OutputT],
    *,
    required_scope: str | tuple[str, ...],
) -> OutputT:
    identity = tool_identity(ctx, required_scope)
    try:
        output = await broker.project_operation(
            identity.session,
            identity.subject,
            operation,
            request_id=tool_request_id(ctx, identity),
        )
    except BrokerError as exc:
        raise ToolError(str(exc)) from exc
    if not isinstance(output, output_type):
        raise ToolError("The local bridge returned an unexpected operation result.")
    return output
