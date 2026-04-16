import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).urlencoded({ extended: true }));
app.post("/", (req, res) => {
  res.send(req.body === undefined ? "undefined" : JSON.stringify(req.body));
});

const parsed = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/x-www-form-urlencoded; charset=utf-8" },
  body: "title=foo+bar&user[name]=tj",
});
assert.equal(parsed.body, JSON.stringify({ title: "foo bar", user: { name: "tj" } }));
