from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_preferred_timezone_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "timezone.ts",
        """
assert.equal(mod.preferredTimezone(null), "UTC");
assert.equal(mod.preferredTimezone({ preferences: null }), "UTC");
assert.equal(mod.preferredTimezone({ preferences: { timezone: "   " } }), "UTC");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
