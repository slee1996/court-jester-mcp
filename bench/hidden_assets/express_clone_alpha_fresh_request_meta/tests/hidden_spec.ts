import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

{
  const app = express();
  app.enable("trust proxy");
  app.get("/", (req, res) => {
    res.json({ protocol: req.protocol, secure: req.secure });
  });
  const response = await invoke(app, {
    url: "/",
    headers: { "X-Forwarded-Proto": "https, http" },
  });
  assert.equal(response.body, JSON.stringify({ protocol: "https", secure: true }));
}
