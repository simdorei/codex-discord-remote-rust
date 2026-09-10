"""Single-user OAuth protocol provider. (# noqa: SIZE_OK)"""

from __future__ import annotations

import hmac
import secrets
import time
from dataclasses import dataclass
from typing import assert_never, override
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import anyio
from mcp.server.auth.provider import (
    AccessToken,
    AuthorizeError,
    AuthorizationCode,
    AuthorizationParams,
    OAuthAuthorizationServerProvider,
    RefreshToken,
    TokenError,
)
from mcp.shared.auth import OAuthClientInformationFull, OAuthToken
from pydantic import SecretStr

from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    DEFAULT_OAUTH_SCOPES,
    OAUTH_SCOPES,
    OAuthProviderConfigurationError,
)
from remote_mcp_server.simdorei_mcp.oauth_store import (
    OAuthStore,
    OAuthTokenFamilyLimitError,
    RefreshRotationOutcome,
)


class ApprovalNotFoundError(Exception):
    pass


class ApprovalDeniedError(Exception):
    pass


class ApprovalCapacityError(Exception):
    pass


class TokenCapacityError(Exception):
    pass


@dataclass(frozen=True, slots=True)
class PendingAuthorization:
    client_id: str
    params: AuthorizationParams
    expires_at: float
    failed_attempts: int = 0


