from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

from codex_app_server_transport_replies import JsonObject
import codex_discord_session_mirror as discord_session_mirror
from codex_discord_session_mirror_item_delivery import SessionMirrorItem

ChannelT = TypeVar("ChannelT")
GetPendingApprovalRequest = Callable[[str], JsonObject | None]


class PendingApprovalDeliveryOwner(Protocol[ChannelT]):
    async def resolve_session_mirror_channel(self, discord_thread_id: int) -> ChannelT | None: ...

    async def send_session_mirror_item(
        self,
        channel: ChannelT,
        item: SessionMirrorItem,
        *,
        target_thread_id: str,
        target_ref: str,
    ) -> None: ...


@dataclass(frozen=True, slots=True)
class PendingApprovalDeliveryDeps(Generic[ChannelT]):
    parse_target: Callable[
        [discord_session_mirror.SessionMirrorTargetMapping],
        discord_session_mirror.SessionMirrorTarget | None,
    ]
    get_pending_request: GetPendingApprovalRequest
    has_delivered: Callable[[str, str], bool]
    claim_delivery: Callable[[str, str], bool]
    resolve_target_ref: Callable[[str], tuple[str | None, str]]
    log: Callable[[str], None]


async def deliver_pending_approval(
    owner: PendingApprovalDeliveryOwner[ChannelT],
    target: discord_session_mirror.SessionMirrorTargetMapping,
    *,
    deps: PendingApprovalDeliveryDeps[ChannelT],
) -> bool:
    parsed_target = deps.parse_target(target)
    if parsed_target is None:
        return False
    request = deps.get_pending_request(parsed_target.codex_thread_id)
    if request is None:
        return False
    request_id = str(request.get("id") or "").strip()
    if not request_id:
        return False
    digest = f"app_server_request:{request_id}"
    if deps.has_delivered(digest, parsed_target.codex_thread_id):
        return False
    params = request.get("params")
    if not isinstance(params, dict):
        return False
    message = str(params.get("message") or "Approval is required.").strip()
    url = str(params.get("url") or "").strip()
    text = "\n".join(part for part in ("[approval_required]", message, url) if part)
    channel = await owner.resolve_session_mirror_channel(parsed_target.discord_thread_id)
    if channel is None:
        return False
    _, target_ref = deps.resolve_target_ref(parsed_target.codex_thread_id)
    await owner.send_session_mirror_item(
        channel,
        {"kind": "interactive", "text": text},
        target_thread_id=parsed_target.codex_thread_id,
        target_ref=target_ref,
    )
    _ = deps.claim_delivery(digest, parsed_target.codex_thread_id)
    deps.log(
        f"session_mirror_pending_approval_sent target={parsed_target.codex_thread_id} "
        + f"request={request_id} channel={parsed_target.discord_thread_id}"
    )
    return True
