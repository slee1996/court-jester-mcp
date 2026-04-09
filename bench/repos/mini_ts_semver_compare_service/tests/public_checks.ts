import assert from "node:assert/strict";

import { compareVersions } from "../compare.ts";

assert.equal(compareVersions("1.0.0", "1.0.1"), -1);
assert.equal(compareVersions("2.0.0", "1.9.9"), 1);
assert.equal(compareVersions("v1.2.3", "1.2.3"), 0);
