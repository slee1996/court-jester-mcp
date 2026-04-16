import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

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
    body: "team[members][0][name]=tj&team[members][1][name]=simon",
  });
  assert.equal(
    response.body,
    JSON.stringify({ team: { members: [{ name: "tj" }, { name: "simon" }] } }),
  );
}
