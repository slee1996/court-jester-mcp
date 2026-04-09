from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_secondary_label_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "labels.ts",
        """
assert.equal(mod.secondaryLabel([]), "general");
assert.equal(mod.secondaryLabel(["Urgent"]), "general");
assert.equal(mod.secondaryLabel(["Urgent", "   "]), "general");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
