import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const simpleApp = express();
simpleApp.use((express as any).urlencoded({ extended: false }));
simpleApp.post("/", (req, res) => {
  res.send(JSON.stringify(req.body));
});

const simple = await invoke(simpleApp, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "tag=one&tag=two",
});
assert.equal(simple.body, JSON.stringify({ tag: ["one", "two"] }));

const extendedApp = express();
extendedApp.use((express as any).urlencoded({ extended: true }));
extendedApp.post("/", (req, res) => {
  res.send(JSON.stringify(req.body));
});

const extended = await invoke(extendedApp, {
  method: "POST",
  url: "/",
  headers: { "content-type": "application/x-www-form-urlencoded" },
  body: "foo[0][bar]=baz&foo[]=done!",
});
assert.equal(extended.body, JSON.stringify({ foo: [{ bar: "baz" }, "done!"] }));
