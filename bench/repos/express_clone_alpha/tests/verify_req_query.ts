import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const app = express();
app.set("query parser", (input: string) => ({ length: input.length }));
app.use((req, res) => {
  res.json(req.query);
});

const response = await invoke(app, { url: "/?user%5Bname%5D=tj" });
assert.equal(response.body, JSON.stringify({ length: 17 }));
