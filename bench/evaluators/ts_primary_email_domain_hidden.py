from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_primary_email_domain_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "email.ts",
        """
assert.equal(mod.primaryEmailDomain(null), "unknown");
assert.equal(mod.primaryEmailDomain({ emails: null }), "unknown");
assert.equal(mod.primaryEmailDomain({ emails: [] }), "unknown");
assert.equal(mod.primaryEmailDomain({ emails: ["not-an-email"] }), "unknown");
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
