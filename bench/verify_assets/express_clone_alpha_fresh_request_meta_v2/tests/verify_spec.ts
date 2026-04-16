import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

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

{
  const app = express();
  app.enable("trust proxy");
  app.get("/", (req, res) => {
    res.json({ protocol: req.protocol, secure: req.secure });
  });
  const response = await invoke(app, {
    url: "/",
    headers: { "X-Forwarded-Proto": "https, http" },
  });
  assert.equal(response.body, JSON.stringify({ protocol: "https", secure: true }));
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
