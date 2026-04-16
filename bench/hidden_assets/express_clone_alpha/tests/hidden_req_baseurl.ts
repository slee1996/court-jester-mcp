import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const topLevel = express();
topLevel.get("/:a", (req, res) => {
  res.send(req.baseUrl);
});

const topLevelResponse = await invoke(topLevel, { url: "/foo" });
assert.equal(topLevelResponse.body, "");

const app = express();
const sub1 = express.Router();
const sub2 = express.Router();
const sub3 = express.Router();

sub3.get("/:d", (req, res) => {
  res.send(req.baseUrl);
});
sub2.use("/:c", sub3);
sub1.use("/:b", sub2);
app.use("/:a", sub1);

const nestedResponse = await invoke(app, { url: "/foo/bar/baz/zed" });
assert.equal(nestedResponse.body, "/foo/bar/baz");
