from collections.abc import Mapping

from ._quote import format_cookie_value, needs_cookie_quote


def build_cookie_header(cookies: Mapping[str, str | None]) -> str:
    parts: list[str] = []
    for name, value in cookies.items():
        if value is None:
            continue
        rendered = format_cookie_value(value)
        if needs_cookie_quote(rendered):
            parts.append(f"{name}={rendered}")
        else:
            parts.append(f"{name}={rendered}")
    return "; ".join(parts)
