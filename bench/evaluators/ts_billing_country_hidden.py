from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_billing_country_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "billing.ts",
        """
assert.equal(mod.billingCountry(null), "US");
assert.equal(mod.billingCountry({ billing: null }), "US");
assert.equal(mod.billingCountry({ billing: { country: "   " } }), "US");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
