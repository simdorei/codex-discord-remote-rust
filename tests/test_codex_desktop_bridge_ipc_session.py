from __future__ import annotations

import unittest

import codex_desktop_bridge_ipc_session as ipc_session
from codex_bridge_state import JsonObject


class IpcSessionTests(unittest.TestCase):
    def test_read_response_declines_unhandled_client_discovery_request(self) -> None:
        messages = iter(
            [
                {
                    "type": "client-discovery-request",
                    "requestId": "discovery-1",
                    "request": {"method": "thread-follower-start-turn"},
                },
                {
                    "type": "response",
                    "requestId": "request-1",
                    "resultType": "success",
                },
            ]
        )
        written_payloads: list[JsonObject] = []
        deps = ipc_session.IpcSessionDeps(
            read_ipc_message=lambda _handle, _timeout: next(messages),
            write_ipc_message=lambda _handle, payload: written_payloads.append(payload),
            read_ipc_response=lambda _handle, _request, _timeout, _owners: {},
            uuid_factory=lambda: "unused",
            time_now=lambda: 0.0,
            pipe_peek_retry_sec=0.01,
        )

        result = ipc_session.read_ipc_response(7, "request-1", 1.0, {}, deps)

        self.assertEqual(result["resultType"], "success")
        self.assertEqual(
            written_payloads,
            [
                {
                    "type": "client-discovery-response",
                    "requestId": "discovery-1",
                    "response": {"canHandle": False},
                }
            ],
        )

    def test_initialize_ipc_client_writes_initialize_request_and_returns_client_id(self) -> None:
        written_payloads: list[JsonObject] = []
        owner_clients: ipc_session.OwnerClients = {}

        client_id = ipc_session.initialize_ipc_client(
            handle=7,
            owner_clients=owner_clients,
            timeout_sec=1.0,
            deps=_deps_for_response(
                {"resultType": "success", "result": {"clientId": "client-1"}},
                written_payloads,
            ),
        )

        self.assertEqual(client_id, "client-1")
        self.assertEqual(
            written_payloads,
            [
                {
                    "type": "request",
                    "requestId": "request-1",
                    "sourceClientId": "initializing-client",
                    "version": 0,
                    "method": "initialize",
                    "params": {"clientType": "codex-desktop-bridge"},
                }
            ],
        )

    def test_initialize_ipc_client_surfaces_error_response(self) -> None:
        with self.assertRaises(ipc_session.IPCInitializeResponseError) as raised:
            _ = ipc_session.initialize_ipc_client(
                handle=7,
                owner_clients={},
                timeout_sec=1.0,
                deps=_deps_for_response({"resultType": "error", "error": "boom"}),
            )

        self.assertIn("IPC initialize failed: boom", str(raised.exception))

    def test_initialize_ipc_client_rejects_invalid_payload(self) -> None:
        with self.assertRaises(ipc_session.IPCInitializeInvalidPayloadError) as raised:
            _ = ipc_session.initialize_ipc_client(
                handle=7,
                owner_clients={},
                timeout_sec=1.0,
                deps=_deps_for_response({"resultType": "success", "result": "bad"}),
            )

        self.assertEqual(str(raised.exception), "IPC initialize returned an invalid payload.")

    def test_initialize_ipc_client_rejects_missing_client_id(self) -> None:
        with self.assertRaises(ipc_session.IPCInitializeMissingClientIdError) as raised:
            _ = ipc_session.initialize_ipc_client(
                handle=7,
                owner_clients={},
                timeout_sec=1.0,
                deps=_deps_for_response({"resultType": "success", "result": {"clientId": ""}}),
            )

        self.assertEqual(str(raised.exception), "IPC initialize did not return a clientId.")


def _deps_for_response(
    response: JsonObject,
    written_payloads: list[JsonObject] | None = None,
) -> ipc_session.IpcSessionDeps:
    def read_ipc_message(_handle: int, _timeout_sec: float) -> JsonObject:
        raise AssertionError("initialize_ipc_client should not read raw IPC messages")

    def write_ipc_message(_handle: int, payload: JsonObject) -> None:
        if written_payloads is not None:
            written_payloads.append(payload)

    def read_ipc_response(
        _handle: int,
        _request_id: str,
        _timeout_sec: float,
        _owner_clients: ipc_session.OwnerClients,
    ) -> JsonObject:
        return response

    return ipc_session.IpcSessionDeps(
        read_ipc_message=read_ipc_message,
        write_ipc_message=write_ipc_message,
        read_ipc_response=read_ipc_response,
        uuid_factory=lambda: "request-1",
        time_now=lambda: 0.0,
        pipe_peek_retry_sec=0.01,
    )


if __name__ == "__main__":
    _ = unittest.main()
