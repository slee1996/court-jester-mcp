import assert from "node:assert/strict";

import { billingCountry } from "../billing.ts";

assert.equal(billingCountry({ billing: { country: "us" } }), "US");
assert.equal(billingCountry({ billing: { country: " ca " } }), "CA");
