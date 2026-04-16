import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).text());
app.post("/", (req, res) => {
  res.send(typeof req.body === "string" ? req.body : "undefined");
});

const parsed = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "text/plain" },
  body: "hello world",
});
assert.equal(parsed.body, "hello world");

const skipped = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/json" },
  body: "hello world",
});
assert.equal(skipped.body, "undefined");
