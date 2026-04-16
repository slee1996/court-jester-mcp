import assert from "node:assert/strict";

import { Route } from "../index.ts";

const req = { method: "GET", url: "/" };
const route = Route("/foo");

route.get(() => {
  throw new Error("boom!");
});
route.get((err, request, response, next) => {
  throw new Error("oops");
});
route.get((err, request, response, next) => {
  request.message = err.message;
  next();
});

route.dispatch(req, {}, assert.ifError);
assert.equal(req.message, "oops");
