import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.get("/", (req, res) => {
  res.location("back");
  res.sendStatus(204);
});

const response = await invoke(app, {
  url: "/",
  headers: { Referer: "https://example.test/docs?page=1" },
});
assert.equal(response.headers.location, "https://example.test/docs?page=1");
