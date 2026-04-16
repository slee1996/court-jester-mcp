import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const htmlApp = express();
htmlApp.use((req, res) => {
  res.redirect("<la'me>");
});

const htmlResponse = await invoke(htmlApp, { headers: { accept: "text/html" } });
assert.equal(htmlResponse.headers.location, "%3Cla'me%3E");
assert.equal(
  htmlResponse.body,
  "<!DOCTYPE html><head><title>Found</title></head><body><p>Found. Redirecting to %3Cla&#39;me%3E</p></body>",
);

const binaryApp = express();
binaryApp.use((req, res) => {
  res.redirect("http://google.com");
});

const binaryResponse = await invoke(binaryApp, { headers: { accept: "application/octet-stream" } });
assert.equal(binaryResponse.body, "");
assert.equal(binaryResponse.headers["content-type"], undefined);
