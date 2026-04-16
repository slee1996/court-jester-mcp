import assert from "node:assert/strict";

import express from "../index.ts";
import { expectHeader, invoke } from "./harness.ts";

const app = express();
app.enable("json escape");
app.use((req, res) => {
  res.json({ "&": "<script>" });
});

const response = await invoke(app);
expectHeader(response, "content-type", "application/json; charset=utf-8");
assert.equal(response.body, '{"\\u0026":"\\u003cscript\\u003e"}');
