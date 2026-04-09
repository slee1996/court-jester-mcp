import assert from "node:assert/strict";

import { maxStableVersion } from "../versions.ts";

assert.equal(maxStableVersion(["1.0.0", "1.2.0", "1.1.9"]), "1.2.0");
assert.equal(maxStableVersion([null, "v2.0.0", "1.9.9"]), "2.0.0");
