import assert from "node:assert/strict";

import { primaryEmailDomain } from "../email.ts";

assert.equal(primaryEmailDomain({ emails: ["owner@example.com"] }), "example.com");
assert.equal(primaryEmailDomain({ emails: ["team@acme.io"] }), "acme.io");
