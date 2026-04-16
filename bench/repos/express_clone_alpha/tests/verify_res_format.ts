import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.use((req, res, next) => {
  res.format({
    "text/plain; charset=utf-8": () => {
      res.send("hey");
    },
    "text/html; foo=bar; bar=baz": () => {
      res.send("<p>hey</p>");
    },
    "application/json; q=0.5": () => {
      res.send({ message: "hey" });
    },
  });
});
app.use((err, req, res, next) => {
  res.status(err.status);
  res.send(`Supports: ${err.types.join(", ")}`);
});

const response = await invoke(app, { headers: { accept: "foo/bar" } });
assert.equal(response.statusCode, 406);
expectHeader(response, "content-type", "text/plain; charset=utf-8");
assert.equal(response.body, "Supports: text/plain, text/html, application/json");
