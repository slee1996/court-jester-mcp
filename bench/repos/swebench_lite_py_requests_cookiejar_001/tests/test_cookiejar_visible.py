from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from mini_requests import build_cookie_header


class CookieJarVisibleTests(unittest.TestCase):
    def test_already_quoted_value_round_trips(self) -> None:
        self.assertEqual(
            build_cookie_header({"session": '"two words"'}),
            'session="two words"',
        )

    def test_none_values_are_skipped(self) -> None:
        self.assertEqual(build_cookie_header({"theme": "dark", "empty": None}), "theme=dark")


if __name__ == "__main__":
    unittest.main()
