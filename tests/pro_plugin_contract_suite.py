from __future__ import annotations

import unittest


MODULES = (
    "tests.test_pro_connector_control",
    "tests.test_pro_connector_evidence_hook",
    "tests.test_pro_connector_evidence_retry",
    "tests.test_pro_connector_receipt_ordering",
    "tests.test_codex_pro_connector_evidence",
    "tests.test_codex_pro_release_evidence",
    "tests.test_codex_discord_plugin_packaging",
    "tests.test_pro_conversation_map",
    "tests.test_verify_plugin_cachebuster",
    "tests.test_windows_contract_workflow",
)


def load_tests(
    loader: unittest.TestLoader,
    standard_tests: unittest.TestSuite,
    pattern: str | None,
) -> unittest.TestSuite:
    del standard_tests, pattern
    return loader.loadTestsFromNames(MODULES)


def _workflow_command_text(value: object) -> str:
    return (
        str(value)
        .replace("%", "%25")
        .replace("\r", "%0D")
        .replace("\n", "%0A")
    )


def main() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromNames(MODULES)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    for test, traceback in (*result.failures, *result.errors):
        test_name = _workflow_command_text(test)
        details = _workflow_command_text(traceback)
        print(f"::error title=Pro plugin contract failed ({test_name})::{details}")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
