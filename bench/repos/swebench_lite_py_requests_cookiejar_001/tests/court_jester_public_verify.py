from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from mini_requests import build_cookie_header

assert build_cookie_header({"session": '"two words"'}) == 'session="two words"'
assert build_cookie_header({"theme": "dark", "empty": None}) == "theme=dark"
