import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(stringifyQuery({ a: "", b: null }), "a=");
assert.equal(stringifyQuery({ extra: {}, q: "alpha" }), "q=alpha");
