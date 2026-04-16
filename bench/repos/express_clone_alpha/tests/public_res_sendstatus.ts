import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (_req, res) => {
  res.sendStatus(201);
});

const response = await invoke(app, { url: "/" });
assert.equal(response.statusCode, 201);
assert.equal(response.body, "Created");
assert.equal(response.headers["content-type"], "text/plain; charset=utf-8");
