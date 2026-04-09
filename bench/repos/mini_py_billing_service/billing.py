def billing_country(order: dict | None) -> str:
    return order["billing"]["country"].strip().upper()
