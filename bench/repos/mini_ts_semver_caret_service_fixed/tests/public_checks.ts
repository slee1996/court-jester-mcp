import assert from "node:assert/strict";

import { matchesCaret } from "../range.ts";

assert.equal(matchesCaret("1.4.0", "^1.2.3"), true);
assert.equal(matchesCaret("2.0.0", "^1.2.3"), false);
assert.equal(matchesCaret("1.2.3", "^1.2.3"), true);
