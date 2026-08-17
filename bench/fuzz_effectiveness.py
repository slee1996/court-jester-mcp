from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any


BENCH_ROOT = Path(__file__).resolve().parent
DEFAULT_MANIFEST = BENCH_ROOT / "fuzz_effectiveness_cases.json"
SAFE_CASE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}")


def validate_case_id(case_id: object) -> str:
    if not isinstance(case_id, str) or SAFE_CASE_ID.fullmatch(case_id) is None:
        raise ValueError(
            "every fuzz effectiveness case id must be a safe artifact slug"
        )
    return case_id


def load_manifest(path: Path) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise ValueError("fuzz effectiveness manifest must use schema_version 1")
    cases = manifest.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("fuzz effectiveness manifest must contain at least one case")
    if any(not isinstance(case, dict) for case in cases):
        raise ValueError("every fuzz effectiveness case must be an object")
    ids = [validate_case_id(case.get("id")) for case in cases]
    if len(ids) != len(set(ids)):
        raise ValueError("fuzz effectiveness case ids must be unique")
    return manifest


def report_findings(report: dict[str, Any]) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    for stage in report.get("stages", []):
        detail = stage.get("detail")
        if not isinstance(detail, dict):
            continue
        stage_findings = detail.get("findings")
        if isinstance(stage_findings, list):
            findings.extend(item for item in stage_findings if isinstance(item, dict))
    return findings


def evaluate_case(case: dict[str, Any], report: dict[str, Any]) -> dict[str, Any]:
    mismatches: list[str] = []
    expected_verdict = case.get("expected_verdict")
    actual_verdict = report.get("verdict")
    if actual_verdict != expected_verdict:
        mismatches.append(f"verdict: expected {expected_verdict!r}, got {actual_verdict!r}")

    expected_stage = case.get("expected_stage")
    actual_stage: dict[str, Any] | None = None
    if isinstance(expected_stage, dict):
        actual_stage = next(
            (
                stage
                for stage in report.get("stages", [])
                if stage.get("name") == expected_stage.get("name")
            ),
            None,
        )
        if actual_stage is None:
            mismatches.append(f"missing stage {expected_stage.get('name')!r}")
        elif actual_stage.get("status") != expected_stage.get("status"):
            mismatches.append(
                "stage "
                f"{expected_stage.get('name')!r}: expected {expected_stage.get('status')!r}, "
                f"got {actual_stage.get('status')!r}"
            )

    findings = report_findings(report)
    expected_finding = case.get("expected_finding")
    finding_matched = expected_finding is None
    if isinstance(expected_finding, dict):
        finding_matched = any(
            finding.get("location", {}).get("function") == expected_finding.get("function")
            and finding.get("category") == expected_finding.get("category")
            for finding in findings
        )
        if not finding_matched:
            mismatches.append(
                "missing finding "
                f"{expected_finding.get('function')!r}/{expected_finding.get('category')!r}"
            )

    if case.get("kind") == "control" and findings:
        mismatches.append(f"control emitted {len(findings)} finding(s)")

    return {
        "id": case["id"],
        "kind": case.get("kind"),
        "technique": case.get("technique"),
        "matched": not mismatches,
        "mismatches": mismatches,
        "actual_verdict": actual_verdict,
        "expected_verdict": expected_verdict,
        "finding_matched": finding_matched,
        "finding_count": len(findings),
        "stage": {
            "name": actual_stage.get("name"),
            "status": actual_stage.get("status"),
        }
        if actual_stage
        else None,
    }


def summarize(results: list[dict[str, Any]]) -> dict[str, Any]:
    mutations = [result for result in results if result["kind"] == "mutation"]
    controls = [result for result in results if result["kind"] == "control"]
    detected = sum(result["matched"] for result in mutations)
    clean = sum(result["matched"] for result in controls)
    return {
        "cases": len(results),
        "matched": sum(result["matched"] for result in results),
        "mutations": len(mutations),
        "mutations_detected": detected,
        "mutation_recall": detected / len(mutations) if mutations else None,
        "controls": len(controls),
        "controls_clean": clean,
        "specificity": clean / len(controls) if controls else None,
    }


def resolve_fixture(path_text: str) -> Path:
    path = (BENCH_ROOT / path_text).resolve()
    try:
        path.relative_to(BENCH_ROOT)
    except ValueError as error:
        raise ValueError(f"fixture escapes benchmark root: {path_text}") from error
    if not path.is_file():
        raise ValueError(f"fixture does not exist: {path_text}")
    return path

