if (secondaryLabel(["Urgent", "Finance"]) !== "finance") {
  throw new Error("expected finance");
}

if (secondaryLabel(["Internal", "Ops"]) !== "ops") {
  throw new Error("expected ops");
}
