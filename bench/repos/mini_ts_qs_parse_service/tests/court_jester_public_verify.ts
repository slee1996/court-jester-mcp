import assert from "node:assert/strict";

import { parseQuery } from "../parse.ts";

assert.deepEqual(parseQuery("filter[city]=Paris&filter[zip]=75001"), {
  filter: { city: "Paris", zip: "75001" },
});
assert.deepEqual(parseQuery("a[]=b&a[]=c"), { a: ["b", "c"] });
