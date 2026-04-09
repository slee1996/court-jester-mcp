import assert from "node:assert/strict";

import { canonicalQuery } from "../query.ts";

assert.equal(canonicalQuery({ q: "alpha beta", page: 2 }), "page=2&q=alpha%20beta");
assert.equal(canonicalQuery({ tag: ["pro", "beta"] }), "tag=pro&tag=beta");
