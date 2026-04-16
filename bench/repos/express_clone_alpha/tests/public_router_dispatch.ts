import assert from "node:assert/strict";

import express, { Router } from "../index.ts";

const router = new Router();
const child = new Router();

child.get("/:bar", (req, res) => {
  res.send(`${req.params.bar}`);
});
router.use("/:foo", child);

let captured = "";
router.handle(
  { url: "/test/route", method: "GET", headers: {} },
  {
    setHeader() {},
    getHeader() {
      return undefined;
    },
    removeHeader() {},
    end(body?: unknown) {
      captured = String(body ?? "");
    },
  },
  assert.ifError,
);
assert.equal(captured, "route");

const app = express();
let routeAllCalled = false;
let useCalled = false;
const route = app.route("/foo");

route.post((req, res, next) => {
  next(new Error("should not run"));
});
route.all((req, res, next) => {
  routeAllCalled = true;
  next();
});
route.get((req, res, next) => {
  next(new Error("should not run"));
});

app.use((req, res, next) => {
  useCalled = true;
  next();
});

let methodError: unknown = null;
app.handle(
  { url: "/foo", headers: {} },
  {
    setHeader() {},
    getHeader() {
      return undefined;
    },
    removeHeader() {},
    end() {},
  },
  (err?: unknown) => {
    methodError = err ?? null;
  },
);
assert.equal(methodError, null);
assert.equal(routeAllCalled, true);
assert.equal(useCalled, true);
