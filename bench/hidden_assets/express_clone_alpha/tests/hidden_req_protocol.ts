import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.json({
    protocol: req.protocol,
    secure: req.secure,
    xhr: req.xhr,
  });
});

const encrypted = await invoke(app, {
  url: "/",
  headers: { "X-Requested-With": "XMLHttpRequest" },
});
assert.equal(encrypted.body, JSON.stringify({ protocol: "http", secure: false, xhr: true }));
