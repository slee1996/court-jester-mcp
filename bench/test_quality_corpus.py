"""Current-binary advisory test-quality classification corpus, with no adequacy score."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


MANIFEST = Path(__file__).with_name("test_quality_cases.json")
OUTCOMES = ("killed", "survived", "invalid", "blocked", "no_coverage")
VALIDATION_FAULTS = {"valid": "valid", "stale_source": "invalid_edit", "invalid_range": "invalid_edit",
                     "split_utf8": "invalid_edit", "syntax": "invalid_syntax", "changed_surface": "changed_surface"}


def check_validation(report: dict, verifier_digest: str, validator_digest: str) -> list[str]:
    if not isinstance(report, dict):
        return ["validation artifact must be an object"]
    errors = []
    for key, expected in {"artifact_schema_version": 1, "suite": "test-quality-validation-v1",
                          "evidence_kind": "fault_injected_validation_boundary_not_generated_runtime_mutants",
                          "status": "passed", "verifier_binary_sha256": verifier_digest,
                          "validator_binary_sha256": validator_digest}.items():
        if type(report.get(key)) is not type(expected) or report.get(key) != expected:
            errors.append(f"invalid validation {key}")
    for key in ("validation_source_sha256", "fixture_source_sha256"):
        value = report.get(key)
        if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
            errors.append(f"invalid validation {key}")
    rows = report.get("cases")
    if not isinstance(rows, list):
        return errors + ["validation cases must be a list"]
    seen = set()
    expected_keys = {(language, fault) for language in ("python", "typescript") for fault in VALIDATION_FAULTS}
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("language"), str) or not isinstance(row.get("fault"), str):
            errors.append("malformed validation case")
            continue
        key = (row["language"], row["fault"])
        if key not in expected_keys or key in seen:
            errors.append("unexpected or duplicate validation case")
            continue
        seen.add(key)
        expected = VALIDATION_FAULTS[key[1]]
        if (row.get("id") != "-".join(key) or row.get("expected") != expected or row.get("observed") != expected
                or row.get("classification") != ("valid" if expected == "valid" else "invalid")
                or row.get("matched") is not True or row.get("mutant_execution_started") is not False):
            errors.append(f"validation case contract mismatch: {key}")
    if seen != expected_keys:
        errors.append("incomplete validation matrix")
    return errors


def run_validation(binary: Path, validator: Path, verifier_digest: str) -> dict:
    result = {"status": "failed"}
    try:
        digest = hashlib.sha256(validator.read_bytes()).hexdigest()
        output = subprocess.run([str(validator), str(binary)], text=True, capture_output=True, timeout=30)
        report = json.loads(output.stdout)
        errors = check_validation(report, verifier_digest, digest)
        if output.returncode != 0:
            errors.append("validation helper exited unsuccessfully")
        if hashlib.sha256(validator.read_bytes()).hexdigest() != digest:
            errors.append("validation helper changed during execution")
        result.update(report=report, errors=errors, status="failed" if errors else "passed")
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        result["errors"] = [str(error)]
    return result


def check_case(case: dict, report: dict, exit_code: int) -> list[str]:
    errors = []
    if not isinstance(report, dict) or report.get("schema_version") != 3:
        return ["schema-v3 verification report required"]
    if exit_code != 0 or report.get("verdict") != "pass":
        errors.append("clean authoritative baseline must pass; mutation observations remain advisory")
    stages = [stage for stage in report.get("stages", []) if stage.get("name") == "test_quality"]
    if len(stages) != 1:
        return errors + ["exactly one test-quality stage is required"]
    stage = stages[0]
    detail = stage.get("detail") or {}
    expected = case["expected"]
    if detail.get("mode") != "advisory" or detail.get("baseline_eligible") is not True:
        errors.append("advisory mode and an eligible baseline are required")
    if any(key in detail for key in ("score", "grade", "percentage")):
        errors.append("classification evidence must not invent an adequacy score")
    counts = detail.get("counts") or {}
    observations = detail.get("mutants") or []
    observed = Counter(item.get("outcome") for item in observations)
    if counts.get("planned") != 1 or len(observations) != 1:
        errors.append("fixture requires exactly one planned and observed mutant")
    for outcome in OUTCOMES:
        if type(counts.get(outcome)) is not int or counts.get(outcome) != int(outcome == expected["outcome"]):
            errors.append(f"unexpected {outcome} count: {counts.get(outcome)!r}")
        if observed[outcome] != counts.get(outcome):
            errors.append(f"{outcome} observations do not reconcile with counts")
    if any(outcome not in OUTCOMES for outcome in observed):
        errors.append("unknown mutant outcome")
    for observation in observations:
        if observation.get("entered_mutated_surface") is not expected["entered_mutated_surface"]:
            errors.append("mutated-surface entry evidence differs from the fixture contract")
    coupling = sorted(item.get("kind", "") for item in detail.get("coupling_findings", []))
    if coupling != sorted(expected["coupling"]):
        errors.append(f"unexpected coupling classifications: {coupling!r}")
    if detail.get("coupling_error") is not None or detail.get("planning_error") is not None:
        errors.append("fixture planning/coupling analysis must not abstain")
    expected_stage = "passed" if expected["outcome"] == "killed" and not coupling else "advisory"
    if stage.get("status") != expected_stage:
        errors.append(f"expected {expected_stage} advisory-stage status")
    return errors


def run(binary: Path, manifest_path: Path = MANIFEST, validation_binary: Path | None = None) -> dict:
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema_version") != 1 or not manifest.get("cases"):
        raise ValueError("unsupported or empty test-quality corpus")
    ids = [case["id"] for case in manifest["cases"]]
    if len(ids) != len(set(ids)):
        raise ValueError("duplicate corpus case ids")
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    result = {"artifact_schema_version": 1, "suite": manifest["suite"], "scope": manifest["scope"],
              "binary_sha256": digest, "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "cases": []}
    for case in manifest["cases"]:
        started = time.monotonic()
        record = {"id": case["id"], "language": case["language"], "expected": case["expected"], "status": "failed"}
        result["cases"].append(record)
        try:
            with tempfile.TemporaryDirectory(prefix="court-jester-quality-") as directory:
                root = Path(directory)
                extension = {"python": "py", "typescript": "ts"}[case["language"]]
                source = root / f"target.{extension}"
                tests = root / f"checks.test.{extension}"
                source.write_text(case["source"])
                tests.write_text(case["tests"].replace("{state_path}", json.dumps(str(root / "baseline-ran"))))
                args = [str(binary), "verify", "--file", str(source), "--language", case["language"],
                        "--project-dir", str(root), "--test-file", str(tests), "--tests-only", "--test-quality", "1",
                        "--no-repo-config", "--no-auto-seed", "--timeout-seconds", "2", "--memory-mb", "512"]
                if extension == "ts":
                    args.extend(["--test-runner", "node"])
                output = subprocess.run(args, cwd=root, text=True, capture_output=True, timeout=30)
                report = json.loads(output.stdout)
                record.update(exit_code=output.returncode, report=report)
                errors = check_case(case, report, output.returncode)
                record["errors"] = errors
                record["status"] = "failed" if errors else "passed"
        except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.TimeoutExpired) as error:
            record["errors"] = [str(error)]
        record["duration_ms"] = round((time.monotonic() - started) * 1000, 3)
    result["validation"] = (run_validation(binary, validation_binary, digest) if validation_binary is not None
                            else {"status": "not_run", "reason": "runtime-only invocation; validation helper not supplied"})
    result["binary_unchanged"] = hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    result["status"] = "passed" if result["binary_unchanged"] and all(case["status"] == "passed" for case in result["cases"]) else "failed"
    result["summary"] = {"cases": len(result["cases"]), "matched": sum(case["status"] == "passed" for case in result["cases"])}
    if result["validation"]["status"] == "failed":
        result["status"] = "failed"
    result["classification_evidence_complete"] = result["status"] == "passed" and result["validation"]["status"] == "passed"
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/court-jester"))
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--validation-binary", type=Path, help="fault-injection helper built alongside the verifier")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = run(args.binary.resolve(strict=True), args.manifest,
                 args.validation_binary.resolve(strict=True) if args.validation_binary else None)
    rendered = json.dumps(result, indent=2)
    if args.output:
        with args.output.open("x") as output:
            output.write(rendered + "\n")
    print(rendered)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
