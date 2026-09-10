from __future__ import annotations

import json
import os
import runpy
import sqlite3
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import cast
from unittest import TestCase, mock


SCRIPT = Path(
    "plugins/codex-discord-remote/skills/ask-chatgpt-pro/scripts/conversation_map.py"
)
PROJECT_SCRIPT = Path(".agents/skills/ask-chatgpt-pro/scripts/conversation_map.py")
SCOPE = "codex-pro-0123456789abcdef01234567"
URL = "https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
REPLACEMENT_URL = "https://chatgpt.com/c/fedcba98-7654-3210-fedc-ba9876543210"


class ConversationMapLeaseTests(TestCase):
    def test_conversation_creation_lease_and_url_survive_new_processes(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))

            first = _run(environment, "acquire", "--scope", SCOPE)
            busy = _run(environment, "acquire", "--scope", SCOPE)
            saved = _run(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                URL,
                "--lease-token",
                first["lease_token"],
            )
            restored = _run(environment, "acquire", "--scope", SCOPE)

            self.assertEqual(first["status"], "acquired")
            self.assertEqual(busy, {"status": "busy"})
            self.assertEqual(saved, {"status": "saved"})
            self.assertEqual(restored, {"status": "found", "url": URL})

    def test_project_and_plugin_skill_ship_the_same_conversation_store(self) -> None:
        self.assertEqual(SCRIPT.read_bytes(), PROJECT_SCRIPT.read_bytes())

    def test_generated_lease_token_cannot_be_parsed_as_an_option(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            with (
                mock.patch.dict(os.environ, environment, clear=True),
                mock.patch(
                    "secrets.token_urlsafe",
                    return_value="-leading-hyphen-token-value",
                ),
            ):
                script_globals: object = runpy.run_path(str(SCRIPT))
                acquire = _object_mapping(script_globals)["acquire"]
                self.assertTrue(callable(acquire))
                lease = _string_mapping(acquire(SCOPE))  # type: ignore[operator]

            lease_token = str(lease["lease_token"])
            completed = _run_process(
                environment,
                "release",
                "--scope",
                SCOPE,
                "--lease-token",
                lease_token,
            )

            self.assertFalse(lease_token.startswith("-"))
            self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_expired_but_still_owned_lease_can_save(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            lease = _run(environment, "acquire", "--scope", SCOPE)
            _expire_lease(environment)

            saved = _run(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                URL,
                "--lease-token",
                lease["lease_token"],
            )

            self.assertEqual(saved, {"status": "saved"})
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": URL},
            )

    def test_reacquired_lease_fences_stale_creator(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            stale_lease = _run(environment, "acquire", "--scope", SCOPE)
            _expire_lease(environment)
            current_lease = _run(environment, "acquire", "--scope", SCOPE)
            _expire_lease(environment)

            rejected = _run_process(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                URL,
                "--lease-token",
                stale_lease["lease_token"],
            )
            saved = _run(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                URL,
                "--lease-token",
                current_lease["lease_token"],
            )

            self.assertEqual(rejected.returncode, 2)
            self.assertIn("missing or was replaced", rejected.stderr)
            self.assertEqual(saved, {"status": "saved"})


class ConversationMapRecoveryTests(TestCase):
    def test_thinking_failure_restart_allows_exactly_one_replacement_chat(
        self,
    ) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)

            def restart() -> dict[str, str]:
                return _run(
                    environment,
                    "restart",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    URL,
                )

            with ThreadPoolExecutor(max_workers=2) as executor:
                results = list(executor.map(lambda _: restart(), range(2)))

            replacement = next(
                result for result in results if result["status"] == "acquired"
            )
            self.assertEqual(
                sorted(result["status"] for result in results),
                ["acquired", "busy"],
            )

            self.assertEqual(
                _run(
                    environment,
                    "set",
                    "--scope",
                    SCOPE,
                    "--url",
                    REPLACEMENT_URL,
                    "--lease-token",
                    replacement["lease_token"],
                ),
                {"status": "saved"},
            )

            self.assertEqual(
                restart(),
                {"status": "superseded", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "restart",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    REPLACEMENT_URL,
                ),
                {"status": "exhausted", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "complete-restart",
                    "--scope",
                    SCOPE,
                    "--url",
                    URL,
                ),
                {"status": "superseded", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "restart",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    REPLACEMENT_URL,
                ),
                {"status": "exhausted", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "complete-restart",
                    "--scope",
                    SCOPE,
                    "--url",
                    REPLACEMENT_URL,
                ),
                {"status": "completed"},
            )
            next_recovery = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                REPLACEMENT_URL,
            )
            self.assertEqual(next_recovery["status"], "acquired")
            self.assertEqual(
                _run(
                    environment,
                    "release",
                    "--scope",
                    SCOPE,
                    "--lease-token",
                    next_recovery["lease_token"],
                ),
                {"status": "released"},
            )

    def test_thinking_failure_restart_fails_closed_for_wrong_or_missing_mapping(
        self,
    ) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)

            wrong_url = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                REPLACEMENT_URL,
            )
            self.assertEqual(wrong_url, {"status": "superseded", "url": URL})
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": URL},
            )

            _ = _run(environment, "delete", "--scope", SCOPE)
            self.assertEqual(
                _run(
                    environment,
                    "restart",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    URL,
                ),
                {"status": "missing"},
            )

    def test_waiting_contender_cannot_fence_a_slow_restart_owner(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            owner = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                URL,
            )
            _expire_lease(environment)

            self.assertEqual(
                _run(environment, "status", "--scope", SCOPE),
                {"status": "stalled"},
            )
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "stalled"},
            )
            self.assertEqual(
                _run(
                    environment,
                    "set",
                    "--scope",
                    SCOPE,
                    "--url",
                    REPLACEMENT_URL,
                    "--lease-token",
                    owner["lease_token"],
                ),
                {"status": "saved"},
            )

    def test_releasing_restart_lease_restores_failed_mapping(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            recovery = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                URL,
            )
            _expire_lease(environment)

            self.assertEqual(
                _run(
                    environment,
                    "release",
                    "--scope",
                    SCOPE,
                    "--lease-token",
                    recovery["lease_token"],
                ),
                {"status": "released"},
            )
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": URL},
            )

    def test_stalled_restart_has_guarded_manual_restore(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            stale_owner = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                URL,
            )
            _expire_lease(environment)

            self.assertEqual(
                _run(
                    environment,
                    "restore-stalled",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    URL,
                ),
                {"status": "restored", "url": URL},
            )
            rejected = _run_process(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                REPLACEMENT_URL,
                "--lease-token",
                stale_owner["lease_token"],
            )
            self.assertEqual(rejected.returncode, 2)
            self.assertIn("missing or was replaced", rejected.stderr)
            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": URL},
            )

    def test_restart_rejects_failed_chat_as_the_replacement(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            recovery = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                URL,
            )
            equivalent_failed_url = URL.replace(
                "chatgpt.com", "www.chatgpt.com"
            ) + "/?model=pro"

            rejected = _run_process(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                equivalent_failed_url,
                "--lease-token",
                recovery["lease_token"],
            )

            self.assertEqual(rejected.returncode, 2)
            self.assertIn("must differ from the failed conversation", rejected.stderr)
            self.assertEqual(
                _run(
                    environment,
                    "set",
                    "--scope",
                    SCOPE,
                    "--url",
                    REPLACEMENT_URL,
                    "--lease-token",
                    recovery["lease_token"],
                ),
                {"status": "saved"},
            )

    def test_normal_delete_cannot_erase_pending_restart_guard(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            recovery = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                URL,
            )
            _ = _run(
                environment,
                "set",
                "--scope",
                SCOPE,
                "--url",
                REPLACEMENT_URL,
                "--lease-token",
                recovery["lease_token"],
            )

            self.assertEqual(
                _run(environment, "delete", "--scope", SCOPE),
                {"status": "protected", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "restart",
                    "--scope",
                    SCOPE,
                    "--failed-url",
                    REPLACEMENT_URL,
                ),
                {"status": "exhausted", "url": REPLACEMENT_URL},
            )
            self.assertEqual(
                _run(
                    environment,
                    "complete-restart",
                    "--scope",
                    SCOPE,
                    "--url",
                    REPLACEMENT_URL,
                ),
                {"status": "completed"},
            )
            self.assertEqual(
                _run(environment, "delete", "--scope", SCOPE),
                {"status": "deleted"},
            )

    def test_existing_database_is_migrated_before_recovery(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            database = environment["SIMDOREI_PRO_CONVERSATION_DB"]
            with closing(sqlite3.connect(database)) as connection:
                _ = connection.execute(
                    """
                    CREATE TABLE conversations (
                        scope TEXT PRIMARY KEY,
                        conversation_url TEXT,
                        lease_hash TEXT,
                        lease_expires_at INTEGER,
                        updated_at INTEGER NOT NULL
                    )
                    """
                )
                _ = connection.execute(
                    """
                    INSERT INTO conversations(
                        scope, conversation_url, lease_hash,
                        lease_expires_at, updated_at
                    ) VALUES (?, ?, NULL, NULL, 0)
                    """,
                    (SCOPE, URL),
                )
                connection.commit()

            self.assertEqual(
                _run(environment, "acquire", "--scope", SCOPE),
                {"status": "found", "url": URL},
            )
            with closing(sqlite3.connect(database)) as connection:
                columns = {
                    str(row[1])
                    for row in connection.execute("PRAGMA table_info(conversations)")
                }
            self.assertTrue({"restart_pending", "restart_from_url"} <= columns)

    def test_restart_accepts_equivalent_canonical_url_forms(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _save_initial_conversation(environment)
            equivalent = URL.replace("chatgpt.com", "www.chatgpt.com") + "/?model=pro#x"

            result = _run(
                environment,
                "restart",
                "--scope",
                SCOPE,
                "--failed-url",
                equivalent,
            )

            self.assertEqual(result["status"], "acquired")

    def test_malformed_store_rows_fail_closed(self) -> None:
        with TemporaryDirectory() as directory:
            environment = _environment(Path(directory))
            _ = _run(environment, "acquire", "--scope", SCOPE)
            database = environment["SIMDOREI_PRO_CONVERSATION_DB"]

            with closing(sqlite3.connect(database)) as connection:
                _ = connection.execute(
                    "UPDATE conversations SET lease_expires_at = ? WHERE scope = ?",
                    ("not-an-integer", SCOPE),
                )
                connection.commit()
            invalid_expiry = _run_process(environment, "acquire", "--scope", SCOPE)

            with closing(sqlite3.connect(database)) as connection:
                _ = connection.execute(
                    """
                    UPDATE conversations
                    SET conversation_url = ?, lease_hash = NULL,
                        lease_expires_at = NULL
                    WHERE scope = ?
                    """,
                    (sqlite3.Binary(b"not-text"), SCOPE),
                )
                connection.commit()
            invalid_url = _run_process(environment, "acquire", "--scope", SCOPE)

            self.assertEqual(invalid_expiry.returncode, 2)
            self.assertIn("store contains invalid data", invalid_expiry.stderr)
            self.assertEqual(invalid_url.returncode, 2)
            self.assertIn("store contains invalid data", invalid_url.stderr)


def _environment(tmp_path: Path) -> dict[str, str]:
    return {
        **os.environ,
        "SIMDOREI_PRO_CONVERSATION_DB": str(tmp_path / "conversations.sqlite3"),
    }


def _expire_lease(environment: dict[str, str]) -> None:
    with closing(
        sqlite3.connect(environment["SIMDOREI_PRO_CONVERSATION_DB"])
    ) as connection:
        _ = connection.execute(
            "UPDATE conversations SET lease_expires_at = 0 WHERE scope = ?",
            (SCOPE,),
        )
        connection.commit()


def _save_initial_conversation(environment: dict[str, str]) -> None:
    lease = _run(environment, "acquire", "--scope", SCOPE)
    assert _run(
        environment,
        "set",
        "--scope",
        SCOPE,
        "--url",
        URL,
        "--lease-token",
        lease["lease_token"],
    ) == {"status": "saved"}


def _run(
    environment: dict[str, str],
    *arguments: str,
) -> dict[str, str]:
    completed = _run_process(environment, *arguments)
    assert completed.returncode == 0, completed.stderr
    value = cast(object, json.loads(completed.stdout))
    return _string_mapping(value)


def _object_mapping(value: object) -> dict[object, object]:
    assert isinstance(value, dict)
    mapping = cast(dict[object, object], value)
    return mapping


def _string_mapping(value: object) -> dict[str, str]:
    mapping = _object_mapping(value)
    result: dict[str, str] = {}
    for key, item in mapping.items():
        assert isinstance(key, str)
        assert isinstance(item, str)
        result[key] = item
    return result


def _run_process(
    environment: dict[str, str],
    *arguments: str,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *arguments],
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=environment,
        check=False,
    )
