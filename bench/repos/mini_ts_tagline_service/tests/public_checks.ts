import assert from "node:assert/strict";

import { primaryTagline } from "../tagline.ts";

assert.equal(primaryTagline({ segments: [" Launch ", "Ignore me"] }), "Launch");
assert.equal(primaryTagline({ segments: ["Focus"] }), "Focus");
