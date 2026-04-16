import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(stringifyQuery({ items: [], q: "alpha" }), "q=alpha");
assert.equal(stringifyQuery({ filter: { city: "" } }), "filter%5Bcity%5D=");
