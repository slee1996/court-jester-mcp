from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_display_initials_badge_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "initials.ts",
        """
assert.equal(mod.displayInitials(null), "A.N");
assert.equal(mod.displayInitials(""), "A.N");
assert.equal(mod.displayInitials("   "), "A.N");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
