import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res) => {
  res.redirect("https://google.com?q=\u2603 §10");
});

const plainResponse = await invoke(app, { headers: { accept: "text/plain, */*" } });
assert.equal(plainResponse.statusCode, 302);
expectHeader(plainResponse, "location", "https://google.com?q=%E2%98%83%20%C2%A710");
expectHeader(plainResponse, "content-type", "text/plain; charset=utf-8");
assert.equal(plainResponse.body, "Found. Redirecting to https://google.com?q=%E2%98%83%20%C2%A710");

const headResponse = await invoke(app, { method: "HEAD" });
assert.equal(headResponse.body, "");
