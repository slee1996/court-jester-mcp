import assert from "node:assert/strict";

import { supportEmailDomain } from "../support.ts";

assert.equal(
  supportEmailDomain({ contacts: { supportEmail: "ops@example.com" } }),
  "example.com",
);
assert.equal(
  supportEmailDomain({ contacts: { supportEmail: "HELP@Travel.test" } }),
  "travel.test",
);
