from __future__ import annotations

import json
import sys
from pathlib import Path

from helpers import hidden_rng, run_bun_assertions


def _parse_ident(value: str) -> tuple[int, int | str]:
    if value.isdigit():
        return (0, int(value))
    return (1, value)


def compare_semver(left: str, right: str) -> int:
    def parse(value: str) -> tuple[tuple[int, int, int], list[str] | None]:
        normalized = value.strip().removeprefix("v").split("+", 1)[0]
        if "-" in normalized:
            core, prerelease = normalized.split("-", 1)
            prerelease_parts = prerelease.split(".")
        else:
            core = normalized
            prerelease_parts = None
        major, minor, patch = (int(part) for part in core.split("."))
        return (major, minor, patch), prerelease_parts

    left_core, left_pre = parse(left)
    right_core, right_pre = parse(right)
    if left_core != right_core:
        return -1 if left_core < right_core else 1
    if left_pre is None and right_pre is None:
        return 0
    if left_pre is None:
        return 1
    if right_pre is None:
        return -1
    for lval, rval in zip(left_pre, right_pre):
        lkey = _parse_ident(lval)
        rkey = _parse_ident(rval)
        if lkey == rkey:
            continue
        return -1 if lkey < rkey else 1
    if len(left_pre) == len(right_pre):
        return 0
    return -1 if len(left_pre) < len(right_pre) else 1


def build_cases() -> list[tuple[str, str, int]]:
    rng = hidden_rng()
    cases = [
        ("1.0.0-beta.1", "1.0.0", -1),
        ("1.0.0-alpha", "1.0.0-alpha.1", -1),
        ("1.0.0+build.1", "1.0.0+build.9", 0),
    ]
    prerelease_heads = ["alpha", "beta", "rc"]
    prerelease_tails = ["1", "2", "11", "x"]
    for _ in range(12):
        major = rng.randint(0, 3)
        minor = rng.randint(0, 5)
        patch = rng.randint(0, 8)
        head = rng.choice(prerelease_heads)
        tail_a = rng.choice(prerelease_tails)
        tail_b = rng.choice(prerelease_tails)
        left = f"{major}.{minor}.{patch}-{head}.{tail_a}"
        right = f"{major}.{minor}.{patch}-{head}.{tail_b}"
        cases.append((left, right, compare_semver(left, right)))
    for _ in range(6):
        major = rng.randint(0, 3)
        minor = rng.randint(0, 5)
        patch = rng.randint(0, 8)
        left = f"v{major}.{minor}.{patch}+build.{rng.randint(1, 9)}"
        right = f"{major}.{minor}.{patch}+build.{rng.randint(10, 99)}"
        cases.append((left, right, 0))
    return cases


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_semver_compare_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    lines = []
    for left, right, expected in build_cases():
        lines.append(
            f"assert.equal(mod.compareVersions({json.dumps(left)}, {json.dumps(right)}), {expected});"
        )
        lines.append(
            f"assert.equal(mod.compareVersions({json.dumps(right)}, {json.dumps(left)}), {-expected});"
        )
    run_bun_assertions(workspace / "compare.ts", "\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
