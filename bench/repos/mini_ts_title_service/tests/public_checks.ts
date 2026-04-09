import assert from "node:assert/strict";

import { primaryTitle } from "../title.ts";

assert.equal(primaryTitle(["Welcome"]), "Welcome");
assert.equal(primaryTitle(["  News "]), "News");
