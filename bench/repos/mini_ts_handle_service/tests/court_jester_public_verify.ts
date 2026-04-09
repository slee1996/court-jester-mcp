import assert from "node:assert/strict";

import { displayHandle } from "../handle.ts";

assert.equal(
  displayHandle({ profile: { handle: " Admin " }, username: "root" }),
  "admin",
);
