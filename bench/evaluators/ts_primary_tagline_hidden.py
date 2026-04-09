from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_primary_tagline_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "tagline.ts",
        """
assert.equal(mod.primaryTagline(null), "general");
assert.equal(mod.primaryTagline({ segments: [] }), "general");
assert.equal(mod.primaryTagline({ segments: ["   ", "Growth "] }), "Growth");
assert.equal(mod.primaryTagline({ segments: ["  ", "\\t"] }), "general");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
