if (supportEmailDomain({ contacts: { supportEmail: "ops@example.com" } }) !== "example.com") {
  throw new Error("expected example.com");
}

if (supportEmailDomain({ contacts: { supportEmail: "HELP@Travel.test" } }) !== "travel.test") {
  throw new Error("expected travel.test");
}
