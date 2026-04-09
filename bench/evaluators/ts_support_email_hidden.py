from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_support_email_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "support.ts",
        """
assert.equal(mod.supportEmailDomain(null), "unknown");
assert.equal(mod.supportEmailDomain({ contacts: null }), "unknown");
assert.equal(mod.supportEmailDomain({ contacts: { supportEmail: "ops" } }), "unknown");
assert.equal(mod.supportEmailDomain({ contacts: { supportEmail: "   " } }), "unknown");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
