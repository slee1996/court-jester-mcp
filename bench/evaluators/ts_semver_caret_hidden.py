from __future__ import annotations

import json
import sys
from pathlib import Path

from helpers import hidden_rng, run_bun_assertions


def parse_version(value: str) -> tuple[int, int, int, bool]:
    normalized = value.strip().removeprefix("v").split("+", 1)[0]
    has_prerelease = "-" in normalized
    core = normalized.split("-", 1)[0]
    major, minor, patch = (int(part) for part in core.split("."))
    return major, minor, patch, has_prerelease


def matches_caret_expected(version: str, range_text: str) -> bool:
    if not range_text.startswith("^"):
        return False
    major, minor, patch, _ = parse_version(range_text[1:])
    c_major, c_minor, c_patch, c_prerelease = parse_version(version)
    if c_prerelease:
        return False
    if (c_major, c_minor, c_patch) < (major, minor, patch):
        return False
    if major > 0:
        return c_major == major
    if minor > 0:
        return c_major == 0 and c_minor == minor
    return c_major == 0 and c_minor == 0 and c_patch == patch


def build_cases() -> list[tuple[str, str, bool]]:
    rng = hidden_rng()
    cases = [
        ("1.3.0-beta.1", "^1.2.3", False),
        ("0.3.0", "^0.2.3", False),
        ("0.2.9", "^0.2.3", True),
        ("0.0.4", "^0.0.3", False),
    ]
    for _ in range(16):
        base_major = rng.randint(0, 2)
        base_minor = rng.randint(0, 3)
        base_patch = rng.randint(0, 4)
        range_text = f"^{base_major}.{base_minor}.{base_patch}"
        if rng.random() < 0.35:
            candidate = f"{base_major}.{base_minor}.{base_patch}-beta.{rng.randint(1, 4)}"
        else:
            c_major = base_major + rng.randint(0, 1 if base_major > 0 else 0)
            c_minor = rng.randint(0, 4)
            c_patch = rng.randint(0, 6)
            candidate = f"{c_major}.{c_minor}.{c_patch}"
        cases.append((candidate, range_text, matches_caret_expected(candidate, range_text)))
    return cases


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_semver_caret_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    body = "\n".join(
        f"assert.equal(mod.matchesCaret({json.dumps(version)}, {json.dumps(range_text)}), {str(expected).lower()});"
        for version, range_text, expected in build_cases()
    )
    run_bun_assertions(workspace / "range.ts", body)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
