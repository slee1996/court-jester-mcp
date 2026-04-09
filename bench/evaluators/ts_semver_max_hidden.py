from __future__ import annotations

import json
import sys
from pathlib import Path

from helpers import hidden_rng, run_bun_assertions


def parse_version(value: str) -> tuple[int, int, int] | None:
    if not isinstance(value, str):
        return None
    normalized = value.strip().removeprefix("v")
    if not normalized:
        return None
    normalized = normalized.split("+", 1)[0]
    if "-" in normalized:
        return None
    parts = normalized.split(".")
    if len(parts) != 3:
        return None
    try:
        major, minor, patch = (int(part) for part in parts)
    except ValueError:
        return None
    if min(major, minor, patch) < 0:
        return None
    return major, minor, patch


def max_stable_expected(values: list[str | None]) -> str | None:
    best: tuple[int, int, int] | None = None
    best_text: str | None = None
    for raw in values:
        parsed = parse_version(raw)
        if parsed is None:
            continue
        if best is None or parsed > best:
            best = parsed
            best_text = ".".join(str(part) for part in parsed)
    return best_text


def build_cases() -> list[tuple[list[str | None], str | None]]:
    rng = hidden_rng()
    cases = [
        (["1.4.0-beta.2", "1.3.9"], "1.3.9"),
        (["v1.0.0+build.7", "1.0.0+build.9"], "1.0.0"),
        ([None, "", "v2.1.0-rc.1", "2.0.5"], "2.0.5"),
    ]
    for _ in range(12):
        values: list[str | None] = []
        for _ in range(rng.randint(3, 6)):
            major = rng.randint(0, 3)
            minor = rng.randint(0, 5)
            patch = rng.randint(0, 8)
            variant = rng.random()
            value = f"{major}.{minor}.{patch}"
            if variant < 0.2:
                values.append(None)
            elif variant < 0.4:
                values.append(f"v{value}")
            elif variant < 0.6:
                values.append(f"{value}+build.{rng.randint(1, 9)}")
            elif variant < 0.8:
                values.append(f"{value}-beta.{rng.randint(1, 4)}")
            else:
                values.append(value)
        cases.append((values, max_stable_expected(values)))
    return cases


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_semver_max_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    lines = []
    for values, expected in build_cases():
        expected_literal = "null" if expected is None else json.dumps(expected)
        lines.append(
            f"assert.equal(mod.maxStableVersion({json.dumps(values)}), {expected_literal});"
        )
    run_bun_assertions(workspace / "versions.ts", "\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
