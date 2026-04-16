import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  app.use((express as any).json());
  app.post("/", (req, res) => {
    res.send(JSON.stringify(req.body));
  });
  const response = await invoke(app, {
    method: "POST",
    url: "/",
    headers: { "content-type": "application/json; charset=utf-8" },
    body: '{"project":"express"}',
  });
  assert.equal(response.body, JSON.stringify({ project: "express" }));
}

{
  const app = express();
  app.use((express as any).text());
  app.post("/", (req, res) => {
    res.send(req.body);
  });
  const response = await invoke(app, {
    method: "POST",
    url: "/",
    headers: { "content-type": "text/plain; charset=utf-8" },
    body: "alpha\nbeta",
  });
  assert.equal(response.body, "alpha\nbeta");
}

{
  const app = express();
  app.use((express as any).raw());
  app.post("/", (req, res) => {
    res.json({ isBuffer: Buffer.isBuffer(req.body), value: req.body.toString("utf8") });
  });
  const response = await invoke(app, {
    method: "POST",
    url: "/",
    headers: { "content-type": "application/octet-stream; charset=utf-8" },
    body: "beta",
  });
  assert.equal(response.body, JSON.stringify({ isBuffer: true, value: "beta" }));
}
