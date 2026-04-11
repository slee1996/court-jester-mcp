from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from mini_requests import build_cookie_header


class CookieJarHiddenTests(unittest.TestCase):
    def test_semicolon_values_are_quoted(self) -> None:
        self.assertEqual(build_cookie_header({"prefs": "a;b"}), 'prefs="a;b"')

    def test_comma_values_are_quoted(self) -> None:
        self.assertEqual(build_cookie_header({"prefs": "a,b"}), 'prefs="a,b"')

    def test_cookie_order_is_preserved(self) -> None:
        self.assertEqual(
            build_cookie_header({"theme": "dark", "prefs": "a;b"}),
            'theme=dark; prefs="a;b"',
        )


if __name__ == "__main__":
    unittest.main()
