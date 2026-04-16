import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const headApp = express();
headApp.use((req, res) => {
  res.send("yay");
});

const headResponse = await invoke(headApp, { method: "HEAD" });
assert.equal(headResponse.body, "");

const bufferApp = express();
bufferApp.use((req, res) => {
  res.send(Buffer.from("hello"));
});

const bufferResponse = await invoke(bufferApp);
assert.equal(bufferResponse.headers["content-type"], "application/octet-stream");
assert.equal(bufferResponse.body, "hello");

const noContentApp = express();
noContentApp.use((req, res) => {
  res.status(204).set("Transfer-Encoding", "chunked").send("foo");
});

const noContentResponse = await invoke(noContentApp);
assert.equal(noContentResponse.statusCode, 204);
assert.equal(noContentResponse.body, "");
assert.equal(noContentResponse.headers["content-type"], undefined);
assert.equal(noContentResponse.headers["transfer-encoding"], undefined);
