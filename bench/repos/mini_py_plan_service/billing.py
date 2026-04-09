PLAN_PRICES = {
    "starter": 0,
    "pro": 1900,
    "team": 4900,
}


def plan_price(plan: str | None) -> int:
    normalized = plan.strip().lower()
    return PLAN_PRICES[normalized]
