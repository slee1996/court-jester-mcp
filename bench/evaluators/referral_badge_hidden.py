from __future__ import annotations

import sys
from pathlib import Path

from helpers import load_python_module


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: referral_badge_hidden.py <workspace>")

    workspace = Path(sys.argv[1])
    module = load_python_module(workspace / "badges.py", "badges_under_test")
    badge_for_referrals = module.badge_for_referrals
    assert badge_for_referrals(None) == "starter"
    assert badge_for_referrals(0) == "starter"
    assert badge_for_referrals(2) == "silver"
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
