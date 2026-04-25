#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
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

    print(f"verify overall_ok: {report.get('overall_ok')}")
    if args.verify_sample and report.get("overall_ok") is not False:
        print("Expected the bundled sample fixture to fail verify.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
