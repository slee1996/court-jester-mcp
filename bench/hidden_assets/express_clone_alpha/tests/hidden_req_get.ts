import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.send(req.header("x-forwarded-proto") ?? "none");
});

const response = await invoke(app, {
  url: "/",
  headers: { "X-Forwarded-Proto": "https" },
});
assert.equal(response.body, "https");
