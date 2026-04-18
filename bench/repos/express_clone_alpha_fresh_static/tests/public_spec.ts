import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const root = path.join(fileURLToPath(new URL("..", import.meta.url)), "static");
  const app = express();
  app.use((express as any).static(root));
  const response = await invoke(app, { url: "/hello.txt" });
  assert.equal(response.statusCode, 200);
  assert.equal(response.body, "hello world\n");
  assert.equal(response.headers["content-type"], "text/plain; charset=utf-8");
}

{
  const root = path.join(fileURLToPath(new URL("..", import.meta.url)), "static");
  const app = express();
  app.use((express as any).static(root));
  app.use((_req, res) => {
    res.status(404).send("fallback");
  });
  const response = await invoke(app, { url: "/../index.ts" });
  assert.equal(response.statusCode, 404);
  assert.equal(response.body, "fallback");
}
