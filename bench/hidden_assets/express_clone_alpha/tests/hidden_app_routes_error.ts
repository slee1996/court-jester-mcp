import assert from "node:assert/strict";

import express from "../index.ts";

const app = express();
let message = "";

app.get("/", () => {
  throw new Error("boom!");
});

app.get("/", (err, req, res, next) => {
  throw new Error("oops");
});

app.get("/", (err, req, res, next) => {
  message = err.message;
  next();
});

const response = {
  statusCode: 200,
  setHeader() {},
  getHeader() {
    return undefined;
  },
  removeHeader() {},
  end() {},
};

app.handle({ url: "/", method: "GET", headers: {} }, response, assert.ifError);
assert.equal(message, "oops");
