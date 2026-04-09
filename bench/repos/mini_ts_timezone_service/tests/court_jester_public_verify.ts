if (preferredTimezone({ preferences: { timezone: "UTC" } }) !== "UTC") {
  throw new Error("expected UTC");
}

if (
  preferredTimezone({ preferences: { timezone: " America/Denver " } }) !==
  "America/Denver"
) {
  throw new Error("expected trimmed timezone");
}
