import assert from "node:assert/strict";

import { primaryPlanCode } from "../plans.ts";

assert.equal(primaryPlanCode(null), "FREE");
assert.equal(primaryPlanCode({ plans: [] }), "FREE");
assert.equal(primaryPlanCode({ plans: [" pro "] }), "PRO");
assert.equal(primaryPlanCode({ plans: ["TEAM"] }), "TEAM");
assert.equal(primaryPlanCode({ plans: ["   ", " team "] }), "TEAM");
assert.equal(primaryPlanCode({ plans: [null, " pro "] }), "PRO");
