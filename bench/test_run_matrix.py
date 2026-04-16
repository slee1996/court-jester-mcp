import unittest
from unittest import mock

from bench.common import ModelManifest, PolicyManifest, TaskManifest
from bench.run_matrix import build_run_plan, partition_plan_by_provider, run_parallel_provider_plan


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
            self.assertEqual(len({str(cell["hidden_seed"]) for cell in block}), 1)

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


if __name__ == "__main__":
    unittest.main()
