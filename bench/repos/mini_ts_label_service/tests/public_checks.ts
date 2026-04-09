import assert from "node:assert/strict";

import { secondaryLabel } from "../labels.ts";

assert.equal(secondaryLabel(["Urgent", "Finance"]), "finance");
assert.equal(secondaryLabel(["Internal", "Ops"]), "ops");
