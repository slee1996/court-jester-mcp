import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const root = fileURLToPath(new URL("../static", import.meta.url));
  const app = express();
  app.use((express as any).static(root));
  const response = await invoke(app, { url: "/hello.txt" });
  assert.equal(response.headers["content-type"], "text/plain; charset=utf-8");
}
