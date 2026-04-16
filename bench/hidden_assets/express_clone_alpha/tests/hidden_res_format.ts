import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.use((req, res, next) => {
  res.format({
    text: () => {
      res.send("hey");
    },
    json: () => {
      res.send({ message: "hey" });
    },
    default: function () {
      this.json({ message: "default" });
    },
  });
});
app.use((err, req, res, next) => {
  res.status(err.status);
  res.send(`Supports: ${err.types.join(", ")}`);
});

const fallback = await invoke(app, { headers: { accept: "text/html" } });
assert.equal(fallback.body, JSON.stringify({ message: "default" }));

const noMatchApp = express();
noMatchApp.use((req, res, next) => {
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
noMatchApp.use((err, req, res, next) => {
  res.status(err.status);
  res.send(`Supports: ${err.types.join(", ")}`);
});

const noMatch = await invoke(noMatchApp, { headers: { accept: "foo/bar" } });
assert.equal(noMatch.statusCode, 406);
assert.equal(noMatch.body, "Supports: text/plain, text/html, application/json");
