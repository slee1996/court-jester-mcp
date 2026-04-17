import assert from "node:assert/strict";

import { betaCheckoutEnabled } from "../flags.ts";

assert.equal(betaCheckoutEnabled(null), true);
assert.equal(betaCheckoutEnabled({ flags: { betaCheckout: true } }), true);
