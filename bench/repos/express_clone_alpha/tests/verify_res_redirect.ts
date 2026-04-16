import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res) => {
  res.redirect("<la'me>");
});

const response = await invoke(app, {
  headers: { accept: "text/html", host: "http://example.com" },
});
assert.equal(response.statusCode, 302);
expectHeader(response, "location", "%3Cla'me%3E");
assert.equal(
  response.body,
  "<!DOCTYPE html><head><title>Found</title></head><body><p>Found. Redirecting to %3Cla&#39;me%3E</p></body>",
);
