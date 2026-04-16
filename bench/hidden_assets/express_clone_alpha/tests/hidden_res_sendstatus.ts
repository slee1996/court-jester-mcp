import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (_req, res) => {
  res.sendStatus(500);
});

const response = await invoke(app, { url: "/" });
assert.equal(response.statusCode, 500);
assert.equal(response.body, "Internal Server Error");
