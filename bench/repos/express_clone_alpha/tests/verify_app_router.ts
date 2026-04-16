import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();

app.use((req, res, next) => {
  if (req.method !== "POST") {
    next();
    return;
  }
  req.method = "DELETE";
  res.setHeader("x-method-altered", "1");
  next();
});

app.delete("/", (req, res) => {
  res.send("deleted everything");
});

const response = await invoke(app, { method: "POST", url: "/" });
assert.equal(response.headers["x-method-altered"], "1");
assert.equal(response.body, "deleted everything");
