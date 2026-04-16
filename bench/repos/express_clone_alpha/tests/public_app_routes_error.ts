import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
let a = false;
let b = false;
let c = false;
let d = false;

app.get(
  "/",
  (req, res, next) => {
    next(new Error("fabricated error"));
  },
  (req, res, next) => {
    a = true;
    next();
  },
  (err, req, res, next) => {
    b = true;
    assert.equal(err.message, "fabricated error");
    next(err);
  },
  (err, req, res, next) => {
    c = true;
    assert.equal(err.message, "fabricated error");
    next();
  },
  (err, req, res, next) => {
    d = true;
    next();
  },
  (req, res) => {
    assert.equal(a, false);
    assert.equal(b, true);
    assert.equal(c, true);
    assert.equal(d, false);
    res.sendStatus(204);
  },
);

const response = await invoke(app, { url: "/" });
assert.equal(response.statusCode, 204);
