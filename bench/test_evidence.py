import json
import tempfile
import unittest
from pathlib import Path

from bench.evidence import EvidenceBuildError, build_evidence_bundle


class EvidenceBundleTest(unittest.TestCase):
    def write_json(self, path: Path, value: dict) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    def valid_artifact(self, **extra: object) -> dict:
        return {
            "artifact_schema_version": 1,
            "verify_schema_version_required": 3,
            **extra,
        }

    def test_strict_bundle_validates_versions_and_excludes_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name)
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", self.valid_artifact(rows=1))
            self.write_json(root / "cell" / "result.json", self.valid_artifact(success=True))
            (root / "cell" / "change.diff").write_text("- before\n+ after\n", encoding="utf-8")
            (root / "workspace").mkdir()
            (root / "workspace" / "secret.txt").write_text("must not ship", encoding="utf-8")
            output = root / "bundle"

            manifest = build_evidence_bundle(root, output, redaction="transcripts", strict=True)

            self.assertEqual(manifest["artifact_schema_version"], 1)
            self.assertEqual(manifest["verify_schema_version_required"], 3)
            self.assertTrue(manifest["manifest_hash"])
            paths = {entry["path"] for entry in manifest["entries"]}
            self.assertIn("matrix.json", paths)
            self.assertIn("cell/change.diff", paths)
            self.assertNotIn("workspace/secret.txt", paths)
            self.assertTrue(all(not Path(path).is_absolute() for path in paths))
            self.assertTrue((output / "manifest.json").exists())

    def test_redaction_modes_remove_transcripts_and_all_text(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name)
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", self.valid_artifact(rows=1))
            self.write_json(root / "result.json", self.valid_artifact(prompt="secret", response="secret", stdout="secret", stderr="secret", diff="keep"))
            (root / "fix.diff").write_text("diff text", encoding="utf-8")

            transcript = root / "transcripts" / "events.json"
            transcript.parent.mkdir()
            self.write_json(transcript, {"prompt": "secret", "response": "secret", "stdout": "secret", "structured": "keep"})
            output = root / "evidence"
            manifest = build_evidence_bundle(root, output, redaction="transcripts", strict=True)
            result = json.loads((output / "result.json").read_text(encoding="utf-8"))
            self.assertNotIn("prompt", result)
            self.assertNotIn("response", result)
            self.assertIn("diff", result)
            self.assertFalse((output / "transcripts" / "events.json").exists())
            self.assertTrue(any(entry["path"] == "fix.diff" for entry in manifest["entries"]))

            all_text = root / "all-text"
            all_manifest = build_evidence_bundle(root, all_text, redaction="all-text", strict=True)
            all_paths = {entry["path"] for entry in all_manifest["entries"]}
            self.assertNotIn("fix.diff", all_paths)
            all_result = json.loads((all_text / "result.json").read_text(encoding="utf-8"))
            self.assertNotIn("diff", all_result)
            self.assertIn("artifact_schema_version", all_result)

    def test_strict_bundle_rejects_missing_or_legacy_required_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name)
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", {"artifact_schema_version": 1, "verify_schema_version_required": 2})
            self.write_json(root / "result.json", self.valid_artifact())
            with self.assertRaises(EvidenceBuildError) as error:
                build_evidence_bundle(root, root / "bundle", strict=True)
            self.assertTrue(any("summary.json" in message for message in error.exception.errors))

    def test_bundle_destination_is_not_recursed_into_and_manifest_entries_are_checksummed(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name)
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", self.valid_artifact(rows=1))
            self.write_json(root / "result.json", self.valid_artifact())
            output = root / "evidence"
            first = build_evidence_bundle(root, output, strict=True)
            second = build_evidence_bundle(root, output, strict=True)
            self.assertEqual(first["manifest_hash"], second["manifest_hash"])
            self.assertNotIn("evidence/manifest.json", {entry["path"] for entry in second["entries"]})


    def test_transcript_redaction_excludes_dot_stdout_and_stderr_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name)
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", self.valid_artifact(rows=1))
            self.write_json(root / "result.json", self.valid_artifact(success=False))
            (root / "cell").mkdir()
            (root / "cell" / "public_check_0.stdout.txt").write_text("secret public output", encoding="utf-8")
            (root / "cell" / "hidden_check_0.stderr.txt").write_text("secret hidden output", encoding="utf-8")
            (root / "cell" / "provider-output.log").write_text("secret provider output", encoding="utf-8")
            (root / "cell" / "notes.txt").write_text("portable note", encoding="utf-8")

            output = root / "evidence"
            manifest = build_evidence_bundle(root, output, redaction="transcripts", strict=True)
            paths = {entry["path"] for entry in manifest["entries"]}

            self.assertNotIn("cell/public_check_0.stdout.txt", paths)
            self.assertNotIn("cell/hidden_check_0.stderr.txt", paths)
            self.assertNotIn("cell/provider-output.log", paths)
            self.assertFalse((output / "cell" / "public_check_0.stdout.txt").exists())
            self.assertFalse((output / "cell" / "hidden_check_0.stderr.txt").exists())
            self.assertIn("cell/notes.txt", paths)
            self.assertEqual((output / "cell" / "notes.txt").read_text(encoding="utf-8"), "portable note")

    def test_structured_json_rewrites_internal_and_external_absolute_paths(self) -> None:
        with tempfile.TemporaryDirectory() as root_name:
            root = Path(root_name).resolve()
            internal = root / "cell" / "workspace" / "app.py"
            external = Path("/Users/example/.credentials/provider.json")
            self.write_json(root / "matrix.json", self.valid_artifact(expected_total=1))
            self.write_json(root / "summary.json", self.valid_artifact(rows=1))
            self.write_json(
                root / "result.json",
                self.valid_artifact(
                    workspace_path=str(internal),
                    provider_config_path=str(external),
                    nested={"paths": [str(internal), str(external)]},
                    detail=f"opened {internal} then {external}",
                ),
            )

            output = root / "evidence"
            build_evidence_bundle(root, output, redaction="transcripts", strict=True)
            result = json.loads((output / "result.json").read_text(encoding="utf-8"))

            self.assertEqual(result["workspace_path"], "cell/workspace/app.py")
            self.assertEqual(result["provider_config_path"], "<absolute-path>/provider.json")
            self.assertEqual(
                result["nested"]["paths"],
                ["cell/workspace/app.py", "<absolute-path>/provider.json"],
            )
            serialized = json.dumps(result, sort_keys=True)
            self.assertNotIn(str(root), serialized)
            self.assertNotIn("/Users/example", serialized)
if __name__ == "__main__":
    unittest.main()
