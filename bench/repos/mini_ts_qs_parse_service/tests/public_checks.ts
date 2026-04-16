import assert from "node:assert/strict";

import { parseQuery } from "../parse.ts";

assert.deepEqual(parseQuery("page=2&q=alpha%20beta"), { page: "2", q: "alpha beta" });
assert.deepEqual(parseQuery("tag=pro&tag=beta"), { tag: ["pro", "beta"] });
assert.deepEqual(parseQuery("filter[city]=Paris"), { filter: { city: "Paris" } });