def run_verify(
    binary: Path,
    case: dict[str, Any],
    output_dir: Path,
    extra_args: list[str] | None = None,
    report_level: str = "minimal",
) -> tuple[dict[str, Any], int]:
    source = resolve_fixture(case["source"])
    command = [
        str(binary),
        "verify",
        "--file",
        str(source),
        "--language",
        case["language"],
        "--project-dir",
        str(source.parent),
        "--output-dir",
        str(output_dir),
        "--report-level",
        report_level,
    ]
    if case.get("base_source"):
        base_source = resolve_fixture(case["base_source"])
        command.extend(
            [
                "--base-file",
                str(base_source),
                "--base-project-dir",
                str(base_source.parent),
            ]
        )
    if extra_args:
        command.extend(extra_args)

    started = time.monotonic()
    completed = subprocess.run(command, capture_output=True, text=True, check=False)
    duration_ms = int((time.monotonic() - started) * 1000)
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"{case['id']}: verifier did not emit JSON (exit {completed.returncode}): "
            f"{completed.stderr.strip() or completed.stdout.strip()}"
        ) from error
    report["_benchmark_duration_ms"] = duration_ms
    report["_benchmark_exit_code"] = completed.returncode
    return report, duration_ms


def write_llm_provider(path: Path, seeds: list[dict[str, Any]]) -> None:
    payload = json.dumps({"seeds": seeds}, ensure_ascii=False)
    path.write_text(
        "#!/usr/bin/env python3\n"
        "import json\n"
        "import sys\n"
        "json.load(sys.stdin)\n"
        f"print({payload!r})\n",
        encoding="utf-8",
    )
    path.chmod(path.stat().st_mode | 0o111)

def run_case(binary: Path, case: dict[str, Any], output_dir: Path) -> dict[str, Any]:
    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True)
    duration_ms = 0
    if case.get("llm_seeds"):
        baseline, baseline_ms = run_verify(
            binary, case, output_dir, report_level="full"
        )
        duration_ms += baseline_ms
        coverage = next(
            (stage for stage in baseline.get("stages", []) if stage.get("name") == "coverage"),
            {},
        )
        retained = coverage.get("detail", {}).get("corpus_retained", 0)
        if not isinstance(retained, int) or retained <= 0:
            result = evaluate_case(case, baseline)
            result["matched"] = False
            result["mismatches"].append("baseline run retained no corpus for plateau detection")
            result["duration_ms"] = duration_ms
            return result
        provider = output_dir / "llm-seed-provider"
        write_llm_provider(provider, case["llm_seeds"])
        report, plateau_ms = run_verify(
            binary,
            case,
            output_dir,
            ["--llm-plateau-command", str(provider)],
        )
        duration_ms += plateau_ms
    else:
        report, duration_ms = run_verify(binary, case, output_dir)

    result = evaluate_case(case, report)
    result["duration_ms"] = duration_ms
    result["verifier_exit_code"] = report["_benchmark_exit_code"]
    result["report_path"] = report.get("report_path")
    if report["_benchmark_exit_code"] not in (0, 1):
        result["matched"] = False
        result["mismatches"].append(
            f"verifier infrastructure exit code {report['_benchmark_exit_code']}"
        )
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Measure Court Jester mutation recall and clean-control specificity."
    )
    parser.add_argument("--binary", type=Path, default=Path("target/release/court-jester"))
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--case", action="append", dest="case_ids", default=[])
    parser.add_argument("--artifacts-dir", type=Path)
    parser.add_argument("--json-out", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"court-jester binary is not executable: {binary}")
    manifest = load_manifest(args.manifest.resolve())
    cases = manifest["cases"]
    if args.case_ids:
        selected = set(args.case_ids)
        cases = [case for case in cases if case["id"] in selected]
        missing = sorted(selected - {case["id"] for case in cases})
        if missing:
            raise SystemExit(f"unknown fuzz effectiveness case(s): {', '.join(missing)}")

    temporary: tempfile.TemporaryDirectory[str] | None = None
    if args.artifacts_dir:
        artifacts_dir = args.artifacts_dir.resolve()
        artifacts_dir.mkdir(parents=True, exist_ok=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="court-jester-fuzz-effectiveness-")
        artifacts_dir = Path(temporary.name)

    try:
        results = [
            run_case(binary, case, artifacts_dir / case["id"])
            for case in cases
        ]
        output = {
            "schema_version": 1,
            "suite": manifest.get("suite"),
            "binary": str(binary),
            "summary": summarize(results),
            "results": results,
        }
        rendered = json.dumps(output, indent=2, sort_keys=True) + "\n"
        print(rendered, end="")
        if args.json_out:
            args.json_out.parent.mkdir(parents=True, exist_ok=True)
            args.json_out.write_text(rendered, encoding="utf-8")
        return 0 if output["summary"]["matched"] == output["summary"]["cases"] else 1
    finally:
        if temporary is not None:
            temporary.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
