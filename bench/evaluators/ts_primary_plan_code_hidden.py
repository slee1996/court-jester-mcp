from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_primary_plan_code_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "plans.ts",
        """
assert.equal(mod.primaryPlanCode(null), "FREE");
assert.equal(mod.primaryPlanCode({ plans: [] }), "FREE");
assert.equal(mod.primaryPlanCode({ plans: ["   ", " team "] }), "TEAM");
assert.equal(mod.primaryPlanCode({ plans: [null, " pro "] }), "PRO");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
