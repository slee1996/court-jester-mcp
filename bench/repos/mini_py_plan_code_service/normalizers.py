def normalize_plan_code(value: str | None) -> str:
    if not isinstance(value, str):
        return ""
    return value.strip().upper()
