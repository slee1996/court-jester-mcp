def secondary_support_email(account: dict | None) -> str:
    return account["contacts"]["emails"][1].strip().lower()
