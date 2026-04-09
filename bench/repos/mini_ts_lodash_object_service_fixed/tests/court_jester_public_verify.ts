import assert from "node:assert/strict";

import { defaults, omit, pick } from "../object.ts";

assert.deepEqual(defaults({ a: 1 }, { a: 2, b: 2 }), { a: 1, b: 2 });
assert.deepEqual(omit({ a: 1, b: 2, c: 3, d: 4 }, "a", "c"), { b: 2, d: 4 });
assert.deepEqual(pick({ a: 1, b: 2, c: 3, d: 4 }, "a", "c"), { a: 1, c: 3 });
