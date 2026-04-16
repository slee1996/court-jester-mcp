import assert from "node:assert/strict";
import path from "node:path";

import express, { Route } from "../index.ts";
import { invoke } from "./harness.ts";

{
  const child = express();
  child.get("/", (_req, res) => {
    res.send("child");
  });
  const app = express();
  app.use("/blog", child);
  app.use((_req, res) => {
    res.status(404).send("fallback");
  });
  const response = await invoke(app, { url: "/blog" });
  assert.equal(response.body, "child");
}

{
  const route = Route("/foo");
  let called = false;
  route.all((_req, _res, next) => {
    called = true;
    next();
  });
  route.dispatch(
    { url: "/foo", headers: {} },
    {
      setHeader() {},
      getHeader() {
        return undefined;
      },
      removeHeader() {},
      end() {},
    },
    assert.ifError,
  );
  assert.equal(called, true);
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
    headers: { "content-type": "application/json" },
    body: '{"name":"tj"}',
  });
  assert.equal(response.body, JSON.stringify({ name: "tj" }));
}

{
  const app = express();
  app.use((express as any).urlencoded({ extended: true }));
  app.post("/", (req, res) => {
    res.send(JSON.stringify(req.body));
  });
  const response = await invoke(app, {
    method: "POST",
    url: "/",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: "user[name]=tj&user[roles][0]=admin",
  });
  assert.equal(response.body, JSON.stringify({ user: { name: "tj", roles: ["admin"] } }));
}

{
  const app = express();
  app.get("/", (req, res) => {
    res.send(req.get("host") ?? "missing");
  });
  const response = await invoke(app, {
    url: "/",
    headers: { Host: "example.test" },
  });
  assert.equal(response.body, "example.test");
}

{
  const app = express();
  app.enable("trust proxy");
  app.get("/", (req, res) => {
    res.json({ protocol: req.protocol, secure: req.secure });
  });
  const response = await invoke(app, {
    url: "/",
    headers: { "X-Forwarded-Proto": "https" },
  });
  assert.equal(response.body, JSON.stringify({ protocol: "https", secure: true }));
}

{
  const app = express();
  app.get("/", (_req, res) => {
    res.location("/a path/with spaces");
    res.sendStatus(204);
  });
  const response = await invoke(app, { url: "/" });
  assert.equal(response.headers.location, "/a%20path/with%20spaces");
}

{
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
}

{
  const app = express();
  app.get("/", (_req, res) => {
    res.vary("Accept-Encoding");
    res.vary("Accept");
    res.sendStatus(204);
  });
  const response = await invoke(app, { url: "/" });
  assert.equal(response.headers.vary, "Accept-Encoding, Accept");
}

{
  const root = path.join(import.meta.dir, "..", "static");
  const app = express();
  app.use((express as any).static(root));
  app.use((_req, res) => {
    res.status(404).send("fallback");
  });
  const response = await invoke(app, { url: "/missing.txt" });
  assert.equal(response.statusCode, 404);
  assert.equal(response.body, "fallback");
}
