import assert from "node:assert/strict";

import express, { Router } from "../index.ts";

const app = express();
const mounted = new Router();

mounted.get("/:slug", (req, res) => {
  res.send(req.params.slug);
});

app.use("/post/:article", mounted);

let body = "";
app.handle(
  { url: "/post/once-upon-a-time/chapter-1", method: "GET", headers: {} },
  {
    setHeader() {},
    getHeader() {
      return undefined;
    },
    removeHeader() {},
    end(value?: unknown) {
      body = String(value ?? "");
    },
  },
  assert.ifError,
);

assert.equal(body, "chapter-1");
