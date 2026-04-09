from __future__ import annotations

import sys
from pathlib import Path

from helpers import run_bun_assertions


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: ts_secondary_support_email_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    run_bun_assertions(
        workspace / "contact.ts",
        """
assert.equal(mod.secondarySupportEmail(null), "help@example.com");
assert.equal(mod.secondarySupportEmail({ contacts: null }), "help@example.com");
assert.equal(mod.secondarySupportEmail({ contacts: { emails: ["owner@example.com"] } }), "help@example.com");
assert.equal(
  mod.secondarySupportEmail({ contacts: { emails: ["owner@example.com", "   "] } }),
  "help@example.com",
);
""".strip(),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
