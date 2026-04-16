import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  app.get("/", (_req, res) => {
    res.location("/a path/with spaces");
    res.sendStatus(204);
  });
  const response = await invoke(app, { url: "/" });
  assert.equal(response.statusCode, 204);
  assert.equal(response.body, "");
  assert.equal(response.headers.location, "/a%20path/with%20spaces");
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
