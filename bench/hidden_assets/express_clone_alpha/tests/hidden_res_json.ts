import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const replacerApp = express();
replacerApp.set("json replacer", (key, value) => (key.startsWith("_") ? undefined : value));
replacerApp.use((req, res) => {
  res.json({ name: "tobi", _id: 12345 });
});

const replacerResponse = await invoke(replacerApp);
assert.equal(replacerResponse.body, '{"name":"tobi"}');

const spacesApp = express();
spacesApp.set("json spaces", 2);
spacesApp.use((req, res) => {
  res.json({ name: "tobi", age: 2 });
});

const spacesResponse = await invoke(spacesApp);
assert.equal(spacesResponse.body, '{\n  "name": "tobi",\n  "age": 2\n}');

const undefinedApp = express();
undefinedApp.enable("json escape");
undefinedApp.use((req, res) => {
  res.json(undefined);
});

const undefinedResponse = await invoke(undefinedApp);
assert.equal(undefinedResponse.body, "");
