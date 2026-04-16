import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res) => {
  res.set("Content-Type", "text/plain");
  res.send("hey");
});

const response = await invoke(app);
expectHeader(response, "content-type", "text/plain; charset=utf-8");
assert.equal(response.body, "hey");
