import assert from "node:assert/strict";

import { chunk, flatten, uniq } from "../array.ts";

const values = [0, 1, 2, 3, 4, 5];

assert.deepEqual(chunk(values, 4), [
  [0, 1, 2, 3],
  [4, 5],
]);
assert.deepEqual(chunk(values), [[0], [1], [2], [3], [4], [5]]);
assert.deepEqual(chunk(values, false as unknown as number), []);
assert.deepEqual(flatten([[], [[]], [[], [[[]]]]]), [[], [], [[[]]]]);
assert.deepEqual(uniq([1, 2, 2]), [1, 2]);
