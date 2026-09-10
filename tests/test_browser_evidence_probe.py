from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path
from typing import cast


PROBE_PATH = Path(
    "plugins/codex-discord-remote/skills/ask-chatgpt-pro/scripts/"
    + "browser_evidence_probe.mjs"
).resolve()


def _object_map(raw: object) -> dict[str, object]:
    if not isinstance(raw, dict):
        raise AssertionError("expected JSON object")
    values = cast(dict[object, object], raw)
    if not all(isinstance(key, str) for key in values):
        raise AssertionError("expected string JSON keys")
    return {cast(str, key): value for key, value in values.items()}


def _run_probe(setup: str) -> dict[str, object]:
    script = f"""
const calls = [];
{setup}
const probe = await import({json.dumps(PROBE_PATH.as_uri())});
const evidence = await probe.probeChrome(globalThis);
process.stdout.write(JSON.stringify({{ evidence, calls }}));
"""
    completed = subprocess.run(
        ["node", "--input-type=module", "--eval", script],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    return _object_map(cast(object, json.loads(completed.stdout)))


def _evidence(result: dict[str, object]) -> dict[str, object]:
    return _object_map(result["evidence"])


class BrowserEvidenceProbeTests(unittest.TestCase):
    def test_empty_tab_list_is_available(self) -> None:
        result = _run_probe(
            """
const browser = { tabs: { list: async () => [] } };
globalThis.agent = {
  browsers: { get: async (kind) => { calls.push(`get:${kind}`); return browser; } },
  documentation: { get: async () => { throw new Error("not expected"); } },
};
"""
        )

        self.assertEqual(result["calls"], ["get:chrome"])
        evidence = _evidence(result)
        self.assertEqual(evidence["browser_type"], "chrome")
        self.assertEqual(evidence["status"], "available")
        self.assertEqual(evidence["tab_count"], 0)
        self.assertFalse(cast(bool, evidence["can_report_unavailable"]))

    def test_second_real_selection_failure_is_unavailable(self) -> None:
        result = _run_probe(
            """
globalThis.agent = {
  browsers: { get: async (kind) => { calls.push(`get:${kind}`); throw new Error("offline"); } },
  documentation: { get: async (name) => { calls.push(`docs:${name}`); } },
};
"""
        )

        self.assertEqual(
            result["calls"],
            ["get:chrome", "docs:chrome-troubleshooting", "get:chrome"],
        )
        evidence = _evidence(result)
        self.assertEqual(evidence["status"], "unavailable")
        self.assertTrue(cast(bool, evidence["can_report_unavailable"]))
        self.assertEqual(evidence["failed_stage"], "select_chrome_retry")

    def test_failed_troubleshooting_is_unverified(self) -> None:
        result = _run_probe(
            """
globalThis.agent = {
  browsers: { get: async () => { calls.push("get"); throw new Error("offline"); } },
  documentation: { get: async () => { calls.push("docs"); throw new Error("docs offline"); } },
};
"""
        )

        self.assertEqual(result["calls"], ["get", "docs"])
        evidence = _evidence(result)
        self.assertEqual(evidence["status"], "unverified")
        self.assertFalse(cast(bool, evidence["can_report_unavailable"]))

    def test_successful_retry_is_available(self) -> None:
        result = _run_probe(
            """
let attempts = 0;
const browser = { tabs: { list: async () => [{ id: 1 }] } };
globalThis.agent = {
  browsers: { get: async () => {
    calls.push("get");
    attempts += 1;
    if (attempts === 1) throw new Error("first failure");
    return browser;
  } },
  documentation: { get: async () => { calls.push("docs"); } },
};
"""
        )

        self.assertEqual(result["calls"], ["get", "docs", "get"])
        evidence = _evidence(result)
        self.assertEqual(evidence["status"], "available")
        self.assertEqual(evidence["selected_stage"], "retry_selection")

    def test_existing_binding_tab_failure_is_not_unavailable(self) -> None:
        result = _run_probe(
            """
globalThis.agent = {
  browsers: { get: async () => { throw new Error("not expected"); } },
  documentation: { get: async () => { throw new Error("not expected"); } },
};
globalThis.chrome = { tabs: { list: async () => { throw new Error("tab failure"); } } };
"""
        )

        evidence = _evidence(result)
        self.assertEqual(evidence["status"], "available")
        self.assertIn("tab failure", cast(str, evidence["tab_state_error"]))
        self.assertFalse(cast(bool, evidence["can_report_unavailable"]))

    def test_public_error_redacts_credentials_and_project_scope(self) -> None:
        result = _run_probe(
            """
globalThis.agent = {
  browsers: { get: async () => { throw new Error("token=secret codex-project-abcdefghijklmnopqrstuvwxyz123456"); } },
  documentation: { get: async () => {} },
};
"""
        )

        error_text = cast(str, _evidence(result)["public_error"])
        self.assertNotIn("secret", error_text)
        self.assertNotIn("abcdefghijklmnopqrstuvwxyz123456", error_text)
        self.assertIn("<redacted>", error_text)


if __name__ == "__main__":
    _ = unittest.main()
