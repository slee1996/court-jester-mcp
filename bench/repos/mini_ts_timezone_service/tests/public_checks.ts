import assert from "node:assert/strict";

import { preferredTimezone } from "../timezone.ts";

assert.equal(preferredTimezone({ preferences: { timezone: "UTC" } }), "UTC");
assert.equal(
  preferredTimezone({ preferences: { timezone: " America/Denver " } }),
  "America/Denver",
);
