import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.json({
    host: req.get("host"),
    missing: req.get("x-missing") ?? null,
  });
});

const response = await invoke(app, {
  url: "/",
  headers: { Host: "example.test" },
});
assert.equal(response.body, JSON.stringify({ host: "example.test", missing: null }));
