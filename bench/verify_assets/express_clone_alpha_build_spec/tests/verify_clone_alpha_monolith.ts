import assert from "node:assert/strict";
import path from "node:path";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  app.use((req, res, next) => {
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

{
  const app = express();
  app.get("/", (_req, res) => {
    res.sendStatus(204);
  });
  const response = await invoke(app, { url: "/" });
  assert.equal(response.statusCode, 204);
  assert.equal(response.body, "");
}

{
  const app = express();
  app.get("/", (_req, res) => {
    res.location("back");
    res.sendStatus(204);
  });
  const response = await invoke(app, {
    url: "/",
    headers: { Referer: "https://example.test/docs?page=1" },
  });
  assert.equal(response.headers.location, "https://example.test/docs?page=1");
}

{
  const app = express();
  app.get("/", (_req, res) => {
    res.links({ next: "http://api.example.com/users?page=2" });
    res.links({ last: "http://api.example.com/users?page=5" });
    res.sendStatus(204);
  });
  const response = await invoke(app, { url: "/" });
  assert.equal(
    response.headers.link,
    '<http://api.example.com/users?page=2>; rel="next", <http://api.example.com/users?page=5>; rel="last"',
  );
}

{
  const root = path.join(import.meta.dir, "..", "static");
  const app = express();
  app.use((express as any).static(root));
  const response = await invoke(app, { method: "HEAD", url: "/hello.txt" });
  assert.equal(response.statusCode, 200);
  assert.equal(response.body, "");
}
