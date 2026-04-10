import unittest
from unittest import mock

from bench.providers import ProviderResult
from bench.runner import (
    classify_provider_failure,
    provider_retry_delay_seconds,
    provider_retry_limit,
    should_retry_provider_failure,
)


class RunnerFailureClassificationTest(unittest.TestCase):
    def test_classify_provider_failure_usage_limit(self) -> None:
        result = ProviderResult(
            failed=True,
            failure_reason=(
                "You've hit your usage limit for GPT-5.3-Codex-Spark. "
                "Try again at Apr 10th, 2026 1:11 AM."
            ),
        )

        self.assertEqual(classify_provider_failure(result), "usage_limited")

    def test_classify_provider_failure_internal_server_error_beats_timeout(self) -> None:
        result = ProviderResult(
            failed=True,
            failure_reason="codex_cli timed out after 420 seconds",
            transcript=[
                (
                    "ERROR rmcp::transport::worker: worker quit with fatal: "
                    "Transport channel closed, when UnexpectedContentType(Some("
                    '"text/plain;charset=UTF-8; body: Internal server error"))'
                )
            ],
        )

        self.assertEqual(classify_provider_failure(result), "internal_server_error")

    def test_retry_policy_only_retries_transient_provider_failures(self) -> None:
        with mock.patch.dict("os.environ", {"CJ_PROVIDER_INFRA_RETRIES": "2"}, clear=False):
            self.assertEqual(provider_retry_limit(), 2)

        self.assertTrue(should_retry_provider_failure("capacity_busy"))
        self.assertTrue(should_retry_provider_failure("internal_server_error"))
        self.assertTrue(should_retry_provider_failure("transport_error"))
        self.assertFalse(should_retry_provider_failure("usage_limited"))
        self.assertFalse(should_retry_provider_failure("auth_required"))
        self.assertEqual(provider_retry_delay_seconds("internal_server_error", 0), 2.0)
        self.assertEqual(provider_retry_delay_seconds("internal_server_error", 1), 5.0)


if __name__ == "__main__":
    unittest.main()
