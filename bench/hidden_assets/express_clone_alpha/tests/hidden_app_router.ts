import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();

app.get("/:name", (req, res) => {
  res.send(req.params.name);
});

const unicodeResponse = await invoke(app, { url: "/%ce%b1" });
assert.equal(unicodeResponse.body, "α");

const plusResponse = await invoke(app, { url: "/foo+bar" });
assert.equal(plusResponse.body, "foo+bar");
