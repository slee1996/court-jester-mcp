if (primaryCity({ address: { city: "Denver" } }) !== "Denver") {
  throw new Error("expected Denver");
}

if (primaryCity({ address: { city: " Seattle " } }) !== "Seattle") {
  throw new Error("expected Seattle");
}
