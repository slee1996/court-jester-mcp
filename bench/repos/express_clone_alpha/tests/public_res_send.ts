import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res) => {
  res.send(1000);
});

const response = await invoke(app);
assert.equal(response.statusCode, 200);
expectHeader(response, "content-type", "application/json; charset=utf-8");
assert.equal(response.body, "1000");
