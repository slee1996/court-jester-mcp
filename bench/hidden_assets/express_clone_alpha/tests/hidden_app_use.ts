import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const blog = express();
const other = express();
const app = express();

blog.get("/", (req, res) => {
  res.send("success");
});

let otherMounted = false;
other.once("mount", (parent) => {
  otherMounted = true;
  assert.equal(parent, app);
});

function fn1(req, res, next) {
  res.setHeader("x-fn-1", "hit");
  next();
}

function fn2(req, res, next) {
  res.setHeader("x-fn-2", "hit");
  next();
}

app.use("/post/:article", fn1, other, fn2, blog);

const response = await invoke(app, { url: "/post/once-upon-a-time" });
assert.equal(otherMounted, true);
assert.equal(response.body, "success");
assert.equal(response.headers["x-fn-1"], "hit");
assert.equal(response.headers["x-fn-2"], "hit");
