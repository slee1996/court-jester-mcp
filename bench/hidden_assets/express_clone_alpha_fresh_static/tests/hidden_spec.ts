import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const root = path.join(fileURLToPath(new URL("..", import.meta.url)), "static");
  const app = express();
  app.use((express as any).static(root));
  const response = await invoke(app, { url: "/" });
  assert.equal(response.statusCode, 200);
  assert.equal(response.body, "<h1>express alpha</h1>\n");
  assert.equal(response.headers["content-type"], "text/html; charset=utf-8");
}
