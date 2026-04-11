import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from bench.common import BENCH_ROOT, ModelManifest, PolicyManifest, TaskManifest, load_model, load_task
from bench.providers import ClaudeCliProvider
from bench.providers import CodexCliProvider
from bench.providers import ProviderResult
from bench.runner import (
    apply_task_gold_patch,
    classify_provider_failure,
    prepare_workspace_for_run,
    provider_retry_delay_seconds,
    provider_retry_limit,
    run_single,
    select_repair_trigger_source,
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


class CodexProviderConfigTest(unittest.TestCase):
    def test_codex_only_early_aborts_on_terminal_quota_or_capacity_markers(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        provider = CodexCliProvider(model)

        with mock.patch.object(provider, "_run_cli_command", return_value=ProviderResult()) as mocked:
            provider.apply(BENCH_ROOT / "repos" / task.repo_fixture, task)

        markers = mocked.call_args.kwargs["early_abort_markers"]
        self.assertIn("all inference nodes that can serve this model are currently busy", markers)
        self.assertIn("You've hit your usage limit", markers)
        self.assertNotIn("Transport channel closed", markers)
        self.assertNotIn("body: Internal server error", markers)

    def test_codex_disables_user_mcp_servers_for_benchmark_runs(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        provider = CodexCliProvider(model)

        with mock.patch.object(provider, "_run_cli_command", return_value=ProviderResult()) as mocked:
            provider.apply(BENCH_ROOT / "repos" / task.repo_fixture, task)

        command = mocked.call_args.kwargs["command"]
        self.assertEqual(command[:4], ["codex", "exec", "-c", "mcp_servers={}"])


class ClaudeProviderConfigTest(unittest.TestCase):
    def test_claude_does_not_inject_otel_overrides(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        provider = ClaudeCliProvider(model)

        with mock.patch.dict(
            os.environ,
            {
                "OTEL_SDK_DISABLED": "true",
                "OTEL_TRACES_EXPORTER": "none",
                "OTEL_METRICS_EXPORTER": "none",
            },
            clear=False,
        ):
            env = provider._agent_env()

        self.assertNotIn("OTEL_SDK_DISABLED", env)
        self.assertNotIn("OTEL_TRACES_EXPORTER", env)
        self.assertNotIn("OTEL_METRICS_EXPORTER", env)


class RepairPolicyTest(unittest.TestCase):
    def test_verify_only_policy_ignores_public_and_hidden_failures(self) -> None:
        policy = PolicyManifest(
            id="repair-loop-verify-only",
            title="Repair loop (verify only)",
            description="",
            court_jester_mode="required",
            max_repair_rounds=1,
            verify_only_repair=True,
        )

        self.assertEqual(
            select_repair_trigger_source(
                policy=policy,
                verify_failed=True,
                public_ok=False,
                hidden_checks_ran=True,
                hidden_ok=False,
            ),
            "verify",
        )
        self.assertIsNone(
            select_repair_trigger_source(
                policy=policy,
                verify_failed=False,
                public_ok=False,
                hidden_checks_ran=False,
                hidden_ok=True,
            )
        )
        self.assertIsNone(
            select_repair_trigger_source(
                policy=policy,
                verify_failed=False,
                public_ok=True,
                hidden_checks_ran=True,
                hidden_ok=False,
            )
        )


class WorkspacePreparationTest(unittest.TestCase):
    def test_prepare_workspace_reuses_cached_setup(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            workspace1 = tmp_path / "workspace1"
            workspace2 = tmp_path / "workspace2"
            run_dir1 = tmp_path / "run1"
            run_dir2 = tmp_path / "run2"
            cache_root = tmp_path / "cache"
            for path in (workspace1, workspace2, run_dir1, run_dir2):
                path.mkdir(parents=True, exist_ok=True)
            (workspace1 / "base.txt").write_text("base\n")
            (workspace2 / "base.txt").write_text("base\n")

            task = TaskManifest(
                id="setup-cache-task",
                title="",
                repo_fixture="unused",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["base.txt"],
                setup_commands=[["python", "-c", "from pathlib import Path; Path('prepared.txt').write_text('ok\\n')"]],
                setup_cache_key="setup-cache-key",
            )

            with mock.patch.dict(os.environ, {"CJ_SETUP_CACHE_ROOT": str(cache_root)}, clear=False):
                first = prepare_workspace_for_run(task, workspace1, run_dir1)
                self.assertTrue(first.success)
                self.assertFalse(first.cache_hit)
                self.assertTrue((workspace1 / "prepared.txt").exists())

                second = prepare_workspace_for_run(task, workspace2, run_dir2)
                self.assertTrue(second.success)
                self.assertTrue(second.cache_hit)
                self.assertTrue((workspace2 / "prepared.txt").exists())

    def test_prepare_workspace_invalidates_cache_when_fixture_changes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            workspace1 = tmp_path / "workspace1"
            workspace2 = tmp_path / "workspace2"
            run_dir1 = tmp_path / "run1"
            run_dir2 = tmp_path / "run2"
            cache_root = tmp_path / "cache"
            for path in (workspace1, workspace2, run_dir1, run_dir2):
                path.mkdir(parents=True, exist_ok=True)
            (workspace1 / "base.txt").write_text("base\n")
            (workspace2 / "base.txt").write_text("changed\n")

            task = TaskManifest(
                id="setup-cache-task",
                title="",
                repo_fixture="unused",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["base.txt"],
                setup_commands=[
                    [
                        "python",
                        "-c",
                        (
                            "from pathlib import Path; "
                            "Path('prepared.txt').write_text(Path('base.txt').read_text())"
                        ),
                    ]
                ],
                setup_cache_key="setup-cache-key",
            )

            with mock.patch.dict(os.environ, {"CJ_SETUP_CACHE_ROOT": str(cache_root)}, clear=False):
                first = prepare_workspace_for_run(task, workspace1, run_dir1)
                self.assertTrue(first.success)
                self.assertFalse(first.cache_hit)
                self.assertEqual((workspace1 / "prepared.txt").read_text(), "base\n")

                second = prepare_workspace_for_run(task, workspace2, run_dir2)
                self.assertTrue(second.success)
                self.assertFalse(second.cache_hit)
                self.assertEqual((workspace2 / "prepared.txt").read_text(), "changed\n")

    def test_prepare_workspace_invalidates_cache_when_setup_commands_change(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            workspace1 = tmp_path / "workspace1"
            workspace2 = tmp_path / "workspace2"
            run_dir1 = tmp_path / "run1"
            run_dir2 = tmp_path / "run2"
            cache_root = tmp_path / "cache"
            for path in (workspace1, workspace2, run_dir1, run_dir2):
                path.mkdir(parents=True, exist_ok=True)
            (workspace1 / "base.txt").write_text("base\n")
            (workspace2 / "base.txt").write_text("base\n")

            first_task = TaskManifest(
                id="setup-cache-task",
                title="",
                repo_fixture="unused",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["base.txt"],
                setup_commands=[["python", "-c", "from pathlib import Path; Path('prepared.txt').write_text('one\\n')"]],
                setup_cache_key="setup-cache-key",
            )
            second_task = TaskManifest(
                id="setup-cache-task",
                title="",
                repo_fixture="unused",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["base.txt"],
                setup_commands=[["python", "-c", "from pathlib import Path; Path('prepared.txt').write_text('two\\n')"]],
                setup_cache_key="setup-cache-key",
            )

            with mock.patch.dict(os.environ, {"CJ_SETUP_CACHE_ROOT": str(cache_root)}, clear=False):
                first = prepare_workspace_for_run(first_task, workspace1, run_dir1)
                self.assertTrue(first.success)
                self.assertFalse(first.cache_hit)
                self.assertEqual((workspace1 / "prepared.txt").read_text(), "one\n")

                second = prepare_workspace_for_run(second_task, workspace2, run_dir2)
                self.assertTrue(second.success)
                self.assertFalse(second.cache_hit)
                self.assertEqual((workspace2 / "prepared.txt").read_text(), "two\n")


class GoldPatchReplayTest(unittest.TestCase):
    def test_apply_task_gold_patch_updates_expected_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            workspace = tmp_path / "workspace"
            run_dir = tmp_path / "run"
            (workspace / "gold").mkdir(parents=True, exist_ok=True)
            run_dir.mkdir(parents=True, exist_ok=True)
            (workspace / "app.py").write_text("VALUE = 1\n")
            (workspace / "gold" / "fix.patch").write_text(
                (
                    "diff --git a/app.py b/app.py\n"
                    "--- a/app.py\n"
                    "+++ b/app.py\n"
                    "@@ -1 +1 @@\n"
                    "-VALUE = 1\n"
                    "+VALUE = 2\n"
                )
            )
            task = TaskManifest(
                id="gold-patch-task",
                title="",
                repo_fixture="unused",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["app.py"],
                gold_patch_path="gold/fix.patch",
            )

            provider_result, command = apply_task_gold_patch(task, workspace, run_dir, 0)
            self.assertIsNotNone(command)
            self.assertFalse(provider_result.failed)
            self.assertEqual((workspace / "app.py").read_text(), "VALUE = 2\n")
            self.assertEqual(provider_result.changed_files, ["app.py"])

    def test_run_single_can_judge_task_gold_patch_without_provider_generation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "gold_patch_fixture"
            fixture.mkdir(parents=True, exist_ok=True)
            (fixture / "app.py").write_text("VALUE = 1\n")
            (fixture / "gold").mkdir(exist_ok=True)
            (fixture / "gold" / "fix.patch").write_text(
                (
                    "diff --git a/app.py b/app.py\n"
                    "--- a/app.py\n"
                    "+++ b/app.py\n"
                    "@@ -1 +1 @@\n"
                    "-VALUE = 1\n"
                    "+VALUE = 2\n"
                )
            )
            (fixture / "tests").mkdir(exist_ok=True)
            (fixture / "tests" / "public_checks.py").write_text(
                (
                    "from pathlib import Path\n"
                    "import sys\n"
                    "sys.path.insert(0, str(Path(__file__).resolve().parents[1]))\n"
                    "from app import VALUE\n"
                    "assert VALUE == 2\n"
                )
            )

            task = TaskManifest(
                id="gold-patch-run-single",
                title="",
                repo_fixture="gold_patch_fixture",
                prompt="",
                language="python",
                bucket="test",
                verify_paths=["app.py"],
                public_check_commands=[["python", "tests/public_checks.py"]],
                expected_files=["app.py"],
                gold_patch_path="gold/fix.patch",
            )
            model = ModelManifest(id="noop", title="Noop", provider="noop")
            policy = PolicyManifest(
                id="baseline",
                title="Baseline",
                description="",
                court_jester_mode="none",
            )

            with mock.patch("bench.runner.BENCH_ROOT", bench_root):
                result = run_single(
                    task,
                    model,
                    policy,
                    tmp_path / "out",
                    use_task_gold_patches=True,
                )

            self.assertEqual(result["status"], "completed")
            self.assertTrue(result["success"])
            self.assertEqual(result["changed_files"], ["app.py"])


if __name__ == "__main__":
    unittest.main()
