assert preferred_timezone({"preferences": {"timezone": "UTC"}}) == "UTC"
assert preferred_timezone({"preferences": {"timezone": " America/Denver "}}) == "America/Denver"
