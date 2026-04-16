import assert from "node:assert/strict";

import express, { Route } from "../index.ts";

const route = Route("/foo");
const req = { method: "GET", url: "/" };

route.all((request, response, next) => {
  request.called = true;
  next();
});

route.dispatch(req, {}, assert.ifError);
assert.equal(req.called, true);

const routeWithStack = Route("/foo");
const stackedReq = { count: 0, method: "GET", url: "/" };
routeWithStack.all((request, response, next) => {
  request.count += 1;
  next();
});
routeWithStack.all((request, response, next) => {
  request.count += 1;
  next();
});
routeWithStack.dispatch(stackedReq, {}, assert.ifError);
assert.equal(stackedReq.count, 2);
