import assert from "node:assert/strict";

import { maxSatisfying } from "../versions.ts";

assert.equal(maxSatisfying(["1.3.0-beta.1", "1.2.9"], "^1.2.3"), "1.2.9");
assert.equal(maxSatisfying(["0.0.3", "0.0.4"], "^0.0.3"), "0.0.3");
