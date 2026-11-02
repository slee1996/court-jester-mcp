"""Small, subprocess-free regressions for benchmark workspace ownership."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bench import runner
from bench.common import ModelManifest, PolicyManifest, TaskManifest
from bench.providers import ProviderResult


class OutputRetentionTest(unittest.TestCase):
    def make_fixture(self, root: Path) -> tuple[Path, TaskManifest]:
        bench_root = root / "bench"
        fixture = bench_root / "repos" / "tiny"
        fixture.mkdir(parents=True)
        (fixture / "app.py").write_text("VALUE = 1\n")
        task = TaskManifest(
            id="tiny", title="Tiny", repo_fixture="tiny", prompt="",
            language="python", bucket="test", verify_paths=["app.py"],
        )
        return bench_root, task

    def model(self) -> ModelManifest:
        return ModelManifest(id="fake", title="Fake", provider="codex_cli")

    def policy(self) -> PolicyManifest:
        return PolicyManifest(
            id="baseline", title="Baseline", description="", court_jester_mode="none",
        )

    def run_with_provider(self, root: Path, apply, *, retries: int = 1):
        bench_root, task = self.make_fixture(root)
        provider = mock.Mock(supports_repair=False)
        provider.apply.side_effect = apply
        snapshots = []
        snapshot = runner.snapshot_workspace_for_retry

        def record_snapshot(workspace: Path, backup: Path) -> None:
            snapshot(workspace, backup)
            snapshots.append(backup)

        with (
            mock.patch.object(runner, "BENCH_ROOT", bench_root),
            mock.patch.object(runner, "provider_from_manifest", return_value=provider),
            mock.patch.object(runner, "provider_retry_limit", return_value=retries),
            mock.patch.object(runner, "provider_retry_delay_seconds", return_value=0),
            mock.patch.object(runner, "supports_agent_path_trace", return_value=False),
            mock.patch.object(runner, "run_commands", return_value=[]),
            mock.patch.object(runner, "snapshot_workspace_for_retry", side_effect=record_snapshot),
        ):
            result = runner.run_single(task, self.model(), self.policy(), root / "out")
        return result, provider, snapshots

    def test_dry_run_keeps_metadata_without_copying_or_running_work(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bench_root, task = self.make_fixture(root)
            with (
                mock.patch.object(runner, "BENCH_ROOT", bench_root),
                mock.patch.object(runner.shutil, "copytree") as copy,
                mock.patch.object(runner, "prepare_workspace_for_run") as setup,
                mock.patch.object(runner, "provider_from_manifest") as provider,
            ):
                result = runner.run_single(
                    task, self.model(), self.policy(), root / "out", dry_run=True,
                )
            run_dir = root / "out" / result["run_id"]
            self.assertEqual(result["status"], "dry_run")
            self.assertEqual(json.loads((run_dir / "result.json").read_text())["status"], "dry_run")
            self.assertTrue((run_dir / "run.json").is_file())
            self.assertFalse((run_dir / "workspace").exists())
            copy.assert_not_called()
            setup.assert_not_called()
            provider.assert_not_called()

    def test_dry_run_still_reports_missing_fixture_without_copying(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bench_root, task = self.make_fixture(root)
            task.repo_fixture = "missing"
            with (
                mock.patch.object(runner, "BENCH_ROOT", bench_root),
                mock.patch.object(runner.shutil, "copytree") as copy,
            ):
                result = runner.run_single(
                    task, self.model(), self.policy(), root / "out", dry_run=True,
                )
            self.assertEqual(result["status"], "setup_error")
            self.assertFalse((root / "out" / result["run_id"] / "workspace").exists())
            copy.assert_not_called()

    def test_clean_success_removes_workspace_and_retry_snapshot_but_keeps_diff(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def apply(workspace, task, **kwargs):
                (workspace / "app.py").write_text("VALUE = 2\n")
                return ProviderResult(changed_files=["app.py"])

            result, provider, snapshots = self.run_with_provider(root, apply)
            run_dir = root / "out" / result["run_id"]
            self.assertTrue(result["success"])
            self.assertEqual(provider.apply.call_count, 1)
            self.assertFalse((run_dir / "workspace").exists())
            self.assertEqual(len(snapshots), 1)
            self.assertFalse(snapshots[0].exists())
            self.assertFalse(any(path.name.startswith(".") for path in run_dir.iterdir()))
            self.assertIn("+VALUE = 2", (run_dir / "patch.diff").read_text())
            self.assertTrue((run_dir / "attempt_0.diff").is_file())
            self.assertTrue((run_dir / "result.json").is_file())
            self.assertEqual((root / "bench/repos/tiny/app.py").read_text(), "VALUE = 1\n")

    def test_transient_retry_restores_snapshot_then_discards_it(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            calls = []

            def apply(workspace, task, **kwargs):
                calls.append((workspace / "app.py").read_text())
                if len(calls) == 1:
                    (workspace / "app.py").write_text("PARTIAL = True\n")
                    return ProviderResult(failed=True, failure_reason="Transport channel closed")
                (workspace / "app.py").write_text("VALUE = 2\n")
                return ProviderResult(changed_files=["app.py"])

            result, provider, snapshots = self.run_with_provider(root, apply)
            self.assertTrue(result["success"])
            self.assertEqual(provider.apply.call_count, 2)
            self.assertEqual(calls, ["VALUE = 1\n", "VALUE = 1\n"])
            self.assertEqual(len(result["attempts"][0]["provider_retries"]), 1)
            self.assertTrue(snapshots)
            self.assertTrue(all(not snapshot.exists() for snapshot in snapshots))

    def test_provider_exception_keeps_workspace_but_discards_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def apply(workspace, task, **kwargs):
                (workspace / "app.py").write_text("PARTIAL = True\n")
                raise RuntimeError("provider crashed")

            result, provider, snapshots = self.run_with_provider(root, apply)
            run_dir = root / "out" / result["run_id"]
            self.assertEqual(result["status"], "provider_error")
            self.assertEqual((run_dir / "workspace/app.py").read_text(), "PARTIAL = True\n")
            self.assertEqual(len(snapshots), 1)
            self.assertFalse(snapshots[0].exists())
            self.assertTrue((run_dir / "result.json").is_file())

    def test_interrupt_discards_retry_snapshot_and_preserves_partial_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bench_root, task = self.make_fixture(root)
            provider = mock.Mock(supports_repair=False)
            snapshots = []
            snapshot = runner.snapshot_workspace_for_retry

            def record_snapshot(workspace, backup):
                snapshot(workspace, backup)
                snapshots.append(backup)

            def interrupt(workspace, task, **kwargs):
                (workspace / "app.py").write_text("PARTIAL = True\n")
                raise KeyboardInterrupt

            provider.apply.side_effect = interrupt
            with (
                mock.patch.object(runner, "BENCH_ROOT", bench_root),
                mock.patch.object(runner, "provider_from_manifest", return_value=provider),
                mock.patch.object(runner, "provider_retry_limit", return_value=1),
                mock.patch.object(runner, "supports_agent_path_trace", return_value=False),
                mock.patch.object(runner, "snapshot_workspace_for_retry", side_effect=record_snapshot),
            ):
                with self.assertRaises(KeyboardInterrupt):
                    runner.run_single(task, self.model(), self.policy(), root / "out")
            run_dir = next((root / "out").iterdir())
            self.assertEqual((run_dir / "workspace/app.py").read_text(), "PARTIAL = True\n")
            self.assertEqual(len(snapshots), 1)
            self.assertFalse(snapshots[0].exists())
            self.assertFalse(any(path.name.startswith(".") for path in run_dir.iterdir()))


class TerminalWorkspaceRetentionTest(unittest.TestCase):
    def make_run(self, root: Path):
        run_dir = root / "current"
        (run_dir / "workspace").mkdir(parents=True)
        (run_dir / "workspace/repro.py").write_text("raise ValueError('evidence')\n")
        for name in ("report.json", "stdout.log", "patch.diff", "attempt_0.diff"):
            (run_dir / name).write_text("evidence\n")
        result = {"status": "completed", "success": True, "timestamps": {}, "attempts": []}
        return run_dir, result

    def report(self, verdict="pass", findings=None):
        return {
            "schema_version": 3, "verdict": verdict, "strength": "property_checked",
            "summary": {}, "stages": [{
                "name": "execute", "status": "advisory" if findings else "passed",
                "duration_ms": 1, "detail": {"findings": findings or []},
            }],
        }

    def test_passing_workspace_cleanup_keeps_evidence_and_sibling_history(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_dir, result = self.make_run(root)
            history = root / "historical/workspace"
            history.mkdir(parents=True)
            (history / "repro.py").write_text("historical evidence\n")
            report = self.report()
            report["stages"].append({"name": "lint", "status": "passed", "duration_ms": 1, "detail": None})
            result["attempts"] = [{"court_jester": {"results": [{"response": report}]}}]
            shadow = root / "shadow.jsonl"
            runner.finish_run_result(run_dir, result, shadow)
            self.assertFalse((run_dir / "workspace").exists())
            self.assertEqual((history / "repro.py").read_text(), "historical evidence\n")
            for name in ("report.json", "stdout.log", "patch.diff", "attempt_0.diff", "result.json"):
                self.assertTrue((run_dir / name).is_file(), name)
            self.assertEqual(len(shadow.read_text().splitlines()), 1)

    def test_failures_inconclusive_and_prior_failure_evidence_keep_workspace(self) -> None:
        cases = [
            {"success": False},
            {"status": "provider_error"},
            {"status": "setup_error"},
            {"status": "inconclusive"},
            {"verify_failed": True},
            {"verify_inconclusive": True},
            {"attempts": [{"public_failed": True}, {}]},
            {"attempts": [{"hidden_failed": True}, {}]},
            {"attempts": [{"court_jester": {"verify_failed": True}}, {}]},
            {"attempts": [{"court_jester": {"verify_inconclusive": True}}, {}]},
            {"attempts": [{"court_jester": {"results": [{"response": self.report("inconclusive")}]}}]},
            {"attempts": [{"court_jester": {"results": [{"response": self.report(findings=[{"suppressed": True, "message": "repro"}])}]}}]},
            {"attempts": [{"court_jester": {"results": [{"tool_error": {"kind": "timeout"}}]}}]},
        ]
        for index, overrides in enumerate(cases):
            with self.subTest(case=overrides), tempfile.TemporaryDirectory() as directory:
                run_dir, result = self.make_run(Path(directory))
                result.update(overrides)
                runner.finish_run_result(run_dir, result, None)
                self.assertTrue((run_dir / "workspace/repro.py").is_file(), index)
                self.assertTrue((run_dir / "result.json").is_file())

    def test_evidence_write_failure_leaves_workspace_intact(self) -> None:
        for operation in ("write_json", "append_shadow_record"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                run_dir, result = self.make_run(Path(directory))
                with mock.patch.object(runner, operation, side_effect=OSError("evidence unavailable")):
                    with self.assertRaisesRegex(OSError, "evidence unavailable"):
                        runner.finish_run_result(run_dir, result, None)
                self.assertTrue((run_dir / "workspace/repro.py").is_file())

    def test_suppressed_findings_in_report_detail_keep_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir, result = self.make_run(Path(directory))
            report = self.report()
            report["stages"][0]["detail"]["suppressed_findings"] = [
                {"severity": "crash", "message": "suppressed repro"},
            ]
            result["attempts"] = [{"court_jester": {"results": [{"response": report}]}}]
            runner.finish_run_result(run_dir, result, None)
            self.assertTrue((run_dir / "workspace/repro.py").is_file())

    def test_cleanup_failure_is_recorded_without_changing_success(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir, result = self.make_run(Path(directory))
            with mock.patch.object(runner.shutil, "rmtree", side_effect=OSError("cleanup denied")):
                runner.finish_run_result(run_dir, result, None)
            self.assertTrue(result["success"])
            self.assertEqual(result["status"], "completed")
            self.assertIn("cleanup denied", result["workspace_cleanup_error"])
            self.assertEqual(json.loads((run_dir / "result.json").read_text())["workspace_cleanup_error"], result["workspace_cleanup_error"])
            self.assertTrue((run_dir / "workspace/repro.py").is_file())

    def test_workspace_symlink_does_not_delete_sibling_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_dir, result = self.make_run(root)
            workspace = run_dir / "workspace"
            sibling = root / "sibling"
            workspace.rename(sibling)
            workspace.symlink_to(sibling, target_is_directory=True)
            runner.finish_run_result(run_dir, result, None)
            self.assertEqual((sibling / "repro.py").read_text(), "raise ValueError('evidence')\n")
            self.assertTrue(workspace.is_symlink())
            self.assertIn("workspace_cleanup_error", result)


if __name__ == "__main__":
    unittest.main()
