import assert from "node:assert/strict";

import { compareVersions } from "../compare.ts";

assert.equal(compareVersions("1.2.3", "1.2.3"), 0);
assert.equal(compareVersions("1.2.3+build.1", "1.2.3+build.9"), 0);