class SingleUserOAuthProvider(
    OAuthAuthorizationServerProvider[AuthorizationCode, RefreshToken, AccessToken]
):
    def __init__(
        self,
        *,
        store: OAuthStore,
        owner_token: SecretStr,
        public_base_url: str,
        resource_url: str,
        access_token_seconds: int,
        refresh_token_seconds: int,
        pending_authorization_limit: int,
        authorization_code_limit: int,
        authorization_code_per_client_limit: int,
    ) -> None:
        if authorization_code_limit < 1 or authorization_code_per_client_limit < 1:
            raise OAuthProviderConfigurationError(
                "OAuth authorization code limits must be positive."
            )
        if authorization_code_per_client_limit > authorization_code_limit:
            raise OAuthProviderConfigurationError(
                "The per-client authorization code limit cannot exceed the global limit."
            )
        self._store = store
        self._owner_token = owner_token
        self._public_base_url = public_base_url.rstrip("/")
        self._resource_url = resource_url
        self._access_token_seconds = access_token_seconds
        self._refresh_token_seconds = refresh_token_seconds
        self._pending_authorization_limit = pending_authorization_limit
        self._authorization_code_limit = authorization_code_limit
        self._authorization_code_per_client_limit = authorization_code_per_client_limit
        self._pending: dict[str, PendingAuthorization] = {}
        self._codes: dict[str, AuthorizationCode] = {}
        self._exchanging_codes: dict[str, AuthorizationCode] = {}
        self._lock = anyio.Lock()

    async def close(self) -> None:
        await self._store.close()

    @override
    async def get_client(self, client_id: str) -> OAuthClientInformationFull | None:
        client = await self._store.get_client(client_id)
        if client is None:
            return None
        current_scope = " ".join(OAUTH_SCOPES)
        if client.scope == current_scope:
            return client
        migrated_client = client.model_copy(update={"scope": current_scope})
        await self._store.save_client(migrated_client)
        return migrated_client

    @override
    async def register_client(self, client_info: OAuthClientInformationFull) -> None:
        await self._store.save_client(client_info)

    @override
    async def authorize(
        self,
        client: OAuthClientInformationFull,
        params: AuthorizationParams,
    ) -> str:
        if client.client_id is None:
            raise OAuthProviderConfigurationError(
                "Registered OAuth client is missing client_id."
            )
        if params.resource not in (None, self._resource_url):
            raise AuthorizeError("invalid_request", "Unknown protected resource.")
        request_id = secrets.token_urlsafe(32)
        pending = PendingAuthorization(
            client_id=client.client_id,
            params=params,
            expires_at=time.time() + 300,
        )
        async with self._lock:
            self._discard_expired()
            if len(self._pending) >= self._pending_authorization_limit:
                raise AuthorizeError(
                    "temporarily_unavailable",
                    "Too many authorization requests are awaiting approval.",
                )
            self._pending[request_id] = pending
        return f"{self._public_base_url}/oauth/approve?{urlencode({'request_id': request_id})}"

    async def approve(self, request_id: str, candidate_owner_token: str) -> str:
        async with self._lock:
            self._discard_expired()
            pending = self._pending.get(request_id)
            if pending is None:
                raise ApprovalNotFoundError
            matches = hmac.compare_digest(
                candidate_owner_token,
                self._owner_token.get_secret_value(),
            )
            if not matches:
                attempts = pending.failed_attempts + 1
                if attempts >= 5:
                    self._pending.pop(request_id, None)
                else:
                    self._pending[request_id] = PendingAuthorization(
                        client_id=pending.client_id,
                        params=pending.params,
                        expires_at=pending.expires_at,
                        failed_attempts=attempts,
                    )
                raise ApprovalDeniedError
            retained_codes = (*self._codes.values(), *self._exchanging_codes.values())
            client_code_count = sum(
                code.client_id == pending.client_id for code in retained_codes
            )
            if (
                len(retained_codes) >= self._authorization_code_limit
                or client_code_count >= self._authorization_code_per_client_limit
            ):
                raise ApprovalCapacityError
            code_value = secrets.token_urlsafe(32)
            code = AuthorizationCode(
                code=code_value,
                scopes=pending.params.scopes or DEFAULT_OAUTH_SCOPES,
                expires_at=time.time() + 300,
                client_id=pending.client_id,
                code_challenge=pending.params.code_challenge,
                redirect_uri=pending.params.redirect_uri,
                redirect_uri_provided_explicitly=(
                    pending.params.redirect_uri_provided_explicitly
                ),
                resource=pending.params.resource or self._resource_url,
                subject="owner",
            )
            self._pending.pop(request_id, None)
            self._codes[code_value] = code
        return _append_query(
            str(pending.params.redirect_uri),
            code=code_value,
            state=pending.params.state,
        )

    async def pending_scopes(self, request_id: str) -> tuple[str, ...] | None:
        async with self._lock:
            self._discard_expired()
            pending = self._pending.get(request_id)
            return (
                None
                if pending is None
                else tuple(pending.params.scopes or DEFAULT_OAUTH_SCOPES)
            )

    @override
    async def load_authorization_code(
        self,
        client: OAuthClientInformationFull,
        authorization_code: str,
    ) -> AuthorizationCode | None:
        async with self._lock:
            self._discard_expired()
            code = self._codes.get(authorization_code)
            if code is None or code.client_id != client.client_id:
                return None
            return code

    @override
    async def exchange_authorization_code(
        self,
        client: OAuthClientInformationFull,
        authorization_code: AuthorizationCode,
    ) -> OAuthToken:
        async with self._lock:
            stored = self._codes.get(authorization_code.code)
            if stored is None or stored.client_id != client.client_id:
                raise TokenError(
                    "invalid_grant", "Authorization code was already used."
                )
            self._codes.pop(authorization_code.code)
            self._exchanging_codes[authorization_code.code] = stored
        access, refresh = self._new_token_pair(
            client_id=stored.client_id,
            scopes=stored.scopes,
            subject=stored.subject or "owner",
            resource=stored.resource or self._resource_url,
        )
        try:
            await self._store.save_token_pair(
                access,
                refresh,
                secrets.token_urlsafe(24),
            )
        except OAuthTokenFamilyLimitError as exc:
            with anyio.CancelScope(shield=True):
                await self._restore_authorization_code(stored)
            raise TokenCapacityError from exc
        except BaseException:  # noqa: RUF100  # noqa: BROAD_EXCEPT_OK - restore then re-raise cancellation.
            with anyio.CancelScope(shield=True):
                await self._restore_authorization_code(stored)
            raise
        with anyio.CancelScope(shield=True):
            await self._complete_authorization_code(stored)
        return self._oauth_token(access, refresh)

    @override
    async def load_refresh_token(
        self,
        client: OAuthClientInformationFull,
        refresh_token: str,
    ) -> RefreshToken | None:
        token = await self._store.load_refresh_token(refresh_token)
        if token is None or token.client_id != client.client_id:
            return None
        return token

    @override
    async def exchange_refresh_token(
        self,
        client: OAuthClientInformationFull,
        refresh_token: RefreshToken,
        scopes: list[str],
    ) -> OAuthToken:
        if client.client_id is None:
            raise TokenError("invalid_client", "OAuth client_id is missing.")
        requested_scopes = scopes or refresh_token.scopes
        if not set(requested_scopes).issubset(refresh_token.scopes):
            raise TokenError(
                "invalid_scope",
                "A refresh token cannot acquire additional OAuth scopes.",
            )
        access, new_refresh = self._new_token_pair(
            client_id=client.client_id,
            scopes=requested_scopes,
            subject=refresh_token.subject or "owner",
            resource=self._resource_url,
        )
        outcome = await self._store.rotate_token_pair(
            refresh_token.token,
            access,
            new_refresh,
        )
        match outcome:
            case RefreshRotationOutcome.ROTATED:
                return self._oauth_token(access, new_refresh)
            case (
                RefreshRotationOutcome.MISSING
                | RefreshRotationOutcome.REPLAYED
                | RefreshRotationOutcome.HISTORY_EXHAUSTED
            ):
                raise TokenError("invalid_grant", "Refresh token is no longer valid.")
        assert_never(outcome)

    @override
    async def load_access_token(self, token: str) -> AccessToken | None:
        return await self._store.load_access_token(token)

    @override
    async def revoke_token(self, token: AccessToken | RefreshToken) -> None:
        await self._store.revoke_family(token.token)

    def _new_token_pair(
        self,
        *,
        client_id: str,
        scopes: list[str],
        subject: str,
        resource: str,
    ) -> tuple[AccessToken, RefreshToken]:
        issued_at = int(time.time())
        access = AccessToken(
            token=secrets.token_urlsafe(48),
            client_id=client_id,
            scopes=scopes,
            expires_at=issued_at + self._access_token_seconds,
            resource=resource,
            subject=subject,
        )
        refresh = RefreshToken(
            token=secrets.token_urlsafe(48),
            client_id=client_id,
            scopes=scopes,
            expires_at=issued_at + self._refresh_token_seconds,
            subject=subject,
        )
        return access, refresh

    def _oauth_token(
        self,
        access: AccessToken,
        refresh: RefreshToken,
    ) -> OAuthToken:
        return OAuthToken(
            access_token=access.token,
            expires_in=self._access_token_seconds,
            scope=" ".join(access.scopes),
            refresh_token=refresh.token,
        )

    def _discard_expired(self) -> None:
        now = time.time()
        self._pending = {
            key: value
            for key, value in self._pending.items()
            if value.expires_at >= now
        }
        self._codes = {
            key: value for key, value in self._codes.items() if value.expires_at >= now
        }
        self._exchanging_codes = {
            key: value
            for key, value in self._exchanging_codes.items()
            if value.expires_at >= now
        }

    async def _restore_authorization_code(self, code: AuthorizationCode) -> None:
        async with self._lock:
            self._discard_expired()
            self._exchanging_codes.pop(code.code, None)
            if code.expires_at >= time.time():
                self._codes.setdefault(code.code, code)

    async def _complete_authorization_code(self, code: AuthorizationCode) -> None:
        async with self._lock:
            self._exchanging_codes.pop(code.code, None)


def _append_query(url: str, **values: str | None) -> str:
    parts = urlsplit(url)
    query = parse_qsl(parts.query, keep_blank_values=True)
    query.extend((key, value) for key, value in values.items() if value is not None)
    return urlunsplit(
        (parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment)
    )
