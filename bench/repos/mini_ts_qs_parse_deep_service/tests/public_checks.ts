import assert from "node:assert/strict";

import { parseQuery } from "../parse.ts";

assert.deepEqual(parseQuery("tag=pro&tag=beta"), { tag: ["pro", "beta"] });
assert.deepEqual(parseQuery("filter[city]=Paris&filter[tags][]=pro&filter[tags][]=beta"), {
  filter: { city: "Paris", tags: ["pro", "beta"] },
});
