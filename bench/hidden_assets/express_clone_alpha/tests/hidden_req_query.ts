import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const custom = express();
custom.set("query parser", (input: string) => ({ length: input.length }));
custom.use((req, res) => {
  res.json(req.query);
});

const customResponse = await invoke(custom, { url: "/?user%5Bname%5D=tj" });
assert.equal(customResponse.body, JSON.stringify({ length: 17 }));

assert.throws(() => {
  const app = express();
  app.set("query parser", "bogus");
});
