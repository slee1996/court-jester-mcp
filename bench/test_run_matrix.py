import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path
from unittest import mock

from bench.common import ModelManifest, PolicyManifest, TaskManifest
from bench.run_matrix import build_run_plan, main, partition_plan_by_provider, run_parallel_provider_plan


class RunMatrixSchedulingTest(unittest.TestCase):
    def make_task(self, task_id: str) -> TaskManifest:
        return TaskManifest(
            id=task_id,
            title=task_id,
            repo_fixture="fixture",
            prompt="",
            language="python",
            bucket="test",
            verify_paths=["app.py"],
        )

    def make_model(self, model_id: str, provider: str = "fake") -> ModelManifest:
        return ModelManifest(id=model_id, title=model_id, provider=provider)

    def make_policy(self, policy_id: str) -> PolicyManifest:
        return PolicyManifest(
            id=policy_id,
            title=policy_id,
            description="",
            court_jester_mode="none",
        )

    def cell_signature(self, cell: dict[str, object]) -> tuple[str, str, str, int, str]:
        task = cell["task"]
        model = cell["model"]
        policy = cell["policy"]
        return (
            task.id,
            model.id,
            policy.id,
            int(cell["repeat_index"]),
            str(cell["hidden_seed"]),
        )

    def test_task_major_schedule_preserves_nested_loop_order(self) -> None:
        tasks = [self.make_task("task-a"), self.make_task("task-b")]
        models = [self.make_model("model-1"), self.make_model("model-2")]
        policies = [self.make_policy("baseline"), self.make_policy("repair")]

        plan = build_run_plan(
            tasks,
            models,
            policies,
            repeats=1,
            schedule="task-major",
            shuffle_seed=7,
        )

        self.assertEqual(
            [self.cell_signature(cell)[:4] for cell in plan],
            [
                ("task-a", "model-1", "baseline", 0),
                ("task-a", "model-1", "repair", 0),
                ("task-a", "model-2", "baseline", 0),
                ("task-a", "model-2", "repair", 0),
                ("task-b", "model-1", "baseline", 0),
                ("task-b", "model-1", "repair", 0),
                ("task-b", "model-2", "baseline", 0),
                ("task-b", "model-2", "repair", 0),
            ],
        )

    def test_blocked_random_schedule_is_deterministic_and_keeps_task_repeat_blocks_together(self) -> None:
        tasks = [self.make_task("task-a"), self.make_task("task-b")]
        models = [self.make_model("model-1"), self.make_model("model-2")]
        policies = [self.make_policy("baseline"), self.make_policy("repair")]

        plan_one = build_run_plan(
            tasks,
            models,
            policies,
            repeats=2,
            schedule="blocked-random",
            shuffle_seed=11,
        )
        plan_two = build_run_plan(
            tasks,
            models,
            policies,
            repeats=2,
            schedule="blocked-random",
            shuffle_seed=11,
        )

        self.assertEqual(
            [self.cell_signature(cell) for cell in plan_one],
            [self.cell_signature(cell) for cell in plan_two],
        )

        block_size = len(models) * len(policies)
        for index in range(0, len(plan_one), block_size):
            block = plan_one[index:index + block_size]
            self.assertEqual(len({cell["task"].id for cell in block}), 1)
            self.assertEqual(len({int(cell["repeat_index"]) for cell in block}), 1)
            self.assertEqual(
                len({str(cell["hidden_seed"]) for cell in block if cell["model"].id == models[0].id}),
                1,
            )
            self.assertEqual(
                len({str(cell["hidden_seed"]) for cell in block if cell["model"].id == models[1].id}),
                1,
            )

    def test_partition_plan_by_provider_preserves_relative_order_within_provider(self) -> None:
        tasks = [self.make_task("task-a")]
        models = [
            self.make_model("codex-a", provider="codex_cli"),
            self.make_model("claude-a", provider="claude_cli"),
            self.make_model("codex-b", provider="codex_cli"),
        ]
        policies = [self.make_policy("baseline"), self.make_policy("repair")]

        plan = build_run_plan(
            tasks,
            models,
            policies,
            repeats=1,
            schedule="task-major",
            shuffle_seed=7,
        )
        queues = partition_plan_by_provider(plan)

        self.assertEqual(list(queues.keys()), ["codex_cli", "claude_cli"])
        self.assertEqual(
            [self.cell_signature(cell)[:3] for cell in queues["codex_cli"]],
            [
                ("task-a", "codex-a", "baseline"),
                ("task-a", "codex-a", "repair"),
                ("task-a", "codex-b", "baseline"),
                ("task-a", "codex-b", "repair"),
            ],
        )
        self.assertEqual(
            [self.cell_signature(cell)[:3] for cell in queues["claude_cli"]],
            [
                ("task-a", "claude-a", "baseline"),
                ("task-a", "claude-a", "repair"),
            ],
        )

    def test_parallel_plan_terminates_active_providers_on_keyboard_interrupt(self) -> None:
        plan = [
            {
                "task": self.make_task("task-a"),
                "model": self.make_model("codex-a", provider="codex_cli"),
                "policy": self.make_policy("baseline"),
                "repeat_index": 0,
                "hidden_seed": "seed-a",
            },
            {
                "task": self.make_task("task-b"),
                "model": self.make_model("claude-a", provider="claude_cli"),
                "policy": self.make_policy("baseline"),
                "repeat_index": 0,
                "hidden_seed": "seed-b",
            },
        ]

        class InterruptingFuture:
            def result(self) -> int:
                raise KeyboardInterrupt

        class FakeExecutor:
            def __init__(self, *args, **kwargs) -> None:
                self.shutdown_calls: list[tuple[bool, bool]] = []

            def __enter__(self) -> "FakeExecutor":
                return self

            def __exit__(self, exc_type, exc, tb) -> bool:
                return False

            def submit(self, fn, cells):
                return InterruptingFuture()

            def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
                self.shutdown_calls.append((wait, cancel_futures))

        with mock.patch("bench.run_matrix.ThreadPoolExecutor", return_value=FakeExecutor()) as mocked_executor:
            with mock.patch("bench.run_matrix.terminate_active_provider_processes") as mocked_cleanup:
                with self.assertRaises(KeyboardInterrupt):
                    run_parallel_provider_plan(
                        plan,
                        output_dir=mock.Mock(),
                        dry_run=True,
                        repeats=1,
                        use_task_gold_patches=False,
                    )

        mocked_cleanup.assert_called_once()
        executor = mocked_executor.return_value
        self.assertEqual(executor.shutdown_calls, [(False, True)])


    def test_hidden_seed_is_shared_by_policy_cells_but_separated_by_model_and_repeat(self) -> None:
        tasks = [self.make_task("task-a")]
        models = [self.make_model("model-1"), self.make_model("model-2")]
        policies = [self.make_policy("baseline"), self.make_policy("repair")]
        plan = build_run_plan(tasks, models, policies, repeats=2, schedule="task-major", shuffle_seed=0)
        by_key = {(cell["model"].id, int(cell["repeat_index"],)): str(cell["hidden_seed"]) for cell in plan}
        self.assertNotEqual(by_key[("model-1", 0)], by_key[("model-1", 1)])
        self.assertNotEqual(by_key[("model-1", 0)], by_key[("model-2", 0)])
        for model in models:
            for repeat in range(2):
                seeds = {str(cell["hidden_seed"]) for cell in plan if cell["model"].id == model.id and int(cell["repeat_index"]) == repeat}
                self.assertEqual(len(seeds), 1, "all policies in a pair must share the hidden seed")


