import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((req, res, next) => {
  next(new Error("boom!"));
});
app.get("/bar", (req, res) => {
  res.send("hello, world!");
});

const response = await invoke(app, { method: "POST", url: "/bar" });
assert.equal(response.statusCode, 500);
assert.match(response.body, /Error: boom!/);
