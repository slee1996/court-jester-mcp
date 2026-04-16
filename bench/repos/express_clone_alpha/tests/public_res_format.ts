import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res, next) => {
  res.format({
    text: () => {
      res.send("hey");
    },
    html: () => {
      res.send("<p>hey</p>");
    },
    json: () => {
      res.send({ message: "hey" });
    },
  });
});
app.use((err, req, res, next) => {
  res.status(err.status);
  res.send(`Supports: ${err.types.join(", ")}`);
});

const jsonResponse = await invoke(app, {
  headers: { accept: "text/html; q=.5, application/json, */*; q=.1" },
});
expectHeader(jsonResponse, "content-type", "application/json; charset=utf-8");
assert.equal(jsonResponse.body, JSON.stringify({ message: "hey" }));

const plainResponse = await invoke(app, { headers: { accept: "text/html; q=.5, text/plain" } });
expectHeader(plainResponse, "vary", "Accept");
expectHeader(plainResponse, "content-type", "text/plain; charset=utf-8");
assert.equal(plainResponse.body, "hey");
