#!/usr/bin/env python3
"""Validate Court Jester release metadata and packaging contracts."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SUPPORTED_PLATFORMS = (
    "darwin-arm64",
    "darwin-amd64",
    "linux-amd64",
    "linux-arm64",
)


def read(path: str) -> str:
    return (REPO_ROOT / path).read_text(encoding="utf-8")


def cargo_version() -> str:
    manifest = read("Cargo.toml")
    package = manifest.split("[package]", 1)[1].split("[", 1)[0]
    match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', package, re.MULTILINE)
    if not match:
        raise ValueError("Cargo.toml [package] version is missing")
    return match.group(1)


def validate(tag: str) -> list[str]:
    errors: list[str] = []
    version = cargo_version()
    expected_tag = f"v{version}"
    if tag != expected_tag:
        errors.append(f"tag {tag!r} does not match Cargo package version {version!r}; expected {expected_tag!r}")

    lock = read("Cargo.lock")
    package_match = re.search(
        r'\[\[package\]\]\s*\nname = "court-jester"\s*\nversion = "([^"]+)"',
        lock,
    )
    if not package_match or package_match.group(1) != version:
        found = package_match.group(1) if package_match else "missing"
        errors.append(f"Cargo.lock court-jester version is {found!r}, expected {version!r}")

    changelog = read("CHANGELOG.md")
    if f"## {version} - " not in changelog:
        errors.append(f"CHANGELOG.md has no dated {version} release heading")

    notes_path = REPO_ROOT / "docs" / f"release-notes-{version}.md"
    if not notes_path.is_file():
        errors.append(f"missing release notes: {notes_path.relative_to(REPO_ROOT)}")

    workflow = read(".github/workflows/release.yml")
    if "python3 scripts/check_release.py --tag" not in workflow:
        errors.append("release workflow does not invoke the release contract validator")
    for platform in SUPPORTED_PLATFORMS:
        if f"platform: {platform}" not in workflow:
            errors.append(f"release workflow is missing platform {platform}")
    if 'court-jester-${RELEASE_TAG}-${{ matrix.platform }}.tar.gz' not in workflow:
        errors.append("release workflow asset naming no longer matches the installer contract")
    if "sha256" not in workflow.lower():
        errors.append("release workflow does not publish SHA-256 checksums")

    quality_workflow = read(".github/workflows/quality.yml")
    for required_gate in (
        "cargo fmt --all -- --check",
        "cargo clippy --locked --all-targets -- -D warnings",
        "cargo test --locked --tests",
        "scripts/smoke_cli.py",
        "--enforce-heldout-lock",
    ):
        if required_gate not in quality_workflow:
            errors.append(f"quality workflow is missing required gate: {required_gate}")

    installer = read("install.sh")
    if 'ASSET="court-jester-${TAG}-${PLATFORM}.tar.gz"' not in installer:
        errors.append("install.sh asset naming no longer matches the release workflow")
    if '"${URL}.sha256"' not in installer or "sha256sum" not in installer:
        errors.append("install.sh does not download and verify the published checksum")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="Release tag to validate, for example v0.2.0")
    args = parser.parse_args()
    try:
        errors = validate(args.tag)
    except (OSError, ValueError) as exc:
        errors = [str(exc)]
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(f"release metadata valid for {args.tag}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
