import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
const router = new express.Router();

function handler1(req, res, next) {
  res.setHeader("x-user-id", String(req.params.id));
  next();
}

function handler2(req, res) {
  res.send(req.params.id);
}

router.use((req, res, next) => {
  res.setHeader("x-router", String(req.params.id));
  next();
});

app.get("/user/:id", handler1, router, handler2);

const response = await invoke(app, { url: "/user/1" });
assert.equal(response.headers["x-router"], "undefined");
assert.equal(response.headers["x-user-id"], "1");
assert.equal(response.body, "1");
