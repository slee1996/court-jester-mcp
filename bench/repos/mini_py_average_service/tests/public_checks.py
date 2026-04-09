from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from metrics import average_order_value


assert average_order_value(1200, 3) == 4.0
assert average_order_value(999, 1) == 9.99
