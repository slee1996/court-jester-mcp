if (primaryEmailDomain({ emails: ["owner@example.com"] }) !== "example.com") {
  throw new Error("expected example.com");
}

if (primaryEmailDomain({ emails: ["team@acme.io"] }) !== "acme.io") {
  throw new Error("expected acme.io");
}
