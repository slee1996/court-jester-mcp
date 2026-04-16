import assert from "node:assert/strict";

import { Route } from "../index.ts";

const req = { order: "", method: "GET", url: "/" };
const route = Route("/foo");

route.get((request, response, next) => {
  request.order += "a";
  next();
});
route.all((request, response, next) => {
  request.order += "b";
  next();
});
route.get((request, response, next) => {
  request.order += "c";
  next();
});

route.dispatch(req, {}, assert.ifError);
assert.equal(req.order, "abc");
