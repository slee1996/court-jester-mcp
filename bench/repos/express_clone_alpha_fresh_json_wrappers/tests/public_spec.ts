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
