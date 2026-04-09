def support_email_domain(team: dict | None) -> str:
    return team["contacts"]["support_email"].split("@")[1].lower()
