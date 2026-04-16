import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
let called = 0;
let count = 0;

app.param("user", (req, res, next, user) => {
  called += 1;
  req.user = user;
  next();
});

app.get("/foo/:user", (req, res, next) => {
  count += 1;
  next();
});
app.get("/foo/:user", (req, res, next) => {
  count += 1;
  next();
});
app.use((req, res) => {
  res.send(`${count} ${called} ${req.user}`);
});

const response = await invoke(app, { url: "/foo/bob" });
assert.equal(response.body, "2 1 bob");
