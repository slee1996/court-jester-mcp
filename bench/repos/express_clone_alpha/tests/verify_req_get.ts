import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.json({
    referrer: req.get("Referrer"),
    referer: req.get("referer"),
  });
});

const response = await invoke(app, {
  url: "/",
  headers: { Referer: "https://example.test/docs" },
});
assert.equal(
  response.body,
  JSON.stringify({
    referrer: "https://example.test/docs",
    referer: "https://example.test/docs",
  }),
);
