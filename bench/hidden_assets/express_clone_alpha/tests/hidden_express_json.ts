import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).json());
app.post("/", (req, res) => {
  res.send(req.body === undefined ? "undefined" : JSON.stringify(req.body));
});

const vendor = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/ld+json" },
  body: '{"@id":"urn:test"}',
});
assert.equal(vendor.body, JSON.stringify({ "@id": "urn:test" }));

const primitive = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/json" },
  body: "true",
});
assert.equal(primitive.statusCode, 400);
assert.match(primitive.body, /invalid json body/i);
