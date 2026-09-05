#!/usr/bin/env python3
"""Fresh temporary project onboarding contract; no installs or user data."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


def run(binary: Path, language: str = "python") -> dict:
    if language not in ("python", "typescript"):
        raise ValueError(f"Unsupported onboarding language: {language}")
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    evidence = {"schema_version": 1, "suite": f"{language}-onboarding-v1", "language": language, "binary_sha256": digest,
                "scope": "temporary_project_config_doctor_probe_not_user_research", "phases": [], "status": "failed"}
    try:
        with tempfile.TemporaryDirectory(prefix="court-jester-onboarding-") as directory:
            root = Path(directory)
            marker = root / "entrypoint-ran"
            if language == "python":
                source, tests, dependency = "target.py", "checks.py", "dependency.py"
                target_code = "from dependency import identity\ndef inspect(value: bool):\n    return identity(value)\n"
                test_code = f"from pathlib import Path\nfrom target import inspect\nPath({str(marker)!r}).touch()\nassert inspect(False) is False\n"
                repaired = "def identity(value):\n    return value\n"
                incorrect = "def identity(value):\n    return True\n"
            else:
                source, tests, dependency = "target.ts", "checks.test.ts", "dependency.ts"
                target_code = "import { identity } from './dependency.ts';\nexport function inspect(value: boolean): boolean { return identity(value); }\n"
                test_code = ("import { writeFileSync } from 'node:fs';\nimport assert from 'node:assert/strict';\n"
                             "import { test } from 'node:test';\nimport { inspect } from './target.ts';\n"
                             f"writeFileSync({json.dumps(str(marker))}, 'ran');\n"
                             "test('configured entrypoint preserves false', () => { assert.equal(inspect(false), false); });\n")
                repaired = "export function identity(value: boolean): boolean { return value; }\n"
                incorrect = "export function identity(value: boolean): boolean { return true; }\n"
                (root / "package.json").write_text(json.dumps({"private": True, "type": "module"}))
            (root / source).write_text(target_code)
            (root / tests).write_text(test_code)
            (root / ".court-jester.json").write_text(json.dumps({"schema_version": 1, "defaults": {"timeout_seconds": 2, "memory_mb": 256}, "targets": [{"source": source, "test_files": [tests]}]}))

            def invoke(phase: str, extra: list[str], exit_code: int = 0) -> dict:
                started = time.monotonic()
                result = subprocess.run([str(binary), "doctor", "--file", source, "--language", language, *extra], cwd=root, text=True, capture_output=True, timeout=30)
                value = json.loads(result.stdout)
                evidence["phases"].append({"phase": phase, "exit_code": result.returncode, "duration_ms": round((time.monotonic() - started) * 1000, 3), "report": value})
                if result.returncode != exit_code:
                    raise ValueError(f"{phase}: expected exit {exit_code}, got {result.returncode}")
                return value

            def probe(value: dict) -> dict:
                return next(check for check in value["checks"] if check["name"] == "entrypoint_probe")

            def require(condition: bool, message: str) -> None:
                if not condition:
                    raise ValueError(message)

            settings = invoke("inspect_config", ["--show-config"])
            require(settings["execution_started"] is False and settings["limits"]["memory_mb"] == 256, "configuration inspection contract failed")
            readiness = invoke("readiness_without_execution", [])
            require(not marker.exists() and all(check["name"] != "entrypoint_probe" for check in readiness["checks"]), "default doctor executed the entrypoint")
            broken = invoke("missing_dependency", ["--probe-entrypoint"], 1)
            require(probe(broken)["status"] == "failed" and not marker.exists(), "missing dependency did not fail the explicit probe")
            (root / dependency).write_text(repaired)
            ready = invoke("dependency_repaired", ["--probe-entrypoint"])
            require(ready["verdict"] == "pass" and probe(ready)["status"] == "passed" and marker.exists(), "repaired project did not become ready")
            require(probe(ready)["detail"]["test_stage"]["detail"]["target_module_loaded"] is True, "ready probe lacks target module-load evidence")
            marker.unlink()
            (root / dependency).write_text(incorrect)
            failing = invoke("failing_test", ["--probe-entrypoint"], 1)
            require(probe(failing)["status"] == "failed", "failing assertion was reported ready")
            require(marker.exists() and probe(failing)["detail"]["test_stage"]["detail"]["target_module_loaded"] is True, "failing test did not load and execute the configured entrypoint")
            evidence["status"] = "passed"
    except (OSError, ValueError, KeyError, StopIteration, subprocess.TimeoutExpired) as error:
        evidence["error"] = str(error)
    evidence["binary_unchanged"] = hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    if not evidence["binary_unchanged"]:
        evidence["status"] = "failed"
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/court-jester"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--language", choices=["python", "typescript"], default="python")
    args = parser.parse_args()
    result = run(args.binary.resolve(strict=True), args.language)
    rendered = json.dumps(result, indent=2)
    if args.output:
        with args.output.open("x") as output:
            output.write(rendered + "\n")
    print(rendered)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
