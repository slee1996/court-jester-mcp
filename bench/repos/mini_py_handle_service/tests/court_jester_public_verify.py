from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from handle import display_handle

assert display_handle({"profile": {"handle": " Admin "}, "username": "root"}) == "admin"
