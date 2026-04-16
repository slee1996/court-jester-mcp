import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.type("application/vnd.example+json");
  res.json({ hello: "world" });
});

const response = await invoke(app);
expectHeader(response, "content-type", "application/vnd.example+json; charset=utf-8");
assert.equal(response.body, '{"hello":"world"}');
