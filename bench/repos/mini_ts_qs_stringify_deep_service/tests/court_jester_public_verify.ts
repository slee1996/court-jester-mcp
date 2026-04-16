import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(
  stringifyQuery({ filter: { nested: { city: "Denver" }, tags: ["a", "b"] } }),
  "filter%5Bnested%5D%5Bcity%5D=Denver&filter%5Btags%5D%5B%5D=a&filter%5Btags%5D%5B%5D=b",
);
assert.equal(stringifyQuery({ filter: { city: "", zip: null } }), "filter%5Bcity%5D=");