class RunMatrixOutputContractTest(unittest.TestCase):
    def test_custom_summary_cannot_overwrite_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            summary = root / "old-summary.json"
            summary.write_text("preserve this summary", encoding="utf-8")
            with (
                mock.patch.object(sys, "argv", self.matrix_argv(root / "new", "--dry-run", "--summary-json", str(summary))),
                mock.patch("bench.run_matrix.run_serial_plan") as execute,
            ):
                with self.assertRaisesRegex(SystemExit, "cannot claim matrix summary"):
                    main()
            execute.assert_not_called()
            self.assertEqual(summary.read_text(), "preserve this summary")

    def test_default_runs_have_independent_evidence_directories(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Keep a historical result alongside current matrices.
            historical = root / "dev"
            historical.mkdir()
            (historical / "result.json").write_text("legacy evidence", encoding="utf-8")
            argv = self.matrix_argv(root, "--dry-run")
            output_flag = argv.index("--output-dir")
            del argv[output_flag:output_flag + 2]
            with (
                mock.patch.object(sys, "argv", argv),
                mock.patch("bench.run_matrix.DEFAULT_OUTPUT_ROOT", root),
                redirect_stdout(StringIO()),
            ):
                self.assertEqual(main(), 0)
                first = next(root.glob("matrix-*"))
                original = {path.relative_to(first): path.read_bytes() for path in first.rglob("*") if path.is_file()}
                self.assertEqual(main(), 0)
            runs = list(root.glob("matrix-*"))
            self.assertEqual(len(runs), 2)
            self.assertEqual(original, {path.relative_to(first): path.read_bytes() for path in first.rglob("*") if path.is_file()})
            self.assertEqual((historical / "result.json").read_text(), "legacy evidence")
            for run in runs:
                self.assertEqual(len(list(run.glob("*/result.json"))), 1)
                self.assertTrue((run / "summary.json").is_file())

    def test_concurrent_matrix_processes_cannot_share_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for initially_empty in (False, True):
                output = Path(directory) / str(initially_empty)
                if initially_empty:
                    output.mkdir()
                command = [sys.executable, "-m", "bench.run_matrix", *self.matrix_argv(output, "--dry-run")[1:]]
                processes = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
                messages = [process.communicate(timeout=30) for process in processes]
                self.assertEqual(sorted(process.returncode for process in processes), [0, 1], messages)
                self.assertEqual(len(list(output.glob("*/result.json"))), 1)
                self.assertEqual(json.loads((output / "matrix.json").read_text())["expected_total"], 1)
                self.assertTrue((output / "summary.json").is_file())

    def test_existing_output_is_rejected_before_execution_without_changing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            previous = output / "matrix.json"
            previous.write_text('{"historical": true}\n', encoding="utf-8")
            with (
                mock.patch.object(sys, "argv", self.matrix_argv(output, "--dry-run")),
                mock.patch("bench.run_matrix.run_serial_plan", return_value=(0, 0)) as execute,
                mock.patch("bench.summarize_runs.build_summary", return_value={}),
            ):
                with self.assertRaisesRegex(SystemExit, "output directory.*not empty"):
                    main()
            execute.assert_not_called()
            self.assertEqual(previous.read_text(), '{"historical": true}\n')
            self.assertEqual(list(output.iterdir()), [previous])

    def matrix_argv(self, output_dir: Path, *extra: str) -> list[str]:
        return [
            "run_matrix",
            "--tasks",
            "py-billing-country-fallback",
            "--models",
            "noop",
            "--policies",
            "baseline",
            "--output-dir",
            str(output_dir),
            *extra,
        ]

    def test_default_summary_is_written_beside_matrix_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "matrix-output"
            stdout = StringIO()
            with mock.patch.object(sys, "argv", self.matrix_argv(output, "--dry-run")):
                with redirect_stdout(stdout):
                    exit_code = main()

            summary = json.loads((output / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(exit_code, 0)
            self.assertEqual(summary["artifact_schema_version"], 1)
            self.assertEqual(summary["verify_schema_version_required"], 3)
            self.assertIn("matrix complete: 1 runs, 0 succeeded", stdout.getvalue())
    def test_verify_policy_flags_are_persisted_in_matrix_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "matrix-output"
            with mock.patch.object(
                sys,
                "argv",
                self.matrix_argv(
                    output,
                    "--dry-run",
                    "--verify-memory-mb",
                    "96",
                    "--verify-network",
                    "allow",
                ),
            ):
                self.assertEqual(main(), 0)

            metadata = json.loads((output / "matrix.json").read_text(encoding="utf-8"))
            self.assertEqual(metadata["verify_memory_mb"], 96)
            self.assertEqual(metadata["verify_network"], "allow")
            self.assertEqual(metadata["verification_policy"]["memory_mb"], 96)
            self.assertEqual(metadata["verification_policy"]["network_policy"], "allow")


    def test_summary_failure_writes_failure_artifact_and_returns_nonzero(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "matrix-output"
            stderr = StringIO()
            stdout = StringIO()
            with (
                mock.patch.object(sys, "argv", self.matrix_argv(output, "--dry-run")),
                mock.patch("bench.summarize_runs.build_summary", side_effect=RuntimeError("summary exploded")),
                redirect_stderr(stderr),
                redirect_stdout(stdout),
            ):
                exit_code = main()

            summary = json.loads((output / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(exit_code, 1)
            self.assertEqual(summary["gate"]["failures"], ["summary_error:RuntimeError:summary exploded"])
            self.assertIn("failed to summarize benchmark matrix: summary exploded", stderr.getvalue())

    def test_invalid_or_stale_doctor_is_rejected_before_output_creation(self) -> None:
        valid_report = {
            "schema_version": 3,
            "verdict": "pass",
            "runtime_profile": "local-trusted",
            "checks": [
                {
                    "name": "runtime",
                    "language": "python",
                    "status": "passed",
                    "detail": {"version": "3.12.0"},
                }
            ],
        }
        cases = [
            ("schema_must_be_exact_integer_v3", {**valid_report, "schema_version": 3.0}, False, "schema_version must be exactly 3"),
            (
                "selected_runtime_must_be_ready",
                {**valid_report, "checks": [{**valid_report["checks"][0], "status": "advisory"}]},
                False,
                "lacks passed runtime readiness for python",
            ),
            ("doctor_must_be_fresh", valid_report, True, "doctor report is stale"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, report, stale, expected_message in cases:
                with self.subTest(name=name):
                    doctor = root / f"{name}.json"
                    doctor.write_text(json.dumps(report), encoding="utf-8")
                    if stale:
                        expired = time.time() - 3601
                        os.utime(doctor, (expired, expired))
                    output = root / f"output-{name}"
                    argv = self.matrix_argv(output, "--doctor-report", str(doctor))
                    with mock.patch.object(sys, "argv", argv):
                        with self.assertRaises(SystemExit) as error:
                            main()
                    self.assertIn(expected_message, str(error.exception))
                    self.assertFalse(output.exists(), "readiness rejection must precede matrix output creation")
if __name__ == "__main__":
    unittest.main()
