import tempfile
import unittest
from pathlib import Path

from bench.materialize_mutation import materialize_mutation


class MaterializeMutationTest(unittest.TestCase):
    def test_materialize_mutation_copies_source_into_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            workspace = root / "workspace"
            workspace.mkdir()
            source = root / "source.py"
            source.write_text("value = 1\n")

            target = materialize_mutation(workspace, "pkg/module.py", source)

            self.assertEqual(target.resolve(), (workspace / "pkg" / "module.py").resolve())
            self.assertEqual(target.read_text(), "value = 1\n")

    def test_materialize_mutation_rejects_workspace_escape(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            workspace = root / "workspace"
            workspace.mkdir()
            source = root / "source.py"
            source.write_text("value = 1\n")

            with self.assertRaises(ValueError):
                materialize_mutation(workspace, "../escape.py", source)


if __name__ == "__main__":
    unittest.main()
