#!/usr/bin/env python3
"""Fresh temporary Python project onboarding contract; no installs or user data."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


def run(binary: Path) -> dict:
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    evidence = {"schema_version": 1, "suite": "python-onboarding-v1", "binary_sha256": digest,
                "scope": "temporary_project_config_doctor_probe_not_user_research", "phases": [], "status": "failed"}
    try:
        with tempfile.TemporaryDirectory(prefix="court-jester-onboarding-") as directory:
            root = Path(directory)
            marker = root / "entrypoint-ran"
            (root / "target.py").write_text("from dependency import identity\ndef inspect(value: bool):\n    return identity(value)\n")
            (root / "checks.py").write_text(f"from pathlib import Path\nfrom target import inspect\nPath({str(marker)!r}).touch()\nassert inspect(False) is False\n")
            (root / ".court-jester.json").write_text(json.dumps({"schema_version": 1, "defaults": {"timeout_seconds": 2, "memory_mb": 256}, "targets": [{"source": "target.py", "test_files": ["checks.py"]}]}))

            def invoke(phase: str, extra: list[str], exit_code: int = 0) -> dict:
                started = time.monotonic()
                result = subprocess.run([str(binary), "doctor", "--file", "target.py", "--language", "python", *extra], cwd=root, text=True, capture_output=True, timeout=30)
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
            (root / "dependency.py").write_text("def identity(value):\n    return value\n")
            ready = invoke("dependency_repaired", ["--probe-entrypoint"])
            require(ready["verdict"] == "pass" and probe(ready)["status"] == "passed" and marker.exists(), "repaired project did not become ready")
            require(probe(ready)["detail"]["test_stage"]["detail"]["target_module_loaded"] is True, "ready probe lacks target module-load evidence")
            (root / "dependency.py").write_text("def identity(value):\n    return True\n")
            failing = invoke("failing_test", ["--probe-entrypoint"], 1)
            require(probe(failing)["status"] == "failed", "failing assertion was reported ready")
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
    args = parser.parse_args()
    result = run(args.binary.resolve(strict=True))
    rendered = json.dumps(result, indent=2)
    if args.output:
        with args.output.open("x") as output:
            output.write(rendered + "\n")
    print(rendered)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
