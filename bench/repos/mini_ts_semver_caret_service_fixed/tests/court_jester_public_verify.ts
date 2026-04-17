import assert from "node:assert/strict";

import { matchesCaret } from "../range.ts";

assert.equal(matchesCaret("1.9.9", "^1.2.3"), true);
assert.equal(matchesCaret("0.2.5", "^0.2.3"), true);
