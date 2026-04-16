import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

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
  app.set("query parser", "extended");
  app.get("/", (req, res) => {
    res.json(req.query);
  });
  const response = await invoke(app, {
    url: "/?user[name]=tj&user[roles][0]=admin",
  });
  assert.equal(response.body, JSON.stringify({ user: { name: "tj", roles: ["admin"] } }));
}

{
  const app = express();
  app.get("/", (req, res) => {
    res.json({ via: req.header("x-requested-with"), xhr: req.xhr });
  });
  const response = await invoke(app, {
    url: "/",
    headers: { "X-Requested-With": "XMLHttpRequest" },
  });
  assert.equal(response.body, JSON.stringify({ via: "XMLHttpRequest", xhr: true }));
}
