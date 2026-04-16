import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).json());
app.post("/", (req, res) => {
  res.send(req.body === undefined ? "undefined" : JSON.stringify(req.body));
});

const parsed = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/json" },
  body: '{"name":"tj","roles":["admin"]}',
});
assert.equal(parsed.body, JSON.stringify({ name: "tj", roles: ["admin"] }));

const skipped = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "text/plain" },
  body: '{"name":"tj"}',
});
assert.equal(skipped.body, "undefined");
