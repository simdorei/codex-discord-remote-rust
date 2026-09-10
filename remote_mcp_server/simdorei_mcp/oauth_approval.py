from __future__ import annotations

from html import escape
from typing import ClassVar

from fastapi import APIRouter, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from remote_mcp_server.simdorei_mcp.oauth_provider import (
    ApprovalCapacityError,
    ApprovalDeniedError,
    ApprovalNotFoundError,
    SingleUserOAuthProvider,
)
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_SCOPE,
    COMPUTER_OBSERVE_SCOPE,
    READ_SCOPE,
    TERMINAL_EXECUTE_SCOPE,
    WRITE_SCOPE,
)

SECURITY_HEADERS = {
    "Cache-Control": "no-store",
    "Content-Security-Policy": (
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; "
        "base-uri 'none'; frame-ancestors 'none'"
    ),
    "Pragma": "no-cache",
    "Referrer-Policy": "no-referrer",
    "X-Content-Type-Options": "nosniff",
}


class ApprovalSubmission(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    request_id: str = Field(min_length=20, max_length=200)
    owner_token: str = Field(min_length=1, max_length=500)


def create_approval_router(
    provider: SingleUserOAuthProvider,
    *,
    approval_path: str,
) -> APIRouter:
    router = APIRouter()

    @router.get("/oauth/approve", response_class=HTMLResponse)
    async def approval_page(  # pyright: ignore[reportUnusedFunction]
        request_id: str,
    ) -> HTMLResponse:
        scopes = await provider.pending_scopes(request_id)
        if scopes is None:
            return _page("This authorization request expired.", status_code=400)
        return _page(_approval_form(request_id, scopes, approval_path))

    @router.post("/oauth/approve")
    async def approve(  # pyright: ignore[reportUnusedFunction]
        request: Request,
    ) -> Response:
        try:
            submission = ApprovalSubmission.model_validate(dict(await request.form()))
        except ValidationError:
            return _page("Invalid authorization request.", status_code=400)
        try:
            redirect_url = await provider.approve(
                submission.request_id,
                submission.owner_token,
            )
        except ApprovalNotFoundError:
            return _page("This authorization request expired.", status_code=400)
        except ApprovalDeniedError:
            scopes = await provider.pending_scopes(submission.request_id)
            if scopes is None:
                return _page("This authorization request expired.", status_code=400)
            return _page(
                _approval_form(
                    submission.request_id,
                    scopes,
                    approval_path,
                    message="The owner token did not match.",
                ),
                status_code=401,
            )
        except ApprovalCapacityError:
            return _page(
                "temporarily_unavailable: OAuth authorization capacity is full.",
                status_code=503,
            )
        return RedirectResponse(
            redirect_url,
            status_code=302,
            headers=SECURITY_HEADERS,
        )

    return router


def _approval_form(
    request_id: str,
    scopes: tuple[str, ...],
    approval_path: str,
    message: str = "",
) -> str:
    safe_request_id = escape(request_id, quote=True)
    safe_approval_path = escape(approval_path, quote=True)
    safe_message = escape(message)
    feedback = f'<p class="error">{safe_message}</p>' if safe_message else ""
    permissions = "".join(
        f"<li><code>{escape(scope)}</code> — {escape(_scope_description(scope))}</li>"
        for scope in scopes
    )
    return f"""
    <main>
      <h1>Connect ChatGPT to your local project</h1>
      <p>Enter the private owner token stored for this MCP server.</p>
      <p>This connection requests these permissions:</p>
      <ul>{permissions}</ul>
      {feedback}
      <form method="post" action="{safe_approval_path}">
        <input type="hidden" name="request_id" value="{safe_request_id}">
        <label>Owner token
          <input type="password" name="owner_token" required autofocus>
        </label>
        <button type="submit">Connect</button>
      </form>
    </main>
    """


def _scope_description(scope: str) -> str:
    descriptions = {
        READ_SCOPE: "Read non-sensitive files in the selected local project.",
        WRITE_SCOPE: "Create, update, delete, and run allowed project operations.",
        TERMINAL_EXECUTE_SCOPE: (
            "Run unrestricted local terminal commands with the selected session's "
            "host permissions. Commands do not require per-run approval."
        ),
        COMPUTER_OBSERVE_SCOPE: (
            "List launched app windows and capture screenshots only from the blank "
            "Notepad owned by the selected chat session."
        ),
        COMPUTER_CONTROL_SCOPE: (
            "Launch isolated Chrome or blank Notepad windows; window-bound keyboard, "
            "mouse, clipboard, and close controls are available only in Notepad."
        ),
    }
    return descriptions.get(scope, "Additional connector permission.")


def _page(content: str, status_code: int = 200) -> HTMLResponse:
    html = f"""<!doctype html>
    <html lang="en"><head><meta charset="utf-8"><meta name="viewport"
    content="width=device-width,initial-scale=1"><title>Simdorei MCP OAuth</title>
    <style>
      body {{ font: 16px system-ui; background:#111827; color:#f9fafb; margin:0; }}
      main {{ max-width:32rem; margin:12vh auto; padding:2rem; background:#1f2937;
              border-radius:1rem; }}
      label,input,button {{ display:block; width:100%; box-sizing:border-box; }}
      input,button {{ margin-top:.5rem; padding:.8rem; border-radius:.5rem; }}
      button {{ margin-top:1rem; cursor:pointer; }} .error {{ color:#fca5a5; }}
    </style></head><body>{content}</body></html>"""
    return HTMLResponse(html, status_code=status_code, headers=SECURITY_HEADERS)
