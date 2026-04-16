import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).raw());
app.post("/", (req, res) => {
  res.json({
    isBuffer: Buffer.isBuffer(req.body),
    value: Buffer.isBuffer(req.body) ? req.body.toString("utf8") : null,
  });
});

const parsed = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/octet-stream" },
  body: "alpha",
});
assert.equal(parsed.body, JSON.stringify({ isBuffer: true, value: "alpha" }));
