import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();

app.param("id", (req, res, next, id) => {
  const numeric = Number(id);
  if (Number.isNaN(numeric)) {
    next("route");
    return;
  }
  req.params.id = String(numeric);
  next();
});

app.get("/user/:id", (req, res) => {
  res.send(req.params.id);
});

app.get("/:name/123", (req, res) => {
  res.send("name");
});

const userResponse = await invoke(app, { url: "/user/123" });
assert.equal(userResponse.body, "123");
