def is_quoted(value: str) -> bool:
    return len(value) >= 2 and value[0] == value[-1] == '"'


def needs_cookie_quote(value: str) -> bool:
    return any(ch in value for ch in " ;,")


def format_cookie_value(value: str) -> str:
    normalized = value.strip()
    if is_quoted(normalized):
        return normalized[1:-1]
    return normalized
