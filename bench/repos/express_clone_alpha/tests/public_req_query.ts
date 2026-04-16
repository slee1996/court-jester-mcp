import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

function createApp(setting?: unknown) {
  const app = express();
  if (setting !== undefined) {
    app.set("query parser", setting);
  }
  app.use((req, res) => {
    res.json(req.query);
  });
  return app;
}

const simple = await invoke(createApp(), { url: "/?user[name]=tj" });
assert.equal(simple.body, JSON.stringify({ "user[name]": "tj" }));

const extended = await invoke(createApp("extended"), {
  url: "/?foo[0][bar]=baz&foo[0][fizz]=buzz&foo[]=done!",
});
assert.equal(extended.body, JSON.stringify({ foo: [{ bar: "baz", fizz: "buzz" }, "done!"] }));

const disabled = await invoke(createApp(false), { url: "/?user%5Bname%5D=tj" });
assert.equal(disabled.body, JSON.stringify({}));
