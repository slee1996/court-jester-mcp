import assert from "node:assert/strict";

import express, { Route } from "../index.ts";
import { invoke } from "./harness.ts";

{
  const child = express();
  child.get("/", (_req, res) => {
    res.send("child");
  });
  const app = express();
  app.use("/blog", child);
  app.use((_req, res) => {
    res.status(404).send("fallback");
  });
  const response = await invoke(app, { url: "/blog" });
  assert.equal(response.body, "child");
}

{
  const route = Route("/foo");
  let called = false;
  route.all((_req, _res, next) => {
    called = true;
    next();
  });
  route.dispatch(
    { url: "/foo", headers: {} },
    {
      setHeader() {},
      getHeader() {
        return undefined;
      },
      removeHeader() {},
      end() {},
    },
    assert.ifError,
  );
  assert.equal(called, true);
}

{
  const app = express();
  let seen = "";
  app.param("user", (_req, _res, next, value) => {
    seen += value;
    next();
  });
  app.get("/user/:user", (_req, res) => {
    res.send("ok");
  });
  await invoke(app, { url: "/user/tj" });
  await invoke(app, { url: "/user/tj" });
  assert.equal(seen, "tjtj");
}
