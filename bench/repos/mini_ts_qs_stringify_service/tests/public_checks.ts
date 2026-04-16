import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(stringifyQuery({ page: 2, q: "alpha beta" }), "page=2&q=alpha%20beta");
assert.equal(stringifyQuery({ tag: ["pro", "beta"] }), "tag=pro&tag=beta");
