import fs from "node:fs";
import path from "node:path";

import { parseQueryString } from "./lib/query.ts";
import { createApplication, Route, Router } from "./lib/router.ts";
import { createHttpError, pathnameFromUrl, requestBodyText, requestContentType } from "./lib/http.ts";
import type { ApplicationLike, Handler, MiddlewareFactoryOptions, StaticMiddlewareOptions } from "./lib/types.ts";

function createJsonMiddleware(options?: MiddlewareFactoryOptions): Handler {
  const strict = options?.strict !== false;
  return (req, _res, next) => {
    const contentType = requestContentType(req);
    if (!contentType || !contentType.includes("json")) {
      next();
      return;
    }
    const body = requestBodyText(req);
    if (!body.trim()) {
      req.body = {};
      next();
      return;
    }
    try {
      const parsed = JSON.parse(body);
      if (strict && (parsed === null || typeof parsed !== "object")) {
        next(createHttpError(400, "invalid json body"));
        return;
      }
      req.body = parsed;
      next();
    } catch (error) {
      next(createHttpError(400, error instanceof Error ? error.message : String(error)));
    }
  };
}

function createUrlencodedMiddleware(options?: MiddlewareFactoryOptions): Handler {
  const parserSetting = options?.extended ? "extended" : "simple";
  return (req, _res, next) => {
    if (requestContentType(req) !== "application/x-www-form-urlencoded") {
      next();
      return;
    }
    req.body = parseQueryString(requestBodyText(req), parserSetting);
    next();
  };
}

function createTextMiddleware(): Handler {
  return (req, _res, next) => {
    const contentType = requestContentType(req);
    if (!contentType || !contentType.startsWith("text/")) {
      next();
      return;
    }
    req.body = requestBodyText(req);
    next();
  };
}

function createRawMiddleware(): Handler {
  return (req, _res, next) => {
    const contentType = requestContentType(req);
    if (!contentType || contentType !== "application/octet-stream") {
      next();
      return;
    }
    req.body = Buffer.from(requestBodyText(req), "utf8");
    next();
  };
}

function createStaticMiddleware(root: string, options?: StaticMiddlewareOptions): Handler {
  const resolvedRoot = path.resolve(root);
  const indexFile = options?.index === false ? false : options?.index || "index.html";
  return (req, res, next) => {
    const method = (req.method || "GET").toUpperCase();
    if (method !== "GET" && method !== "HEAD") {
      next();
      return;
    }
    let targetPath = pathnameFromUrl(req.url || req.originalUrl || "/");
    if ((targetPath === "/" || targetPath === "") && indexFile) {
      targetPath = `/${indexFile}`;
    }
    const filePath = path.resolve(resolvedRoot, `.${path.posix.normalize(targetPath)}`);
    try {
      const body = fs.readFileSync(filePath);
      res.type?.(path.extname(filePath));
      res.send?.(body);
    } catch {
      next();
    }
  };
}

function express(): ApplicationLike {
  return createApplication();
}

Object.assign(express, {
  Router,
  Route,
  json: createJsonMiddleware,
  urlencoded: createUrlencodedMiddleware,
  text: createTextMiddleware,
  raw: createRawMiddleware,
  static: createStaticMiddleware,
});

export { Route, Router };
export default express;
