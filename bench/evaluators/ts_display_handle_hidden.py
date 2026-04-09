from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_display_handle_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "handle.ts",
        """
assert.equal(mod.displayHandle(null), "guest");
assert.equal(mod.displayHandle({ profile: null, username: "Spencer" }), "spencer");
assert.equal(mod.displayHandle({ profile: { handle: "   " }, username: " Admin " }), "admin");
assert.equal(mod.displayHandle({ profile: { handle: "   " }, username: "   " }), "guest");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
