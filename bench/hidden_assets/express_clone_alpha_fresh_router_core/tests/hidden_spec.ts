import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  let seen = "";
  app.param("user", (_req, _res, next, value) => {
    seen += value;
    next();
  });
  app.get("/user/:user", (_req, res) => {
    res.send("ok");
  });
  await invoke(app, { url: "/user/tj" });
  await invoke(app, { url: "/user/tj" });
  assert.equal(seen, "tjtj");
}
