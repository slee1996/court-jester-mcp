import { canonicalQuery } from "../query.ts";

if (canonicalQuery({ q: "alpha beta", page: 2 }) !== "page=2&q=alpha%20beta") {
  throw new Error("expected canonical alpha beta query");
}

if (canonicalQuery({ tag: ["pro", "beta"] }) !== "tag=pro&tag=beta") {
  throw new Error("expected repeated tags");
}
