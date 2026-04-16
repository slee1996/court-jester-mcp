import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  app.use((req, _res, next) => {
    if (req.method !== "POST") {
      next();
      return;
    }
    req.method = "DELETE";
    next();
  });
  app.delete("/", (_req, res) => {
    res.send("deleted");
  });
  const response = await invoke(app, { method: "POST", url: "/" });
  assert.equal(response.body, "deleted");
}

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
