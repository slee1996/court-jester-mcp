import assert from "node:assert/strict";

import express from "../index.ts";
import { invoke } from "./harness.ts";

const blog = express();
const forum = express();
const app = express();

let mountedParent: unknown = null;
blog.once("mount", (parent) => {
  mountedParent = parent;
});

blog.get("/", (req, res) => {
  res.send("blog");
});
forum.get("/", (req, res) => {
  res.send("forum");
});

app.use("/blog", blog);
app.use("/forum", forum);

assert.equal(blog.parent, app);
assert.equal(mountedParent, app);

const blogResponse = await invoke(app, { url: "/blog" });
assert.equal(blogResponse.body, "blog");

const forumResponse = await invoke(app, { url: "/forum" });
assert.equal(forumResponse.body, "forum");
