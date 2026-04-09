from normalizers import normalize_plan_code


def primary_plan_code(account: dict | None) -> str:
    return normalize_plan_code(account["plans"][0])
