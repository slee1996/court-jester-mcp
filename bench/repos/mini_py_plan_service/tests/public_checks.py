from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from billing import plan_price


assert plan_price("starter") == 0
assert plan_price(" pro ") == 1900
assert plan_price("TEAM") == 4900
