from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from plans import primary_plan_code

assert primary_plan_code(None) == "FREE"
assert primary_plan_code({"plans": []}) == "FREE"
assert primary_plan_code({"plans": [" pro "]}) == "PRO"
assert primary_plan_code({"plans": ["TEAM"]}) == "TEAM"
assert primary_plan_code({"plans": ["   ", " team "]}) == "TEAM"
assert primary_plan_code({"plans": [None, " pro "]}) == "PRO"
