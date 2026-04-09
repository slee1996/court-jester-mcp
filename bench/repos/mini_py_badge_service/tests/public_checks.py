from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from badges import badge_for_referrals


assert badge_for_referrals(5) == "gold"
assert badge_for_referrals(3) == "silver"
assert badge_for_referrals(1) == "starter"
