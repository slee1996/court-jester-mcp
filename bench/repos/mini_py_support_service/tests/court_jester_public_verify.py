assert support_email_domain({"contacts": {"support_email": "ops@example.com"}}) == "example.com"
assert support_email_domain({"contacts": {"support_email": "HELP@Travel.test"}}) == "travel.test"
