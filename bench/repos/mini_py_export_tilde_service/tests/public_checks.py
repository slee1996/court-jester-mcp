from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from export_name import export_filename


assert export_filename("Quarterly Revenue") == "quarterly~revenue.csv"
assert export_filename(" Team Report ") == "team~report.csv"
