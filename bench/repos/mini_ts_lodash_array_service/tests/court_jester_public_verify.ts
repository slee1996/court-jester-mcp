import assert from "node:assert/strict";

import { chunk, flatten, uniq } from "../array.ts";

assert.deepEqual(chunk([0, 1, 2, 3, 4, 5], 3), [
  [0, 1, 2],
  [3, 4, 5],
]);
assert.deepEqual(flatten([1, [2, [3, [4]], 5]]), [1, 2, [3, [4]], 5]);
assert.deepEqual(uniq([2, 1, 2]), [2, 1]);
