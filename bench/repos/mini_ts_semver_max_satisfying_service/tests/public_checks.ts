import assert from "node:assert/strict";

import { maxSatisfying } from "../versions.ts";

assert.equal(maxSatisfying(["1.2.3", "1.4.0", "2.0.0"], "^1.2.3"), "1.4.0");
assert.equal(maxSatisfying(["0.2.3", "0.2.5", "0.3.0"], "^0.2.3"), "0.2.5");
assert.equal(maxSatisfying(["v1.2.3", "1.2.3+build.9"], "1.2.3"), "1.2.3");
