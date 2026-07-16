import json
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from bench.common import BENCH_ROOT, ModelManifest, PolicyManifest, TaskManifest, load_model, load_task
from bench.agent_trace import AgentTraceSetup
from bench.providers import ClaudeCliProvider
from bench.providers import CodexCliProvider
from bench.providers import ProviderResult
from bench.runner import (
    apply_task_gold_patch,
    classify_provider_failure,
    format_verify_feedback,
    prepare_workspace_for_run,
    provider_retry_delay_seconds,
    provider_retry_limit,
    run_single,
    select_repair_trigger_source,
    should_retry_provider_failure,
    supports_agent_path_trace,
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

    def test_classify_provider_failure_timeout_beats_internal_server_error_noise(self) -> None:
        result = ProviderResult(
            failed=True,
            timed_out=True,
            failure_reason="codex_cli timed out after 420 seconds",
            transcript=[
                (
                    "ERROR rmcp::transport::worker: worker quit with fatal: "
                    "Transport channel closed, when UnexpectedContentType(Some("
                    '"text/plain;charset=UTF-8; body: Internal server error"))'
                )
            ],
        )

        self.assertEqual(classify_provider_failure(result), "timeout")

    def test_classify_provider_failure_model_unsupported_beats_internal_server_noise(self) -> None:
        result = ProviderResult(
            failed=True,
            failure_reason=(
                "ERROR: {\"type\":\"error\",\"status\":400,"
                "\"error\":{\"type\":\"invalid_request_error\","
                "\"message\":\"The 'gpt-5.1-codex-max' model is not supported when using Codex "
                "with a ChatGPT account.\"}}"
            ),
            transcript=[
                "Transport channel closed",
                "body: Internal server error",
            ],
        )

        self.assertEqual(classify_provider_failure(result), "model_unsupported")

    def test_retry_policy_only_retries_transient_provider_failures(self) -> None:
        with mock.patch.dict("os.environ", {"CJ_PROVIDER_INFRA_RETRIES": "2"}, clear=False):
            self.assertEqual(provider_retry_limit(), 2)

        self.assertTrue(should_retry_provider_failure("capacity_busy"))
        self.assertTrue(should_retry_provider_failure("internal_server_error"))
        self.assertTrue(should_retry_provider_failure("transport_error"))
        self.assertFalse(should_retry_provider_failure("model_unsupported"))
        self.assertFalse(should_retry_provider_failure("timeout"))
        self.assertFalse(should_retry_provider_failure("usage_limited"))
        self.assertFalse(should_retry_provider_failure("auth_required"))
        self.assertEqual(provider_retry_delay_seconds("internal_server_error", 0), 2.0)
        self.assertEqual(provider_retry_delay_seconds("internal_server_error", 1), 5.0)


class CodexProviderConfigTest(unittest.TestCase):
    def test_codex_default_timeout_is_1000_seconds(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        provider = CodexCliProvider(model)

        self.assertEqual(provider._timeout_seconds(None), 1000)

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

    def test_codex_disables_user_mcp_servers_and_plugins_for_benchmark_runs(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        provider = CodexCliProvider(model)

        with mock.patch.object(provider, "_run_cli_command", return_value=ProviderResult()) as mocked:
            provider.apply(BENCH_ROOT / "repos" / task.repo_fixture, task)

        command = mocked.call_args.kwargs["command"]
        self.assertEqual(command[:4], ["codex", "exec", "-c", "mcp_servers={}"])
        self.assertIn("--disable", command)
        self.assertIn("plugins", command)
        self.assertIn("--model", command)
        self.assertIn("gpt-5.4", command)
        self.assertIn('-c', command)
        self.assertIn('model_reasoning_effort="medium"', command)

    def test_codex_prefers_task_specific_timeout(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        task = TaskManifest(
            id="timeout-override",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
            provider_timeout_seconds=900,
        )
        provider = CodexCliProvider(model)

        self.assertEqual(provider._timeout_seconds(task), 900)

    def test_codex_prefers_task_specific_idle_timeout(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        task = TaskManifest(
            id="idle-timeout-override",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
            provider_idle_timeout_seconds=300,
        )
        provider = CodexCliProvider(model)

        self.assertEqual(provider._idle_timeout_seconds(task), 300)


class ClaudeProviderConfigTest(unittest.TestCase):
    def test_claude_default_timeout_is_1000_seconds(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        provider = ClaudeCliProvider(model)

        self.assertEqual(provider._timeout_seconds(None), 1000)

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

    def test_claude_preserves_trace_env_overrides_without_mutating_otel(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        provider = ClaudeCliProvider(model)

        env = provider._agent_env({"PATH": "/tmp/trace-shim", "CJ_AGENT_TRACE_FILE": "/tmp/trace.jsonl"})

        self.assertEqual(env["PATH"], "/tmp/trace-shim")
        self.assertEqual(env["CJ_AGENT_TRACE_FILE"], "/tmp/trace.jsonl")
        self.assertNotIn("OTEL_SDK_DISABLED", env)

    def test_claude_prefers_task_specific_timeout(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = TaskManifest(
            id="timeout-override",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
            provider_timeout_seconds=900,
        )
        provider = ClaudeCliProvider(model)

        self.assertEqual(provider._timeout_seconds(task), 900)

    def test_claude_prefers_task_specific_idle_timeout(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = TaskManifest(
            id="idle-timeout-override",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
            provider_idle_timeout_seconds=300,
        )
        provider = ClaudeCliProvider(model)

        self.assertIsNone(provider._idle_timeout_seconds(task))

    def test_claude_uses_bare_benchmark_mode_only_when_explicitly_enabled(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        provider = ClaudeCliProvider(model)

        with mock.patch.dict(
            os.environ,
            {"ANTHROPIC_API_KEY": "test-key", "CJ_CLAUDE_BARE_MODE": "1"},
            clear=False,
        ):
            with mock.patch.object(provider, "_run_cli_command", return_value=ProviderResult()) as mocked:
                provider.apply(BENCH_ROOT / "repos" / task.repo_fixture, task)

        command = mocked.call_args.kwargs["command"]
        self.assertIn("--bare", command)
        self.assertIn("--setting-sources", command)
        self.assertIn("project,local", command)
        self.assertIn("--disable-slash-commands", command)
        self.assertIn("--no-chrome", command)
        self.assertIn("--model", command)
        self.assertIn("claude-opus-4-6", command)
        self.assertIn("--effort", command)
        self.assertIn("medium", command)

    def test_claude_does_not_use_bare_mode_from_api_key_alone(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        provider = ClaudeCliProvider(model)

        with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": "test-key"}, clear=False):
            with mock.patch.object(provider, "_run_cli_command", return_value=ProviderResult()) as mocked:
                provider.apply(BENCH_ROOT / "repos" / task.repo_fixture, task)

        command = mocked.call_args.kwargs["command"]
        self.assertNotIn("--bare", command)
        self.assertIn("--setting-sources", command)

    def test_claude_fast_path_registers_provider_process_for_cleanup(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = TaskManifest(
            id="fast-path-task",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
        )
        provider = ClaudeCliProvider(model)

        mocked_proc = mock.Mock()
        mocked_proc.communicate.return_value = ("{}", "")
        mocked_proc.returncode = 0

        with (
            mock.patch("subprocess.Popen", return_value=mocked_proc) as mocked_popen,
            mock.patch("bench.providers._register_active_provider_proc") as mocked_register,
            mock.patch("bench.providers._unregister_active_provider_proc") as mocked_unregister,
        ):
            result = provider._run_cli_command(
                command=["claude", "-p", "ok"],
                workspace=BENCH_ROOT,
                task=task,
            )

        self.assertTrue(mocked_popen.call_args.kwargs["start_new_session"])
        mocked_register.assert_called_once_with(mocked_proc)
        mocked_unregister.assert_called_once_with(mocked_proc)
        self.assertFalse(result.failed)

    def test_claude_streaming_path_tolerates_missing_early_abort_markers(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")
        task = TaskManifest(
            id="idle-timeout-override",
            title="",
            repo_fixture="mini_py_service",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
            provider_idle_timeout_seconds=300,
        )
        provider = ClaudeCliProvider(model)

        with mock.patch("subprocess.Popen") as mocked_popen:
            mocked_proc = mock.Mock()
            mocked_proc.stdout = mock.Mock()
            mocked_proc.stderr = mock.Mock()
            mocked_proc.poll.return_value = 0
            mocked_proc.communicate.return_value = ("", "")
            mocked_proc.returncode = 0
            mocked_popen.return_value = mocked_proc
            with mock.patch("selectors.DefaultSelector") as mocked_selector_cls:
                mocked_selector = mock.Mock()
                mocked_selector.get_map.return_value = {}
                mocked_selector.select.return_value = []
                mocked_selector_cls.return_value = mocked_selector
                result = provider._run_cli_command(
                    command=["claude", "-p", "ok"],
                    workspace=BENCH_ROOT,
                    task=task,
                )

        self.assertTrue(mocked_popen.call_args.kwargs["start_new_session"])
        self.assertFalse(result.failed)


class VerifyFeedbackFormattingTest(unittest.TestCase):
    def test_tests_only_verify_feedback_prefers_owner_hints_over_failed_call_site(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            workspace = Path(tmpdir)
            lib_dir = workspace / "lib"
            lib_dir.mkdir(parents=True, exist_ok=True)
            (lib_dir / "http.ts").write_text(
                'import { parseQueryString } from "./query.ts";\n'
                "export function decorateRequest(url: string) {\n"
                '  return parseQueryString(url, "extended");\n'
                "}\n"
            )
            (lib_dir / "query.ts").write_text(
                "export function parseQueryString(input: string, mode: string) {\n"
                "  return { input, mode };\n"
                "}\n"
            )
            task = TaskManifest(
                id="ts-tests-only-feedback",
                title="",
                repo_fixture="mini_py_service",
                prompt="",
                language="typescript",
                bucket="test",
                verify_paths=["lib/http.ts", "lib/query.ts"],
                verify_test_path="tests/verify_spec.ts",
                verify_tests_only=True,
            )

            feedback = format_verify_feedback(
                [
                    {
                        "path": "lib/http.ts",
                        "response": {
                            "verdict": "inconclusive",
                            "schema_version": 3,
                            "strength": "static_checked",
                            "summary": {},
                            "stages": [
                                {
                                    "name": "test",
                                    "status": "inconclusive",
                                    "duration_ms": 1,
                                    "detail": {
                                        "stderr": "Process timed out",
                                        "stdout": "",
                                        "timed_out": True,
                                    },
                                    "message": "Process timed out",
                                }
                            ],
                        },
                    }
                ],
                workspace=workspace,
                task=task,
            )

            self.assertIn("Likely owner files: lib/query.ts", feedback)
            self.assertIn("Related call site: lib/http.ts", feedback)
            self.assertNotIn("File: lib/http.ts", feedback)
            self.assertNotIn("Evidence: Process timed out", feedback)


class AgentTraceConfigTest(unittest.TestCase):
    def test_agent_trace_disabled_by_default(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")

        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertFalse(supports_agent_path_trace(model))

    def test_agent_trace_enabled_when_requested(self) -> None:
        model = load_model(BENCH_ROOT / "models" / "claude-default.json")

        with mock.patch.dict(os.environ, {"CJ_AGENT_TRACE": "1"}, clear=False):
            self.assertTrue(supports_agent_path_trace(model))


class AgentTraceEnvIsolationTest(unittest.TestCase):
    def test_run_single_passes_trace_env_to_provider_without_global_env_mutation(self) -> None:
        task = TaskManifest(
            id="trace-env-isolation",
            title="",
            repo_fixture="mini_py_service",
            prompt="noop",
            language="python",
            bucket="test",
            verify_paths=["profile.py"],
        )
        model = load_model(BENCH_ROOT / "models" / "codex-default.json")
        policy = PolicyManifest(
            id="baseline",
            title="",
            description="",
            court_jester_mode="none",
        )
        captured: dict[str, object] = {}

        class FakeProvider:
            supports_repair = True

            def apply(
                self,
                workspace: Path,
                task: TaskManifest,
                *,
                feedback: str | None = None,
                attempt: int = 0,
                history: list[dict[str, object]] | None = None,
                env_overrides: dict[str, str] | None = None,
            ) -> ProviderResult:
                captured["env_overrides"] = dict(env_overrides or {})
                captured["global_trace_file"] = os.environ.get("CJ_AGENT_TRACE_FILE")
                return ProviderResult(
                    parsed_summary={"status": "completed", "files_changed": []},
                    changed_files=[],
                )

        with tempfile.TemporaryDirectory() as tmpdir:
            output_root = Path(tmpdir)
            trace_setup = AgentTraceSetup(
                trace_dir=str(output_root / "trace"),
                events_path=str(output_root / "trace" / "events.jsonl"),
                summary_path=str(output_root / "trace" / "summary.json"),
                shim_dir=str(output_root / "trace" / "shim"),
                env_updates={"CJ_AGENT_TRACE_FILE": "/tmp/fake-trace.jsonl", "PATH": "/tmp/fake-shim"},
                wrapped_commands=["rg"],
            )
            with mock.patch.dict(os.environ, {"CJ_AGENT_TRACE": "1"}, clear=False):
                with mock.patch("bench.runner.provider_from_manifest", return_value=FakeProvider()):
                    with mock.patch("bench.runner.prepare_agent_trace", return_value=trace_setup):
                        with mock.patch("bench.runner.summarize_agent_trace", return_value={"event_count": 0}):
                            result = run_single(
                                task,
                                model,
                                policy,
                                output_root,
                            )

        self.assertEqual(result["status"], "completed")
        self.assertEqual(captured["env_overrides"], trace_setup.env_updates)
        self.assertIsNone(captured["global_trace_file"])

    def test_non_tests_only_verify_feedback_keeps_file_scoped_timeout_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            workspace = Path(tmpdir)
            lib_dir = workspace / "lib"
            lib_dir.mkdir(parents=True, exist_ok=True)
            (lib_dir / "http.ts").write_text("export function decorateRequest() {}\n")
            task = TaskManifest(
                id="ts-file-scoped-feedback",
                title="",
                repo_fixture="mini_py_service",
                prompt="",
                language="typescript",
                bucket="test",
                verify_paths=["lib/http.ts"],
            )

            feedback = format_verify_feedback(
                [
                    {
                        "path": "lib/http.ts",
                        "response": {
                            "verdict": "inconclusive",
                            "schema_version": 3,
                            "strength": "static_checked",
                            "summary": {},
                            "stages": [
                                {
                                    "name": "test",
                                    "status": "inconclusive",
                                    "duration_ms": 1,
                                    "detail": {
                                        "stderr": "Process timed out",
                                        "stdout": "",
                                        "timed_out": True,
                                    },
                                    "message": "Process timed out",
                                }
                            ],
                        },
                    }
                ],
                workspace=workspace,
                task=task,
            )

            self.assertIn("File: lib/http.ts", feedback)
            self.assertIn("Evidence: Process timed out", feedback)


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

    def test_public_only_policy_repairs_on_public_failure_but_ignores_hidden_failure(self) -> None:
        policy = PolicyManifest(
            id="public-repair-1",
            title="Public repair x1",
            description="",
            court_jester_mode="none",
            max_repair_rounds=1,
            public_only_repair=True,
        )

        self.assertEqual(
            select_repair_trigger_source(
                policy=policy,
                verify_failed=False,
                public_ok=False,
                hidden_checks_ran=True,
                hidden_ok=False,
            ),
            "public",
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

    def test_retry_without_feedback_retries_after_verify_failure_without_repro_text(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "blind_retry_fixture"
            fixture.mkdir(parents=True, exist_ok=True)
            (fixture / "app.py").write_text("VALUE = 1\n")

            task = TaskManifest(
                id="blind-retry-task",
                title="",
                repo_fixture="blind_retry_fixture",
                prompt="Fix the behavior bug.",
                language="python",
                bucket="test",
                verify_paths=["app.py"],
                expected_files=["app.py"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="retry-once-no-feedback",
                title="Retry once (no feedback)",
                description="",
                court_jester_mode="required",
                block_on_failed_verify=True,
                max_repair_rounds=1,
                verify_only_repair=True,
                repair_feedback_style="none",
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True
                    self.feedbacks: list[str | None] = []

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    self.feedbacks.append(feedback)
                    return ProviderResult(changed_files=["app.py"], parsed_summary={"status": "completed"})

            class FakeCourtJesterClient:
                def __init__(self, responses: list[dict[str, object]]) -> None:
                    self._responses = responses
                    self._index = 0

                def __enter__(self) -> "FakeCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    response = self._responses[self._index]
                    self._index += 1
                    return response

            provider = FakeProvider()
            fake_client = FakeCourtJesterClient(
                [
                    {
                        "result": {
                            "parsed": {
                                "verdict": "fail",
                                "schema_version": 3,
                                "strength": "property_checked",
                                "summary": {},
                                "stages": [
                                    {
                                        "name": "execute",
                                        "status": "failed",
                                        "duration_ms": 1,
                                        "message": "verification finding",
                                    }
                                ],
                            }
                        }
                    },
                    {
                        "result": {
                            "parsed": {
                                "verdict": "pass",
                                "schema_version": 3,
                                "strength": "property_checked",
                                "summary": {},
                                "stages": [
                                    {"name": "execute", "status": "passed", "duration_ms": 1}
                                ],
                            }
                        }
                    },
                ]
            )

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=fake_client),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            self.assertTrue(result["success"])
            self.assertEqual(result["attempt_count"], 2)
            self.assertTrue(result["repair_attempted"])
            self.assertEqual(result["repair_trigger_source"], "verify")
            self.assertEqual(result["repair_feedback_style"], "none")
            self.assertEqual(provider.feedbacks, [None, None])
            self.assertEqual(result["repair_feedback_styles"], ["none"])
            self.assertEqual(result["attempts"][0]["repair_feedback_style"], "none")
            self.assertFalse(result["attempts"][0]["repair_feedback_present"])

    def test_retry_without_verify_skips_court_jester_and_defers_judging_to_final_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "blind_no_verify_fixture"
            fixture.mkdir(parents=True, exist_ok=True)
            (fixture / "app.py").write_text("VALUE = 1\n")
            (fixture / "tests").mkdir(exist_ok=True)
            (fixture / "tests" / "public_checks.py").write_text(
                (
                    "from pathlib import Path\n"
                    "counter = Path('public_count.txt')\n"
                    "count = int(counter.read_text()) if counter.exists() else 0\n"
                    "counter.write_text(str(count + 1))\n"
                )
            )

            task = TaskManifest(
                id="blind-no-verify-task",
                title="",
                repo_fixture="blind_no_verify_fixture",
                prompt="Fix the behavior bug.",
                language="python",
                bucket="test",
                verify_paths=["app.py"],
                public_check_commands=[["python", "tests/public_checks.py"]],
                expected_files=["app.py"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="retry-once-no-verify",
                title="Retry once (no verify)",
                description="",
                court_jester_mode="none",
                max_repair_rounds=1,
                blind_retry_without_verify=True,
                repair_feedback_style="none",
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True
                    self.feedbacks: list[str | None] = []

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    self.feedbacks.append(feedback)
                    return ProviderResult(changed_files=["app.py"], parsed_summary={"status": "completed"})

            class UnexpectedCourtJesterClient:
                def __enter__(self) -> "UnexpectedCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    raise AssertionError("Court Jester should not be called for blind no-verify retries")

            provider = FakeProvider()

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=UnexpectedCourtJesterClient()),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            self.assertTrue(result["success"])
            self.assertEqual(result["attempt_count"], 2)
            self.assertTrue(result["repair_attempted"])
            self.assertTrue(result["blind_retry_without_verify"])
            self.assertEqual(result["tool_usage"]["verify_calls"], 0)
            self.assertIsNone(result["repair_trigger_source"])
            self.assertIsNone(result["repair_feedback_style"])
            self.assertEqual(provider.feedbacks, [None, None])
            self.assertEqual(result["attempts"][0]["public_checks"], [])
            self.assertTrue(result["attempts"][0]["public_checks_deferred"])
            self.assertTrue((result["attempts"][0]["court_jester"] or {}).get("skipped"))
            self.assertEqual(len(result["attempts"][1]["public_checks"]), 1)
            self.assertEqual((tmp_path / "out" / result["run_id"] / "workspace" / "public_count.txt").read_text(), "1")

    def test_verify_test_is_materialized_from_verify_assets_and_removed_after_call(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "fixture_with_verify_assets"
            fixture.mkdir(parents=True, exist_ok=True)
            (fixture / "app.ts").write_text("export const value = 1;\n")
            (fixture / "tests").mkdir(exist_ok=True)
            verify_assets = bench_root / "verify_assets" / "fixture_with_verify_assets" / "tests"
            verify_assets.mkdir(parents=True, exist_ok=True)
            (verify_assets / "verify_app.ts").write_text(
                'import { value } from "../app.ts";\nif (value !== 1) throw new Error("bad value");\n'
            )

            task = TaskManifest(
                id="verify-assets-task",
                title="",
                repo_fixture="fixture_with_verify_assets",
                prompt="Fix the behavior bug.",
                language="typescript",
                bucket="test",
                verify_paths=["app.ts"],
                verify_test_path="tests/verify_app.ts",
                expected_files=["app.ts"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="repair-loop-verify-only",
                title="Repair loop (verify only)",
                description="",
                court_jester_mode="required",
                block_on_failed_verify=True,
                max_repair_rounds=0,
                verify_only_repair=True,
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    return ProviderResult(changed_files=["app.ts"], parsed_summary={"status": "completed"})

            class FakeCourtJesterClient:
                def __init__(self) -> None:
                    self.seen_test_path: str | None = None

                def __enter__(self) -> "FakeCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    self.seen_test_path = str(arguments["test_file_path"])
                    self_path = Path(self.seen_test_path)
                    assert self_path.exists()
                    return {
                        "result": {
                            "parsed": {
                                "verdict": "pass",
                                "schema_version": 3,
                                "strength": "authoritative_tests",
                                "summary": {},
                                "stages": [
                                    {"name": "execute", "status": "passed", "duration_ms": 1}
                                ],
                            }
                        }
                    }

            provider = FakeProvider()
            fake_client = FakeCourtJesterClient()

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=fake_client),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            self.assertTrue(result["success"])
            self.assertIsNotNone(fake_client.seen_test_path)
            self.assertFalse((tmp_path / "out" / result["run_id"] / "workspace" / "tests" / "verify_app.ts").exists())

    def test_public_repair_retries_on_public_failure_and_defers_hidden_until_final_scoring(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "public_repair_fixture"
            fixture.mkdir(parents=True, exist_ok=True)
            (fixture / "app.py").write_text("VALUE = 0\n")
            (fixture / "tests").mkdir(exist_ok=True)
            (fixture / "tests" / "public_checks.py").write_text(
                (
                    "from pathlib import Path\n"
                    "import sys\n"
                    "sys.path.insert(0, str(Path(__file__).resolve().parents[1]))\n"
                    "from app import VALUE\n"
                    "counter = Path('public_count.txt')\n"
                    "count = int(counter.read_text()) if counter.exists() else 0\n"
                    "counter.write_text(str(count + 1))\n"
                    "if VALUE != 22:\n"
                    "    print('expected VALUE=22 in public check')\n"
                    "    raise SystemExit(1)\n"
                )
            )
            hidden_script = bench_root / "evaluators" / "hidden.py"
            hidden_script.parent.mkdir(parents=True, exist_ok=True)
            hidden_script.write_text(
                (
                    "from pathlib import Path\n"
                    "import sys\n"
                    "workspace = Path(sys.argv[1])\n"
                    "counter = workspace / 'hidden_count.txt'\n"
                    "count = int(counter.read_text()) if counter.exists() else 0\n"
                    "counter.write_text(str(count + 1))\n"
                )
            )

            task = TaskManifest(
                id="public-repair-task",
                title="",
                repo_fixture="public_repair_fixture",
                prompt="Fix the public failure.",
                language="python",
                bucket="test",
                verify_paths=["app.py"],
                public_check_commands=[["python", "tests/public_checks.py"]],
                hidden_check_command=["python", "{bench_root}/evaluators/hidden.py", "{workspace}"],
                expected_files=["app.py"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="public-repair-1",
                title="Public repair x1",
                description="",
                court_jester_mode="none",
                max_repair_rounds=1,
                public_only_repair=True,
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True
                    self.feedbacks: list[str | None] = []

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    self.feedbacks.append(feedback)
                    value = "1" if attempt == 0 else "22"
                    (workspace / "app.py").write_text(f"VALUE = {value}\n")
                    return ProviderResult(changed_files=["app.py"], parsed_summary={"status": "completed"})

            class UnexpectedCourtJesterClient:
                def __enter__(self) -> "UnexpectedCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    raise AssertionError("Court Jester should not be called for public-only repair policies")

            provider = FakeProvider()

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=UnexpectedCourtJesterClient()),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            workspace = tmp_path / "out" / result["run_id"] / "workspace"
            self.assertTrue(result["success"])
            self.assertEqual(result["attempt_count"], 2)
            self.assertTrue(result["repair_attempted"])
            self.assertTrue(result["public_only_repair"])
            self.assertEqual(result["tool_usage"]["verify_calls"], 0)
            self.assertEqual(result["repair_trigger_source"], "public")
            self.assertEqual(result["repair_feedback_style"], "detailed")
            self.assertEqual((workspace / "public_count.txt").read_text(), "2")
            self.assertEqual((workspace / "hidden_count.txt").read_text(), "1")
            self.assertFalse(result["attempts"][0]["hidden_checks_ran"])
            self.assertIn("expected VALUE=22 in public check", provider.feedbacks[1] or "")
            self.assertEqual(result["attempts"][0]["attempt_changed_files"], ["app.py"])
            self.assertTrue(Path(result["attempts"][0]["attempt_patch_diff_path"]).exists())

    def test_verify_tests_only_tasks_pass_tests_only_flag_to_cli(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "fixture_with_verify_assets"
            verify_asset_dir = bench_root / "verify_assets" / "fixture_with_verify_assets" / "tests"
            fixture.mkdir(parents=True, exist_ok=True)
            verify_asset_dir.mkdir(parents=True, exist_ok=True)
            (fixture / "app.ts").write_text("export const value = 1;\n")
            (fixture / "helper.ts").write_text("export const helper = value => value;\n")
            (verify_asset_dir / "verify_app.ts").write_text(
                'import { value } from "../app.ts";\n'
                'import { helper } from "../helper.ts";\n'
                'if (helper(value) !== 1) throw new Error("bad value");\n'
            )

            task = TaskManifest(
                id="verify-tests-only-task",
                title="",
                repo_fixture="fixture_with_verify_assets",
                prompt="Fix the behavior bug.",
                language="typescript",
                bucket="test",
                verify_paths=["app.ts", "helper.ts"],
                verify_test_path="tests/verify_app.ts",
                verify_tests_only=True,
                expected_files=["app.ts", "helper.ts"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="repair-loop-verify-only",
                title="Repair loop (verify only)",
                description="",
                court_jester_mode="required",
                max_repair_rounds=0,
                verify_only_repair=True,
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    return ProviderResult(changed_files=["app.ts"], parsed_summary={"status": "completed"})

            class FakeCourtJesterClient:
                def __init__(self) -> None:
                    self.arguments: list[dict[str, object]] = []

                def __enter__(self) -> "FakeCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    self.arguments.append(dict(arguments))
                    return {
                        "result": {
                            "parsed": {
                                "verdict": "pass",
                                "schema_version": 3,
                                "strength": "authoritative_tests",
                                "summary": {},
                                "stages": [
                                    {"name": "test", "status": "passed", "duration_ms": 1}
                                ],
                            }
                        }
                    }

            provider = FakeProvider()
            fake_client = FakeCourtJesterClient()

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=fake_client),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            self.assertTrue(result["success"])
            self.assertEqual(len(fake_client.arguments), 2)
            for arguments in fake_client.arguments:
                self.assertEqual(arguments.get("tests_only"), True)
                self.assertEqual(
                    Path(str(arguments["test_file_path"])).name,
                    "verify_app.ts",
                )

    def test_verify_cli_failure_does_not_crash_matrix(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bench_root = tmp_path / "bench"
            fixture = bench_root / "repos" / "fixture_with_verify_assets"
            verify_asset_dir = bench_root / "verify_assets" / "fixture_with_verify_assets" / "tests"
            fixture.mkdir(parents=True, exist_ok=True)
            verify_asset_dir.mkdir(parents=True, exist_ok=True)
            (fixture / "app.ts").write_text("export const value = 1;\n")
            (verify_asset_dir / "verify_app.ts").write_text(
                'import { value } from "../app.ts";\nif (value !== 1) throw new Error("bad value");\n'
            )

            task = TaskManifest(
                id="verify-cli-failure-task",
                title="",
                repo_fixture="fixture_with_verify_assets",
                prompt="Fix the behavior bug.",
                language="typescript",
                bucket="test",
                verify_paths=["app.ts"],
                verify_test_path="tests/verify_app.ts",
                verify_tests_only=True,
                expected_files=["app.ts"],
            )
            model = ModelManifest(id="fake-provider", title="Fake provider", provider="fake")
            policy = PolicyManifest(
                id="repair-loop-verify-only",
                title="Repair loop (verify only)",
                description="",
                court_jester_mode="required",
                max_repair_rounds=0,
                verify_only_repair=True,
            )

            class FakeProvider:
                def __init__(self) -> None:
                    self.supports_repair = True

                def apply(
                    self,
                    workspace: Path,
                    task: TaskManifest,
                    *,
                    feedback: str | None = None,
                    attempt: int = 0,
                    history: list[dict[str, object]] | None = None,
                    env_overrides: dict[str, str] | None = None,
                ) -> ProviderResult:
                    return ProviderResult(changed_files=["app.ts"], parsed_summary={"status": "completed"})

            class FailingCourtJesterClient:
                def __enter__(self) -> "FailingCourtJesterClient":
                    return self

                def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
                    return None

                def call_tool(self, name: str, arguments: dict[str, object]) -> dict[str, object]:
                    raise RuntimeError("court-jester verify exited rc=2 stderr=error: unknown flag '--tests-only'")

            provider = FakeProvider()

            with (
                mock.patch("bench.runner.BENCH_ROOT", bench_root),
                mock.patch("bench.runner.provider_from_manifest", return_value=provider),
                mock.patch("bench.runner.CourtJesterClient", return_value=FailingCourtJesterClient()),
            ):
                result = run_single(task, model, policy, tmp_path / "out")

            self.assertEqual(result["status"], "completed")
            self.assertEqual(result["tool_usage"]["verify_calls"], 1)
            self.assertTrue(result["attempts"][0]["court_jester"]["verify_inconclusive"])
            self.assertFalse(result["attempts"][0]["court_jester"]["verify_failed"])
            tool_result = result["attempts"][0]["court_jester"]["results"][0]
            self.assertIsNone(tool_result["response"])
            self.assertEqual(tool_result["tool_error"]["kind"], "error")
            self.assertEqual(result["verifier_observation"]["outcome"], "abstain")
            self.assertEqual(result["verifier_observation"]["reason"], "verify_tool_error")


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


class RunnerArtifactContractTest(unittest.TestCase):
    def test_verifier_observation_abstains_without_a_valid_completed_v3_report(self) -> None:
        from bench.runner import verifier_observation

        valid_stage = {"name": "execute", "status": "passed", "duration_ms": 1}
        cases = [
            ("verifier_not_run", {}, "verify_not_run", None),
            ("provider_terminal", {"failure_category": "provider_timeout"}, "provider_timeout", None),
            (
                "tool_error",
                {"court_jester": {"results": [{"path": "app.py", "tool_error": {"kind": "protocol"}}]}},
                "verify_tool_error",
                "app.py",
            ),
            (
                "tool_timeout",
                {"court_jester": {"results": [{"path": "app.py", "tool_error": {"kind": "timeout"}}]}},
                "verify_tool_timeout",
                "app.py",
            ),
            (
                "report_missing",
                {"court_jester": {"results": [{"path": "app.py"}]}},
                "verify_report_missing",
                "app.py",
            ),
            (
                "schema_missing",
                {"court_jester": {"results": [{"path": "app.py", "response": {"verdict": "pass"}}]}},
                "verify_schema_missing",
                "app.py",
            ),
            (
                "schema_mismatch",
                {
                    "court_jester": {
                        "results": [
                            {
                                "path": "app.py",
                                "response": {
                                    "schema_version": 2,
                                    "verdict": "pass",
                                    "strength": "property_checked",
                                    "stages": [valid_stage],
                                },
                            }
                        ]
                    }
                },
                "verify_schema_mismatch",
                "app.py",
            ),
            (
                "schema_v3_but_invalid_report",
                {
                    "court_jester": {
                        "results": [
                            {
                                "path": "app.py",
                                "response": {
                                    "schema_version": 3,
                                    "verdict": "pass",
                                    "strength": "property_checked",
                                    "stages": [],
                                },
                            }
                        ]
                    }
                },
                "verify_report_invalid",
                "app.py",
            ),
        ]

        for name, result, expected_reason, expected_path in cases:
            with self.subTest(name=name):
                observation = verifier_observation(result)
                self.assertEqual(observation["outcome"], "abstain")
                self.assertEqual(observation["reason"], expected_reason)
                self.assertEqual(observation["failure_path"], expected_path)
        self.assertEqual(
            verifier_observation(cases[6][1])["report_schema_version"],
            2,
            "the mismatched schema must remain observable without becoming a semantic verdict",
        )

    def test_report_verdict_rejects_malformed_schema_v3_structure(self) -> None:
        from bench.runner import report_verdict, verifier_observation

        def valid_report() -> dict[str, object]:
            return {
                "schema_version": 3,
                "verdict": "pass",
                "strength": "property_checked",
                "summary": {},
                "stages": [{"name": "execute", "status": "passed", "duration_ms": 1}],
            }

        malformed_reports: list[tuple[str, dict[str, object]]] = []

        empty_stage = valid_report()
        empty_stage["stages"] = [{}]
        malformed_reports.append(("empty_stage", empty_stage))

        invalid_status = valid_report()
        invalid_status["stages"] = [
            {"name": "execute", "status": "success", "duration_ms": 1}
        ]
        malformed_reports.append(("invalid_status", invalid_status))

        bool_duration = valid_report()
        bool_duration["stages"] = [
            {"name": "execute", "status": "passed", "duration_ms": True}
        ]
        malformed_reports.append(("bool_duration", bool_duration))

        negative_duration = valid_report()
        negative_duration["stages"] = [
            {"name": "execute", "status": "passed", "duration_ms": -1}
        ]
        malformed_reports.append(("negative_duration", negative_duration))

        missing_summary = valid_report()
        del missing_summary["summary"]
        malformed_reports.append(("missing_summary", missing_summary))

        for name, report in malformed_reports:
            with self.subTest(name=name):
                self.assertIsNone(report_verdict(report))
                observation = verifier_observation(
                    {
                        "court_jester": {
                            "results": [{"path": "app.py", "response": report}]
                        }
                    }
                )
                self.assertEqual(observation["outcome"], "abstain")
                self.assertEqual(observation["reason"], "verify_report_invalid")
                self.assertEqual(observation["failure_path"], "app.py")
                self.assertEqual(observation["report_schema_version"], 3)

    def test_verifier_observation_fail_dominates_inconclusive_reports(self) -> None:
        from bench.runner import verifier_observation

        def report(verdict: str, status: str) -> dict[str, object]:
            return {
                "schema_version": 3,
                "verdict": verdict,
                "strength": "property_checked",
                "summary": {},
                "stages": [{"name": "execute", "status": status, "duration_ms": 1}],
            }

        observation = verifier_observation(
            {
                "court_jester": {
                    "results": [
                        {
                            "path": "uncertain.py",
                            "response": report("inconclusive", "inconclusive"),
                        },
                        {
                            "path": "broken.py",
                            "response": report("fail", "failed"),
                        },
                    ]
                }
            }
        )

        self.assertEqual(observation["outcome"], "fail")
        self.assertEqual(observation["reason"], "stage_failure")
        self.assertEqual(observation["failure_stage"], "execute")
        self.assertEqual(observation["failure_path"], "broken.py")
        self.assertEqual(observation["report_schema_version"], 3)


    def test_run_single_dry_run_persists_v1_metadata_and_hidden_seed_digest(self) -> None:
        task = load_task(BENCH_ROOT / "tasks" / "py-billing-country-fallback.json")
        model = load_model(BENCH_ROOT / "models" / "noop.json")
        policy = PolicyManifest(id="baseline", title="Baseline", description="", court_jester_mode="none")
        with tempfile.TemporaryDirectory() as directory:
            result = run_single(task, model, policy, Path(directory), dry_run=True, hidden_seed="secret-seed")
            self.assertEqual(result["artifact_schema_version"], 1)
            self.assertEqual(result["verify_schema_version_required"], 3)
            self.assertEqual(result["verifier_observation"]["outcome"], "abstain")
            self.assertNotIn("secret-seed", json.dumps(result, sort_keys=True))
            result_path = next(Path(directory).glob("*/result.json"))
            run_path = next(Path(directory).glob("*/run.json"))
            self.assertEqual(json.loads(result_path.read_text())["hidden_seed_sha256"], result["hidden_seed_sha256"])
            self.assertEqual(json.loads(run_path.read_text())["verify_schema_version_required"], 3)

    def test_shadow_record_requires_existing_parent_and_emits_non_blocking_record(self) -> None:
        from bench.runner import append_shadow_record

        result = {"run_id": "run-1", "attempt_count": 1, "verify_path": "app.py", "dry_run": True}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "shadow.jsonl"
            append_shadow_record(path, result, patch_digest="patch")
            record = json.loads(path.read_text().strip())
            self.assertEqual(record["artifact_schema_version"], 1)
            self.assertEqual(record["verify_schema_version_required"], 3)
            self.assertEqual(record["blocking_mode"], "shadow")
            self.assertEqual(record["verifier_observation"]["outcome"], "abstain")
        with self.assertRaises(FileNotFoundError):
            append_shadow_record(Path(directory) / "missing" / "shadow.jsonl", result)

    def test_early_terminal_runs_append_exactly_one_shadow_record(self) -> None:
        class FailedProvider:
            supports_repair = False

            def apply(
                self,
                workspace: Path,
                task: TaskManifest,
                *,
                feedback: str | None = None,
                attempt: int = 0,
                history: list[dict[str, object]] | None = None,
                env_overrides: dict[str, str] | None = None,
            ) -> ProviderResult:
                return ProviderResult(failed=True, failure_reason="provider unavailable")

        cases = [
            ("setup", False, False, "setup_error"),
            ("provider", True, False, "provider_error"),
            ("gold", True, True, "gold_patch_apply_error"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bench_root = root / "bench"
            (bench_root / "repos").mkdir(parents=True)
            for name, fixture_exists, use_gold, expected_reason in cases:
                with self.subTest(path=name):
                    fixture_name = f"fixture-{name}"
                    if fixture_exists:
                        fixture = bench_root / "repos" / fixture_name
                        fixture.mkdir()
                        (fixture / "app.py").write_text("VALUE = 1\n", encoding="utf-8")
                    task = TaskManifest(
                        id=f"terminal-{name}",
                        title="",
                        repo_fixture=fixture_name,
                        prompt="",
                        language="python",
                        bucket="test",
                        verify_paths=["app.py"],
                    )
                    model = ModelManifest(id="fake", title="", provider="fake")
                    policy = PolicyManifest(id="baseline", title="", description="", court_jester_mode="none")
                    shadow_path = root / f"shadow-{name}.jsonl"
                    with (
                        mock.patch("bench.runner.BENCH_ROOT", bench_root),
                        mock.patch("bench.runner.provider_from_manifest", return_value=FailedProvider()),
                    ):
                        result = run_single(
                            task,
                            model,
                            policy,
                            root / f"output-{name}",
                            use_task_gold_patches=use_gold,
                            shadow_records=shadow_path,
                        )

                    records = [json.loads(line) for line in shadow_path.read_text(encoding="utf-8").splitlines()]
                    self.assertEqual(len(records), 1)
                    self.assertEqual(records[0]["run_id"], result["run_id"])
                    self.assertEqual(records[0]["verifier_observation"]["outcome"], "abstain")
                    self.assertEqual(records[0]["verifier_observation"]["reason"], expected_reason)
                    persisted = list((root / f"output-{name}").glob("*/result.json"))
                    self.assertEqual(len(persisted), 1)
if __name__ == "__main__":
    unittest.main()
