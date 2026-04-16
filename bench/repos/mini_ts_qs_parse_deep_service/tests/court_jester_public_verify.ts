import assert from "node:assert/strict";

import { parseQuery } from "../parse.ts";

assert.deepEqual(parseQuery("filter[nested][city]=Denver&filter[nested][zip]=80202"), {
  filter: { nested: { city: "Denver", zip: "80202" } },
});
assert.deepEqual(parseQuery("filter[tag]=pro&filter[tag]=beta"), {
  filter: { tag: ["pro", "beta"] },
});
