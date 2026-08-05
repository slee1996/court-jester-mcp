#!/usr/bin/env python3

from __future__ import annotations
import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent

SAMPLE_FIXTURE = {
    "verify_file": REPO_ROOT / "bench/repos/mini_py_literal_domain_service/status.py",
    "language": "python",
    "project_dir": REPO_ROOT / "bench/repos/mini_py_literal_domain_service",
    "test_file": None,
}


def resolve_binary(args: argparse.Namespace) -> Path:
    if args.binary:
        binary = Path(args.binary).expanduser().resolve()
    else:
        profile = "debug" if args.debug else "release"
        binary = REPO_ROOT / "target" / profile / "court-jester"
    if not binary.exists():
        raise FileNotFoundError(
            f"Could not find {binary}. Build it first with `cargo build --{profile_name(args)}`."
        )
    return binary


def profile_name(args: argparse.Namespace) -> str:
    return "debug" if args.debug else "release"


def run_command(argv: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )



def run_extended_release_smoke(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="court-jester-smoke-") as directory:
        project = Path(directory)
        valid_tsx = project / "Badge.tsx"
        valid_tsx.write_text(
            "export function Badge({ label }: { label: string }) {\n"
            "  return <span>{label}</span>;\n"
            "}\n",
            encoding="utf-8",
        )
        analyze = run_command(
            [
                str(binary),
                "analyze",
                "--file",
                str(valid_tsx),
                "--language",
                "typescript",
                "--project-dir",
                str(project),
            ],
            project,
        )
        if analyze.returncode != 0:
            raise RuntimeError(analyze.stderr.strip() or analyze.stdout.strip())
        analysis = json.loads(analyze.stdout)
        if (
            analysis.get("source_mode") != "tsx"
            or analysis.get("parse_error") is not False
            or not any(
                function.get("name") == "Badge"
                for function in analysis.get("functions", [])
            )
        ):
            raise RuntimeError(f"valid TSX analysis was not admitted: {analysis}")

        malformed_tsx = project / "Malformed.tsx"
        malformed_tsx.write_text(
            "export function Badge( {\n  return <span>broken;\n",
            encoding="utf-8",
        )
        malformed = run_command(
            [
                str(binary),
                "analyze",
                "--file",
                str(malformed_tsx),
                "--language",
                "typescript",
                "--project-dir",
                str(project),
            ],
            project,
        )
        if malformed.returncode != 0:
            raise RuntimeError(malformed.stderr.strip() or malformed.stdout.strip())
        malformed_report = json.loads(malformed.stdout)
        diagnostics = malformed_report.get("parse_diagnostics", [])
        if not malformed_report.get("parse_error") or not diagnostics:
            raise RuntimeError(
                f"malformed TSX did not produce structured diagnostics: {malformed_report}"
            )
        first_diagnostic = diagnostics[0]
        if not (
            isinstance(first_diagnostic.get("start_line"), int)
            and first_diagnostic["start_line"] >= 1
            and isinstance(first_diagnostic.get("start_column"), int)
            and first_diagnostic["start_column"] >= 1
        ):
            raise RuntimeError(
                f"malformed TSX diagnostic lacks a location: {first_diagnostic}"
            )

        target = project / "arg_target.py"
        target.write_text(
            "import argparse\n"
            "import json\n"
            "from pathlib import Path\n"
            "\n"
            "_parser = argparse.ArgumentParser()\n"
            "_parser.add_argument('action')\n"
            "_parser.add_argument('--manifest', required=True)\n"
            "_arguments = _parser.parse_args()\n"
            "Path(_arguments.manifest).read_text(encoding='utf-8')\n"
            "\n"
            "def echo(value: str) -> str:\n"
            "    return value\n",
            encoding="utf-8",
        )
        (project / "manifest.json").write_text(
            json.dumps({"name": "smoke"}), encoding="utf-8"
        )
        harness_args = json.dumps(
            [
                {"literal": "run"},
                {"literal": "--manifest"},
                {"project_path": "manifest.json"},
            ]
        )
        verify = run_command(
            [
                str(binary),
                "verify",
                "--file",
                str(target),
                "--language",
                "python",
                "--project-dir",
                str(project),
                "--harness-args-json",
                harness_args,
            ],
            project,
        )
        if verify.returncode not in {0, 1}:
            raise RuntimeError(verify.stderr.strip() or verify.stdout.strip())
        report = json.loads(verify.stdout)
        if "required" in (verify.stdout + verify.stderr).lower() and "manifest" in (
            verify.stdout + verify.stderr
        ).lower():
            raise RuntimeError(
                "Python harness arguments were not forwarded to the target: "
                f"{verify.stderr.strip()}"
            )
        execute_stage = next(
            (
                stage
                for stage in report.get("stages", [])
                if stage.get("name") == "execute"
            ),
            None,
        )
        execute_detail = (execute_stage or {}).get("detail") or {}
        events = execute_detail.get("harness_events") or {}
        if events.get("target_ready") is not True:
            raise RuntimeError(
                f"generated harness did not report target readiness: {events}"
            )
        termination = (execute_detail.get("execution") or {}).get("termination") or {}
        if termination.get("kind") in {"timed_out", "memory_limit"}:
            raise RuntimeError(
                f"argument-forwarding smoke unexpectedly hit a resource limit: {termination}"
            )

