import assert from "node:assert/strict";

import { defaults, omit, pick } from "../object.ts";

assert.deepEqual(defaults({ a: 1, b: 2 }, { b: 3 }, { c: 3 }), { a: 1, b: 2, c: 3 });
assert.equal(defaults({ a: null as number | null }, { a: 1 }).a, null);

assert.deepEqual(omit({ a: 1, b: 2, c: 3, d: 4 }, ["a", "d"], "c"), { b: 2 });
assert.deepEqual(omit({ a: 1, b: { c: 2, d: 3 } }, "b.c"), { a: 1, b: { d: 3 } });

assert.deepEqual(pick({ a: 1, b: 2, c: 3, d: 4 }, ["a", "d"], "c"), { a: 1, c: 3, d: 4 });
assert.deepEqual(pick({ a: 1, b: { c: 2, d: 3 } }, "b.c"), { b: { c: 2 } });
