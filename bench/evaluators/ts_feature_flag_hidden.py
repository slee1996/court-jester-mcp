from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_feature_flag_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "flags.ts",
        """
assert.equal(mod.betaCheckoutEnabled({ flags: { betaCheckout: false } }), false);
assert.equal(mod.betaCheckoutEnabled({ flags: null }), true);
assert.equal(mod.betaCheckoutEnabled({ flags: { betaCheckout: null } }), true);
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
