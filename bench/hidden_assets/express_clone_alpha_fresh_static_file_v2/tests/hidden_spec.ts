import assert from "node:assert/strict";
import path from "node:path";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const root = path.join(import.meta.dir, "..", "static");
  const app = express();
  app.use((express as any).static(root));
  const response = await invoke(app, { url: "/hello.txt" });
  assert.equal(response.headers["content-type"], "text/plain; charset=utf-8");
}
