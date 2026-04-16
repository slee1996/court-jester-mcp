import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const child = express();
  child.get("/post", (req, res) => {
    res.json({ baseUrl: req.baseUrl, url: req.url, path: req.path });
  });
  const app = express();
  app.use("/blog", child);
  const response = await invoke(app, { url: "/blog/post?draft=1" });
  assert.equal(
    response.body,
    JSON.stringify({ baseUrl: "/blog", url: "/post?draft=1", path: "/post" }),
  );
}
