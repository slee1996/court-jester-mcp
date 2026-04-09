import assert from "node:assert/strict";

import { displayInitials } from "../initials.ts";

assert.equal(displayInitials("Spencer Lee"), "S.L");
assert.equal(displayInitials("Nova"), "N");
