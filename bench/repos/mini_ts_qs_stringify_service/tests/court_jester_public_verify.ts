import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(stringifyQuery({ filter: { city: "Paris" } }), "filter%5Bcity%5D=Paris");
assert.equal(stringifyQuery({ extra: null, q: "alpha" }), "q=alpha");
