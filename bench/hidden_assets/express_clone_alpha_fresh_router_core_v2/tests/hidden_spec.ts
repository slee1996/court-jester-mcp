import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  const seen: string[] = [];
  app.param("user", (_req, _res, next, value) => {
    seen.push(`user:${value}`);
    next();
  });
  app.param("post", (_req, _res, next, value) => {
    seen.push(`post:${value}`);
    next();
  });
  app.get("/user/:user/post/:post", (_req, res) => {
    res.send("ok");
  });
  await invoke(app, { url: "/user/tj/post/42" });
  assert.deepEqual(seen, ["user:tj", "post:42"]);
}
