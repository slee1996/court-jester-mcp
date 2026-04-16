import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
const sub1 = express.Router();
const sub2 = express.Router();
const sub3 = express.Router();

sub3.get("/:d", (req, res, next) => {
  req.trace = (req.trace || []).concat(`0@${req.baseUrl}`);
  next();
});
sub2.use("/:c", sub3);
sub1.use("/", (req, res, next) => {
  req.trace = (req.trace || []).concat(`1@${req.baseUrl}`);
  next();
});
sub1.use("/bar", sub2);
sub1.use("/bar", (req, res, next) => {
  req.trace.push(`2@${req.baseUrl}`);
  next();
});
app.use((req, res, next) => {
  req.trace = [`3@${req.baseUrl}`];
  next();
});
app.use("/:a", sub1);
app.use((req, res) => {
  req.trace.push(`4@${req.baseUrl}`);
  res.send(req.trace.join(","));
});

const response = await invoke(app, { url: "/foo/bar/baz/zed" });
assert.equal(response.body, "3@,1@/foo,0@/foo/bar/baz,2@/foo/bar,4@");
