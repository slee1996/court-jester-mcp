import assert from "node:assert/strict";

import { Router } from "../index.ts";

const router = new Router();
let called = false;

router.use((req, res) => {
  called = true;
  res.end("bad");
});

router.handle({ url: "", method: "GET", headers: {} }, {}, assert.ifError);
assert.equal(called, false);

const tallRouter = new Router();
tallRouter.get("/foo", (req, res, next) => {
  req.count = 0;
  next();
});

for (let index = 0; index < 300; index += 1) {
  tallRouter.get("/foo", (req, res, next) => {
    req.count += 1;
    next();
  });
}

let output = "";
tallRouter.get("/foo", (req, res) => {
  res.end(String(req.count));
});

tallRouter.handle(
  { url: "/foo", method: "GET", headers: {} },
  {
    setHeader() {},
    getHeader() {
      return undefined;
    },
    removeHeader() {},
    end(body?: unknown) {
      output = String(body ?? "");
    },
  },
  assert.ifError,
);

assert.equal(output, "300");
