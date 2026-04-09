import assert from "node:assert/strict";

import { secondarySupportEmail } from "../contact.ts";

assert.equal(
  secondarySupportEmail({
    contacts: { emails: ["owner@example.com", " Support@Example.com "] },
  }),
  "support@example.com",
);
