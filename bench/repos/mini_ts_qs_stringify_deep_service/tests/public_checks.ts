import assert from "node:assert/strict";

import { stringifyQuery } from "../stringify.ts";

assert.equal(stringifyQuery({ page: 2, tag: ["pro", "beta"] }), "page=2&tag=pro&tag=beta");
assert.equal(
  stringifyQuery({ filter: { city: "Paris", tags: ["pro", "beta"] } }),
  "filter%5Bcity%5D=Paris&filter%5Btags%5D%5B%5D=pro&filter%5Btags%5D%5B%5D=beta",
);