def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke-test the Court Jester CLI.")
    profile = parser.add_mutually_exclusive_group()
    profile.add_argument(
        "--release",
        action="store_true",
        help="Use target/release/court-jester (default).",
    )
    profile.add_argument(
        "--debug",
        action="store_true",
        help="Use target/debug/court-jester.",
    )
    parser.add_argument(
        "--binary",
        help="Use an explicit binary path instead of target/{release,debug}/court-jester.",
    )
    parser.add_argument("--verify-file", help="Optional source file to verify.")
    parser.add_argument(
        "--language",
        choices=["python", "typescript"],
        help="Language for --verify-file.",
    )
    parser.add_argument(
        "--project-dir",
        help="Optional project directory for import and dependency resolution.",
    )
    parser.add_argument(
        "--test-file",
        help="Optional explicit test file to include in the verify call.",
    )
    parser.add_argument(
        "--verify-sample",
        action="store_true",
        help=(
            "Run a full verify call against the bundled mini_py_service fixture. "
            "Overrides --verify-file/--language/--project-dir/--test-file."
        ),
    )
    args = parser.parse_args()

    if args.verify_sample:
        args.verify_file = str(SAMPLE_FIXTURE["verify_file"])
        args.language = SAMPLE_FIXTURE["language"]
        args.project_dir = str(SAMPLE_FIXTURE["project_dir"])
        if SAMPLE_FIXTURE["test_file"]:
            args.test_file = str(SAMPLE_FIXTURE["test_file"])

    try:
        binary = resolve_binary(args)
    except Exception as exc:
        print(exc, file=sys.stderr)
        return 1

    version = run_command([str(binary), "--version"], REPO_ROOT)
    if version.returncode != 0:
        print(version.stderr.strip() or version.stdout.strip(), file=sys.stderr)
        return 1
    print(version.stdout.strip())

    help_result = run_command([str(binary), "--help"], REPO_ROOT)
    if help_result.returncode != 0 or "verify" not in help_result.stdout:
        print(help_result.stderr.strip() or help_result.stdout.strip(), file=sys.stderr)
        return 1
    print("Help output includes subcommands.")

    if not args.verify_file:
        return 0

    if not args.language:
        print("--language is required when --verify-file is set", file=sys.stderr)
        return 1

    verify_cmd = [
        str(binary),
        "verify",
        "--file",
        str(Path(args.verify_file).expanduser().resolve()),
        "--language",
        args.language,
    ]
    if args.project_dir:
        verify_cmd.extend(["--project-dir", str(Path(args.project_dir).expanduser().resolve())])
    if args.test_file:
        verify_cmd.extend(["--test-file", str(Path(args.test_file).expanduser().resolve())])

    verify_result = run_command(
        verify_cmd,
        Path(args.project_dir).expanduser().resolve() if args.project_dir else REPO_ROOT,
    )
    if verify_result.returncode not in {0, 1}:
        print(verify_result.stderr.strip() or verify_result.stdout.strip(), file=sys.stderr)
        return 1
    try:
        report = json.loads(verify_result.stdout)
    except json.JSONDecodeError as exc:
        print(f"Verify output was not valid JSON: {exc}", file=sys.stderr)
        return 1

    print(f"verify verdict: {report.get('verdict')}")
    if args.verify_sample and report.get("verdict") != "fail":
        print("Expected the bundled sample fixture to fail verify.", file=sys.stderr)
        return 1
    if args.verify_sample:
        try:
            run_extended_release_smoke(binary)
        except (OSError, RuntimeError, json.JSONDecodeError) as exc:
            print(f"Extended release smoke failed: {exc}", file=sys.stderr)
            return 1
        print("Extended TSX and harness-argument smoke checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
