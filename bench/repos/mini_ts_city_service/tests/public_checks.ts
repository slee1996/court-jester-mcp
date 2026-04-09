import assert from "node:assert/strict";

import { primaryCity } from "../location.ts";

assert.equal(primaryCity({ address: { city: "Denver" } }), "Denver");
assert.equal(primaryCity({ address: { city: " Seattle " } }), "Seattle");
