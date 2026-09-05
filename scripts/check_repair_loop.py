#!/usr/bin/env python3
"""Current-binary repair contract evidence; deterministic fixtures, never agent benchmarks."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


CASES = (
    dict(id="python-runtime", language="python", oracle="runtime_contract",
         bug="from typing import Literal\ndef first_character(value: Literal['', 'a']) -> str:\n    return value[0]\n",
         other="def first_character(value: str) -> str:\n    raise ValueError('different failure')\n",
         fixed="def first_character(value: str) -> str:\n    return value[0] if value else ''\n",
         symbol="first_character", examples=[("", ""), ("a", "a"), ("hello", "h")]),
    dict(id="typescript-runtime", language="typescript", oracle="runtime_contract",
         bug="export function firstCharacter(value: '' | 'a'): string { return value[0].toUpperCase(); }",
         other="export function firstCharacter(value: string): string { throw new Error('different failure'); }",
         fixed="export function firstCharacter(value: string): string { return value[0]?.toUpperCase() ?? ''; }",
         symbol="firstCharacter", examples=[("", ""), ("a", "A"), ("hello", "H")]),
    dict(id="python-property", language="python", oracle="declared_property",
         bug="# court-jester-properties sorted\ndef reorder(values: list[int]):\n    return [2, 1]\n",
         other="def reorder(values: list[int]):\n    return {}\n",
         fixed="def reorder(values: list[int]):\n    return sorted(values)\n",
         symbol="reorder", examples=[([], []), ([4], [4]), ([9, -2, 9, 0], [-2, 0, 9, 9])]),
    dict(id="typescript-property", language="typescript", oracle="declared_property",
         bug="// court-jester-properties sorted\nexport function reorder(values: number[]): any { return [2, 1]; }",
         other="export function reorder(values: number[]): any { return {}; }",
         fixed="export function reorder(values: number[]): number[] { return [...values].sort((a, b) => a - b); }",
         symbol="reorder", examples=[([], []), ([4], [4]), ([9, -2, 9, 0], [-2, 0, 9, 9])]),
)


class ContractFailure(Exception):
    def __init__(self, kind: str, message: str):
        super().__init__(message)
        self.kind = kind


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractFailure("contract_mismatch", message)


def command(argv: list[str], root: Path, phase: str, evidence: list[dict], env=None):
    started = time.monotonic()
    record = {"phase": phase}
    evidence.append(record)
    try:
        result = subprocess.run(argv, cwd=root, env=env, text=True, capture_output=True, timeout=60)
    except subprocess.TimeoutExpired as error:
        record["termination"] = "timeout"
        raise ContractFailure("timeout", f"{phase} exceeded its command deadline") from error
    except OSError as error:
        record["termination"] = "launch_failed"
        raise ContractFailure("launch_failed", f"{phase}: {error}") from error
    finally:
        record["duration_ms"] = round((time.monotonic() - started) * 1000, 3)
    record["exit_code"] = result.returncode
    return result


def response(result, evidence: list[dict]) -> dict:
    try:
        value = json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise ContractFailure("protocol", f"non-JSON response: {result.stderr[-1200:]}") from error
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value["schema_version"] != 3:
        raise ContractFailure("protocol", "expected a schema-v3 object")
    evidence[-1]["response"] = {key: value[key] for key in (
        "verdict", "outcome", "check_passed", "finding_id", "diagnostics_summary"
    ) if key in value}
    if value.get("verdict") == "inconclusive" or value.get("outcome") == "inconclusive":
        raise ContractFailure("inconclusive", "required target evidence is inconclusive")
    return value


def replay(binary: Path, root: Path, report: Path, finding: str, phase: str,
           outcome: str, passed: bool, evidence: list[dict]) -> dict:
    result = command([str(binary), "replay", "--report", str(report), "--finding", finding,
                      "--dependency-project-dir", str(root)], root, phase, evidence)
    value = response(result, evidence)
    require(result.returncode == (0 if outcome == "reproduced" else 1), f"{phase}: wrong exit code")
    require(value.get("finding_id") == finding and value.get("outcome") == outcome,
            f"{phase}: wrong finding or reproduction outcome")
    require(value.get("check_passed") is passed, f"{phase}: missing or wrong positive-check evidence")
    return value


def check_repair_examples(root: Path, case: dict, evidence: list[dict]) -> None:
    """Independent public fixture checks, not claims inferred from replay success."""
    inputs = [value for value, _ in case["examples"]]
    if case["language"] == "python":
        script = ("import json, target\n"
                  f"inputs = json.loads({json.dumps(json.dumps(inputs))})\n"
                  f"outputs = [getattr(target, {case['symbol']!r})(value) for value in inputs]\n"
                  "print(json.dumps({'outputs': outputs, 'inputs_after': inputs}))\n")
        argv = [sys.executable, "-c", script]
    else:
        script = ("import * as target from './target.ts';\n"
                  f"const inputs = {json.dumps(inputs)};\n"
                  f"const outputs = inputs.map(value => target[{json.dumps(case['symbol'])}](value));\n"
                  "console.log(JSON.stringify({outputs, inputs_after: inputs}));\n")
        argv = ["node", "--no-warnings", "--experimental-transform-types", "--input-type=module", "-e", script]
    result = command(argv, root, "independent_repair_examples", evidence)
    require(result.returncode == 0, "repaired implementation failed independent examples")
    try:
        actual = json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise ContractFailure("protocol", "independent repair examples returned non-JSON output") from error
    expected = {"outputs": [value for _, value in case["examples"]], "inputs_after": inputs}
    evidence[-1]["expected"] = expected
    evidence[-1]["observed"] = actual
    require(actual == expected, "repair does not preserve independent fixture behavior and input values")


def check_case(binary: Path, case: dict) -> dict:
    record = {"id": case["id"], "language": case["language"], "status": "failed", "phases": []}
    evidence = record["phases"]
    try:
        with tempfile.TemporaryDirectory(prefix="court-jester-repair-contract-") as directory:
            root = Path(directory)
            source = root / ("target.py" if case["language"] == "python" else "target.ts")
            source.write_text(case["bug"], encoding="utf-8")
            result = command([str(binary), "verify", "--file", str(source), "--language", case["language"],
                              "--project-dir", str(root), "--summary", "repair-json", "--timeout-seconds", "10"],
                             root, "verify_bug", evidence)
            report = response(result, evidence)
            require(result.returncode == 1 and report.get("verdict") == "fail", "bug must fail verification")
            findings = report.get("findings")
            require(isinstance(findings, list), "repair report lacks findings")
            eligible = [finding for finding in findings if isinstance(finding, dict)
                        and finding.get("input_classification") == "valid" and not finding.get("suppressed")
                        and finding.get("oracle", {}).get("kind") == case["oracle"]]
            require(bool(eligible), "missing eligible target finding")
            finding = eligible[0]
            finding_id = finding.get("id")
            require(isinstance(finding_id, str) and bool(finding_id), "finding lacks identity")
            record["finding_id"] = finding_id
            record["oracle"] = finding["oracle"]["kind"]
            report_path = root / "report.json"
            report_path.write_text(json.dumps(report), encoding="utf-8")
            replay(binary, root, report_path, finding_id, "replay_bug", "reproduced", False, evidence)
            source.write_text(case["other"], encoding="utf-8")
            replay(binary, root, report_path, finding_id, "reject_false_repair", "not_reproduced", False, evidence)
            source.write_text(case["fixed"], encoding="utf-8")
            check_repair_examples(root, case, evidence)
            replay(binary, root, report_path, finding_id, "replay_fixed", "not_reproduced", True, evidence)
            bundle = root / "regression"
            exported = command([str(binary), "replay", "--report", str(report_path), "--finding", finding_id,
                                "--dependency-project-dir", str(root), "--export-regression", str(bundle)],
                               root, "export_regression", evidence)
            exported_value = response(exported, evidence)
            require(exported.returncode == 0 and isinstance(exported_value.get("regression_export"), dict),
                    "regression export failed")
            test_args = ([sys.executable, str(bundle / "test_regression.py")] if case["language"] == "python"
                         else ["node", "--test", str(bundle / "regression.test.mjs")])
            env = dict(os.environ, COURT_JESTER_BINARY=str(binary))
            fixed = command(test_args, root, "regression_fixed", evidence, env)
            require(fixed.returncode == 0, "exported test must pass after repair")
            source.write_text(case["other"], encoding="utf-8")
            other = command(test_args, root, "regression_false_repair", evidence, env)
            require(other.returncode > 0, "exported test accepted a false repair")
            source.write_text(case["bug"], encoding="utf-8")
            original = command(test_args, root, "regression_bug", evidence, env)
            require(original.returncode > 0, "exported test accepted the original bug")
            replay(binary, root, report_path, finding_id, "confirm_restored_bug", "reproduced", False, evidence)
        record["status"] = "passed"
    except ContractFailure as error:
        record["failure"] = {"kind": error.kind, "message": str(error)}
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        record["failure"] = {"kind": "harness_error", "message": str(error)}
    return record


def check_binary(binary: Path) -> dict:
    before = hashlib.sha256(binary.read_bytes()).hexdigest()
    version = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=10)
    require(version.returncode == 0 and version.stdout.strip().startswith("court-jester "), "binary version probe failed")
    try:
        node = subprocess.run(["node", "--version"], capture_output=True, text=True, timeout=10)
        node_version = node.stdout.strip() if node.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        node_version = None
    results = [check_case(binary, case) for case in CASES]
    after = hashlib.sha256(binary.read_bytes()).hexdigest()
    passed = sum(result["status"] == "passed" for result in results)
    return {
        "artifact_schema_version": 1, "suite": "repair-contract-v1",
        "evidence_kind": "deterministic_repair_contract_not_agent_benchmark",
        "binary": {"sha256": before, "sha256_after": after, "version": version.stdout.strip()},
        "fixture_sha256": hashlib.sha256(json.dumps(CASES, sort_keys=True).encode()).hexdigest(),
        "environment": {"runner_python": sys.version.split()[0], "node_on_path": node_version, "platform": sys.platform},
        "binary_unchanged": before == after,
        "cases": results, "summary": {"cases": len(CASES), "passed": passed, "failed": len(CASES) - passed},
        "status": "passed" if results and passed == len(CASES) and before == after else "failed",
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", type=Path, help="New evidence file; parent must exist, never overwritten")
    args = parser.parse_args(argv)
    if args.output and os.path.lexists(args.output):
        parser.error("evidence output already exists")
    try:
        report = check_binary(args.binary.resolve(strict=True))
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            with args.output.open("x", encoding="utf-8") as destination:
                destination.write(rendered)
        print(rendered, end="")
        return 0 if report["status"] == "passed" else 1
    except (OSError, ContractFailure, subprocess.TimeoutExpired) as error:
        print(f"repair contract unavailable: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
