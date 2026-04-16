import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).json());
app.post("/", (req, res) => {
  res.send(req.body === undefined ? "undefined" : JSON.stringify(req.body));
});

const charset = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/json; charset=utf-8" },
  body: '{"project":"express"}',
});
assert.equal(charset.body, JSON.stringify({ project: "express" }));

const invalid = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/json" },
  body: '{"project":',
});
assert.equal(invalid.statusCode, 400);
assert.match(invalid.body, /invalid|unexpected/i);
