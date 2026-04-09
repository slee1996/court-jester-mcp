import assert from "node:assert/strict";

import { displayInitials } from "../initials.ts";

assert.equal(displayInitials("Spencer Lee"), "SL");
assert.equal(displayInitials("Nova"), "N");
