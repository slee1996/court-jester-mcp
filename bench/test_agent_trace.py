import subprocess
import tempfile
import unittest
from pathlib import Path

from bench.agent_trace import prepare_agent_trace, summarize_agent_trace, temporary_environment


class AgentTraceTest(unittest.TestCase):
    def test_trace_shim_records_shell_and_file_read_commands(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "sample.txt").write_text("hello\n")
            setup = prepare_agent_trace(tmp_path / "trace")

            with temporary_environment(setup.env_updates):
                completed = subprocess.run(
                    ["bash", "-lc", "cat sample.txt >/dev/null"],
                    cwd=tmp_path,
                    capture_output=True,
                    text=True,
                    check=False,
                )

            self.assertEqual(completed.returncode, 0)
            summary = summarize_agent_trace(Path(setup.trace_dir))
            self.assertGreaterEqual(summary["event_count"], 2)
            self.assertIn("bash", summary["commands"])
            self.assertIn("cat", summary["commands"])
            self.assertTrue(any(item.get("shell_command") == "cat sample.txt >/dev/null" for item in summary["tail"]))
            self.assertTrue(any("sample.txt" in (item.get("path_hints") or []) for item in summary["tail"]))
