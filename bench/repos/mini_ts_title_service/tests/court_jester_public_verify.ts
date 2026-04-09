if (primaryTitle(["Welcome"]) !== "Welcome") {
  throw new Error("expected Welcome");
}

if (primaryTitle(["  News "]) !== "News") {
  throw new Error("expected trimmed title");
}
