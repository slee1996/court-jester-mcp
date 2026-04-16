import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((express as any).text());
app.post("/", (req, res) => {
  res.send(typeof req.body === "string" ? req.body : "undefined");
});

const html = await invoke(app, {
  method: "POST",
  url: "/",
  headers: { "content-type": "text/html" },
  body: "<p>hello</p>",
});
assert.equal(html.body, "<p>hello</p>");
