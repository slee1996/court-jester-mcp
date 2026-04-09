from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_primary_city_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "location.ts",
        """
assert.equal(mod.primaryCity(null), "Unknown");
assert.equal(mod.primaryCity({ address: null }), "Unknown");
assert.equal(mod.primaryCity({ address: { city: "   " } }), "Unknown");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
