assert normalize_display_name("spencer") == "Spencer"
assert normalize_display_name(" Spence ") == "Spence"
assert normalize_display_name(None) == "Anonymous"
