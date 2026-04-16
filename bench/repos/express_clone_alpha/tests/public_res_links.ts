import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (_req, res) => {
  res.links({ next: "http://api.example.com/users?page=2" });
  res.links({ prev: "http://api.example.com/users?page=0" });
  res.sendStatus(204);
});

const response = await invoke(app, { url: "/" });
assert.equal(
  response.headers.link,
  '<http://api.example.com/users?page=2>; rel="next", <http://api.example.com/users?page=0>; rel="prev"',
);
