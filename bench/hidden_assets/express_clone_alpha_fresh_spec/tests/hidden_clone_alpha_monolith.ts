import assert from "node:assert/strict";
import path from "node:path";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  let seen = "";
  app.param("user", (req, _res, next, value) => {
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

{
  const app = express();
  app.get("/", (_req, res) => {
    res.vary("Accept-Encoding");
    res.vary("accept-encoding, Accept");
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
  const response = await invoke(app, { url: "/../index.ts" });
  assert.equal(response.statusCode, 404);
  assert.equal(response.body, "fallback");
}
