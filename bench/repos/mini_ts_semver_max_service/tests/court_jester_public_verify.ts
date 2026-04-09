import assert from "node:assert/strict";

import { maxStableVersion } from "../versions.ts";

assert.equal(maxStableVersion(["1.0.0+build.1", "1.0.0+build.2"]), "1.0.0");
assert.equal(maxStableVersion(["1.2.0", "1.2.0"]), "1.2.0");
