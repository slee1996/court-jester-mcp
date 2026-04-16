import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";

import { normalizeQueryParserSetting, parseQueryString } from "./lib/query.ts";

type Next = (arg?: unknown) => void;
type Handler = (...args: unknown[]) => void;

type RequestLike = {
  method?: string;
  url?: string;
  headers?: Record<string, string | undefined>;
  rawBody?: unknown;
  body?: unknown;
  params?: Record<string, string>;
  baseUrl?: string;
  originalUrl?: string;
  app?: ApplicationLike;
  path?: string;
  query?: unknown;
  [key: string]: unknown;
};

type ResponseLike = {
  statusCode?: number;
  statusMessage?: string;
  headersSent?: boolean;
  setHeader?: (name: string, value: string) => void;
  getHeader?: (name: string) => string | undefined;
  removeHeader?: (name: string) => void;
  end?: (body?: unknown) => void;
  [key: string]: unknown;
};

type MatchResult = {
  matched: boolean;
  matchedPath: string;
  remainingPath: string;
  params: Record<string, string>;
};

type Layer = {
  kind: "use" | "route";
  path: string;
  handler?: Handler;
  route?: RouteLayer;
};

type RouteEntry = {
  method: string;
  callbacks: Handler[];
};

type RouteLayer = {
  path: string;
  entries: RouteEntry[];
};

type RouterLike = ((req: RequestLike, res: ResponseLike, next?: Next) => void) & {
  use: (...args: unknown[]) => RouterLike;
  get: (...args: unknown[]) => RouterLike;
  post: (...args: unknown[]) => RouterLike;
  delete: (...args: unknown[]) => RouterLike;
  all: (...args: unknown[]) => RouterLike;
  route: (path: string) => RouteBuilder;
  param: (nameOrNames: string | string[], callback: Handler) => RouterLike;
  handle: (req: RequestLike, res: ResponseLike, next?: Next) => void;
  __isExpressRouter: true;
  __isExpressApp?: false;
  __eventEmitter?: EventEmitter;
  parent?: ApplicationLike;
};

type ApplicationLike = RouterLike & {
  set: (name: string, value: unknown) => ApplicationLike;
  listen: (...args: unknown[]) => { close: () => void };
  __isExpressApp: true;
  __settings: Map<string, unknown>;
  parent?: ApplicationLike;
};

type MiddlewareFactoryOptions = {
  extended?: boolean;
  strict?: boolean;
};

type StaticMiddlewareOptions = {
  index?: string | false;
};

type RouteBuilder = {
  all: (...callbacks: unknown[]) => RouteBuilder;
  get: (...callbacks: unknown[]) => RouteBuilder;
  post: (...callbacks: unknown[]) => RouteBuilder;
  delete: (...callbacks: unknown[]) => RouteBuilder;
};

type StandaloneRoute = RouteBuilder & {
  dispatch: (req: RequestLike, res: ResponseLike, next?: Next) => void;
};

type ParamCallback = (req: RequestLike, res: ResponseLike, next: Next, value: string, name: string) => void;

const STATUS_TEXT: Record<number, string> = {
  200: "OK",
  201: "Created",
  204: "No Content",
  301: "Moved Permanently",
  302: "Found",
  303: "See Other",
  406: "Not Acceptable",
  500: "Internal Server Error",
};

function normalizeHeaderName(name: string): string {
  return name.toLowerCase();
}

function splitPathname(value: string): string[] {
  return value.split("/").filter(Boolean);
}

function pathnameFromUrl(value: string | undefined): string {
  if (!value) {
    return "";
  }
  const [pathname] = value.split("?", 1);
  return pathname || "/";
}

function searchFromUrl(value: string | undefined): string {
  if (!value || !value.includes("?")) {
    return "";
  }
  return value.slice(value.indexOf("?") + 1);
}

function decodeSegment(value: string): string {
  return decodeURIComponent(value);
}

function flattenCallbacks(values: unknown[]): Handler[] {
  const callbacks: Handler[] = [];
  const visit = (input: unknown): void => {
    if (Array.isArray(input)) {
      for (const item of input) {
        visit(item);
      }
      return;
    }
    if (typeof input !== "function") {
      throw new TypeError("callback must be a function");
    }
    callbacks.push(input as Handler);
  };
  for (const value of values) {
    visit(value);
  }
  return callbacks;
}

function matchPath(pattern: string, pathname: string, end: boolean): MatchResult {
  if (!pattern || pattern === "/") {
    return {
      matched: true,
      matchedPath: "",
      remainingPath: pathname || "/",
      params: {},
    };
  }
  const patternParts = splitPathname(pattern);
  const pathParts = splitPathname(pathname);
  if (pathParts.length < patternParts.length) {
    return { matched: false, matchedPath: "", remainingPath: pathname, params: {} };
  }
  const params: Record<string, string> = {};
  for (let index = 0; index < patternParts.length; index += 1) {
    const expected = patternParts[index]!;
    const actual = pathParts[index]!;
    if (expected.startsWith(":")) {
      params[expected.slice(1)] = decodeSegment(actual);
      continue;
    }
    if (expected !== decodeSegment(actual)) {
      return { matched: false, matchedPath: "", remainingPath: pathname, params: {} };
    }
  }
  if (end && pathParts.length !== patternParts.length) {
    return { matched: false, matchedPath: "", remainingPath: pathname, params: {} };
  }
  const matchedParts = pathParts.slice(0, patternParts.length);
  const remainingParts = pathParts.slice(patternParts.length);
  return {
    matched: true,
    matchedPath: matchedParts.length > 0 ? `/${matchedParts.join("/")}` : "",
    remainingPath: remainingParts.length > 0 ? `/${remainingParts.join("/")}` : "/",
    params,
  };
}

function defaultFinalHandler(res: ResponseLike): Next {
  return (err?: unknown) => {
    if (!err) {
      return;
    }
    const message = err instanceof Error ? String(err.stack || err) : String(err);
    const status =
      typeof err === "object" &&
      err !== null &&
      "status" in err &&
      typeof (err as { status?: unknown }).status === "number"
        ? (err as { status: number }).status
        : 500;
    if (typeof res.statusCode !== "number" || res.statusCode < 400) {
      res.statusCode = status;
    }
    if (typeof res.setHeader === "function") {
      res.setHeader("Content-Type", "text/plain; charset=utf-8");
    }
    if (typeof res.end === "function") {
      res.end(message);
    }
  };
}

function requestHeader(request: RequestLike, name: string): string | undefined {
  const target = normalizeHeaderName(name);
  const aliases =
    target === "referer" || target === "referrer"
      ? new Set(["referer", "referrer"])
      : new Set([target]);
  for (const [headerName, headerValue] of Object.entries(request.headers || {})) {
    if (aliases.has(normalizeHeaderName(headerName))) {
      return headerValue;
    }
  }
  return undefined;
}

function requestContentType(request: RequestLike): string | undefined {
  const value = requestHeader(request, "content-type");
  if (!value) {
    return undefined;
  }
  return value.split(";", 1)[0]!.trim().toLowerCase();
}

function requestBodyText(request: RequestLike): string {
  const source = request.rawBody !== undefined ? request.rawBody : request.body;
  if (typeof source === "string") {
    return source;
  }
  if (ArrayBuffer.isView(source)) {
    return Buffer.from(source.buffer, source.byteOffset, source.byteLength).toString("utf8");
  }
  if (source === undefined || source === null) {
    return "";
  }
  return String(source);
}

function createHttpError(status: number, message: string): Error & { status: number } {
  const error = new Error(message) as Error & { status: number };
  error.status = status;
  return error;
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function forceUtf8Charset(contentType: string): string {
  if (/;\s*charset=/i.test(contentType)) {
    return contentType.replace(/;\s*charset=[^;]+/i, "; charset=utf-8");
  }
  return `${contentType}; charset=utf-8`;
}

function appendHeaderValue(existing: string | undefined, value: string): string {
  if (!existing) {
    return value;
  }
  return `${existing}, ${value}`;
}

function shouldUseJsonSend(body: unknown): boolean {
  if (body === null) {
    return false;
  }
  if (typeof body === "number" || typeof body === "boolean") {
    return true;
  }
  return typeof body === "object" && !ArrayBuffer.isView(body);
}

function escapeJsonString(value: string): string {
  return value
    .replaceAll("&", "\\u0026")
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e");
}

function encodeRedirectUrl(value: string): string {
  let encoded = "";
  for (let index = 0; index < value.length; index += 1) {
    const char = value[index]!;
    if (
      char === "%" &&
      index + 2 < value.length &&
      /[0-9A-Fa-f]{2}/.test(value.slice(index + 1, index + 3))
    ) {
      encoded += value.slice(index, index + 3);
      index += 2;
      continue;
    }
    encoded += encodeURIComponent(char);
  }
  return encoded
    .replaceAll("%2F", "/")
    .replaceAll("%3A", ":")
    .replaceAll("%3F", "?")
    .replaceAll("%3D", "=")
    .replaceAll("%26", "&");
}

function canonicalMime(key: string): string {
  const base = key.split(";", 1)[0]!.trim();
  if (base === "text") {
    return "text/plain";
  }
  if (base === "html") {
    return "text/html";
  }
  if (base === "json") {
    return "application/json";
  }
  return base;
}

function mimeTypeForValue(value: string): string {
  const normalized = value.trim().toLowerCase().replace(/^\./, "");
  if (normalized === "html" || normalized === "htm") {
    return "text/html";
  }
  if (normalized === "txt" || normalized === "text") {
    return "text/plain";
  }
  if (normalized === "json") {
    return "application/json";
  }
  if (normalized === "js" || normalized === "mjs" || normalized === "cjs") {
    return "application/javascript";
  }
  if (normalized === "css") {
    return "text/css";
  }
  if (normalized === "svg") {
    return "image/svg+xml";
  }
  if (normalized === "png") {
    return "image/png";
  }
  if (normalized === "jpg" || normalized === "jpeg") {
    return "image/jpeg";
  }
  if (normalized === "gif") {
    return "image/gif";
  }
  if (normalized === "ico") {
    return "image/x-icon";
  }
  if (normalized === "xml") {
    return "application/xml";
  }
  if (normalized === "bin") {
    return "application/octet-stream";
  }
  if (normalized.includes("/")) {
    return normalized;
  }
  return value;
}

function parseAcceptHeader(value: string | undefined): Array<{ type: string; q: number; order: number }> {
  if (!value) {
    return [];
  }
  return value
    .split(",")
    .map((part, order) => {
      const sections = part.trim().split(";").map((item) => item.trim());
      const type = sections[0] || "*/*";
      let q = 1;
      for (const section of sections.slice(1)) {
        if (section.startsWith("q=")) {
          const parsed = Number(section.slice(2));
          if (!Number.isNaN(parsed)) {
            q = parsed;
          }
        }
      }
      return { type, q, order };
    })
    .sort((left, right) => {
      if (right.q !== left.q) {
        return right.q - left.q;
      }
      return left.order - right.order;
    });
}

function mimeMatches(accepted: string, candidate: string): boolean {
  if (accepted === "*/*") {
    return true;
  }
  const [acceptedType, acceptedSubtype] = accepted.split("/", 2);
  const [candidateType, candidateSubtype] = candidate.split("/", 2);
  if (acceptedSubtype === "*") {
    return acceptedType === candidateType;
  }
  return acceptedType === candidateType && acceptedSubtype === candidateSubtype;
}

function chooseFormatOption(
  request: RequestLike,
  options: Array<{ key: string; mime: string; callback: Handler }>,
): { key: string; mime: string; callback: Handler } | undefined {
  const acceptValues = parseAcceptHeader(requestHeader(request, "accept"));
  if (acceptValues.length === 0) {
    return options[0];
  }
  let best: { option: { key: string; mime: string; callback: Handler }; q: number; order: number } | undefined;
  for (const option of options) {
    let candidateQ = -1;
    let candidateOrder = Number.MAX_SAFE_INTEGER;
    for (const accepted of acceptValues) {
      if (!mimeMatches(accepted.type, option.mime)) {
        continue;
      }
      candidateQ = accepted.q;
      candidateOrder = accepted.order;
      break;
    }
    if (candidateQ < 0) {
      continue;
    }
    if (!best || candidateQ > best.q || (candidateQ === best.q && candidateOrder < best.order)) {
      best = { option, q: candidateQ, order: candidateOrder };
    }
  }
  return best?.option;
}

function decorateRequest(request: RequestLike): void {
  if (request.originalUrl === undefined) {
    request.originalUrl = request.url || "/";
  }
  if (request.baseUrl === undefined) {
    request.baseUrl = "";
  }
  const path = pathnameFromUrl(request.url || request.originalUrl || "/");
  request.path = path || "/";
  if (!Object.prototype.hasOwnProperty.call(request, "query")) {
    const parser = normalizeQueryParserSetting(request.app?.__settings.get("query parser"));
    request.query = parseQueryString(searchFromUrl(request.url || request.originalUrl), parser);
  }
  if (!request.params) {
    request.params = {};
  }
  if (typeof request.protocol !== "string") {
    const trustProxy = request.app?.__settings.get("trust proxy") === true;
    const forwarded = requestHeader(request, "x-forwarded-proto");
    const forwardedProtocol = forwarded?.split(",", 1)[0]?.trim();
    const encrypted =
      request.socket?.encrypted === true ||
      request.connection?.encrypted === true ||
      request.encrypted === true;
    request.protocol = trustProxy && forwardedProtocol ? forwardedProtocol : encrypted ? "https" : "http";
  }
  if (typeof request.secure !== "boolean") {
    request.secure = request.protocol === "https";
  }
  if (typeof request.xhr !== "boolean") {
    request.xhr = requestHeader(request, "x-requested-with")?.toLowerCase() === "xmlhttprequest";
  }
  if (typeof request.get !== "function") {
    request.get = (name: string) => requestHeader(request, name);
  }
  if (typeof request.header !== "function") {
    request.header = request.get;
  }
}

function ensureResponseInfrastructure(response: ResponseLike): void {
  if (!response.__headers) {
    response.__headers = new Map<string, string>();
  }
  if (typeof response.setHeader !== "function") {
    response.setHeader = (name: string, value: string) => {
      (response.__headers as Map<string, string>).set(normalizeHeaderName(name), value);
    };
  }
  if (typeof response.getHeader !== "function") {
    response.getHeader = (name: string) =>
      (response.__headers as Map<string, string>).get(normalizeHeaderName(name));
  }
  if (typeof response.removeHeader !== "function") {
    response.removeHeader = (name: string) => {
      (response.__headers as Map<string, string>).delete(normalizeHeaderName(name));
    };
  }
  if (typeof response.end !== "function") {
    response.end = (body?: unknown) => {
      response.headersSent = true;
      response.__body = body ?? "";
    };
  }
  if (typeof response.statusCode !== "number") {
    response.statusCode = 200;
  }
}

function decorateResponse(response: ResponseLike, request: RequestLike): void {
  ensureResponseInfrastructure(response);
  if (typeof response.status !== "function") {
    response.status = (code: number) => {
      response.statusCode = code;
      response.statusMessage = STATUS_TEXT[code] || "Status";
      return response;
    };
  }
  if (typeof response.header !== "function") {
    response.header = (name: string, value: string) => {
      response.setHeader?.(name, value);
      return response;
    };
  }
  if (typeof response.set !== "function") {
    response.set = response.header;
  }
  if (typeof response.type !== "function") {
    response.type = (value: string) => {
      response.setHeader?.("Content-Type", mimeTypeForValue(value));
      return response;
    };
  }
  if (typeof response.send !== "function") {
    response.send = (body: unknown) => {
      if (shouldUseJsonSend(body)) {
        return response.json?.(body);
      }
      let payload: unknown = body;
      if (payload === null || payload === undefined) {
        payload = "";
      }
      const existingContentType = response.getHeader?.("content-type");
      if (typeof payload === "string") {
        if (!existingContentType) {
          response.setHeader?.(
            "Content-Type",
            payload.startsWith("<") ? "text/html; charset=utf-8" : "text/plain; charset=utf-8",
          );
        } else {
          response.setHeader?.("Content-Type", forceUtf8Charset(existingContentType));
        }
      } else if (ArrayBuffer.isView(payload)) {
        if (!existingContentType) {
          response.setHeader?.("Content-Type", "application/octet-stream");
        } else if (
          /^text\/plain\b/i.test(existingContentType) &&
          !/;\s*charset=/i.test(existingContentType)
        ) {
          response.setHeader?.("Content-Type", forceUtf8Charset(existingContentType));
        }
      }
      if (response.statusCode === 204) {
        response.removeHeader?.("Content-Type");
        response.removeHeader?.("Content-Length");
        response.removeHeader?.("Transfer-Encoding");
        payload = "";
      }
      if (response.statusCode === 205) {
        response.removeHeader?.("Transfer-Encoding");
        response.setHeader?.("Content-Length", "0");
        payload = "";
      }
      if (request.method?.toUpperCase() === "HEAD") {
        response.end?.("");
        return response;
      }
      response.end?.(payload);
      return response;
    };
  }
  if (typeof response.json !== "function") {
    response.json = (body: unknown) => {
      const existingContentType = response.getHeader?.("content-type");
      if (!existingContentType) {
        response.setHeader?.("Content-Type", "application/json; charset=utf-8");
      } else {
        response.setHeader?.("Content-Type", forceUtf8Charset(existingContentType));
      }
      const replacer = request.app?.__settings.get("json replacer");
      const spaces = request.app?.__settings.get("json spaces");
      let payload = JSON.stringify(body, typeof replacer === "function" ? replacer : undefined, typeof spaces === "number" ? spaces : undefined);
      if (request.app?.__settings.get("json escape") === true && payload) {
        payload = escapeJsonString(payload);
      }
      if (payload === undefined) {
        payload = "";
      }
      if (request.method?.toUpperCase() === "HEAD") {
        response.end?.("");
        return response;
      }
      response.end?.(payload);
      return response;
    };
  }
  if (typeof response.sendStatus !== "function") {
    response.sendStatus = (code: number) => {
      response.status?.(code);
      const text = STATUS_TEXT[code] || String(code);
      if (code === 204) {
        response.end?.("");
        return response;
      }
      return response.send?.(text);
    };
  }
  if (typeof response.location !== "function") {
    response.location = (value: string) => {
      const target =
        value === "back" ? requestHeader(request, "referrer") || requestHeader(request, "referer") || "/" : value;
      response.setHeader?.("Location", encodeRedirectUrl(target));
      return response;
    };
  }
  if (typeof response.links !== "function") {
    response.links = (mapping: Record<string, string>) => {
      const serialized = Object.entries(mapping)
        .map(([rel, url]) => `<${url}>; rel="${rel}"`)
        .join(", ");
      response.setHeader?.("Link", appendHeaderValue(response.getHeader?.("link"), serialized));
      return response;
    };
  }
  if (typeof response.vary !== "function") {
    response.vary = (field: string) => {
      const existing = response.getHeader?.("vary");
      if (existing === "*") {
        return response;
      }
      const seen = new Map<string, string>();
      for (const part of (existing || "").split(",")) {
        const trimmed = part.trim();
        if (!trimmed) {
          continue;
        }
        seen.set(trimmed.toLowerCase(), trimmed);
      }
      for (const part of field.split(",")) {
        const trimmed = part.trim();
        if (!trimmed) {
          continue;
        }
        if (trimmed === "*") {
          response.setHeader?.("Vary", "*");
          return response;
        }
        if (!seen.has(trimmed.toLowerCase())) {
          seen.set(trimmed.toLowerCase(), trimmed);
        }
      }
      response.setHeader?.("Vary", Array.from(seen.values()).join(", "));
      return response;
    };
  }
  if (typeof response.format !== "function") {
    response.format = (mapping: Record<string, Handler>) => {
      response.setHeader?.("Vary", "Accept");
      const options = Object.entries(mapping)
        .filter(([key]) => key !== "default")
        .map(([key, callback]) => ({ key, mime: canonicalMime(key), callback }));
      const selected = chooseFormatOption(request, options);
      if (selected) {
        if (!response.getHeader?.("content-type")) {
          response.setHeader?.("Content-Type", `${selected.mime}; charset=utf-8`);
        }
        selected.callback.call(response, request, response, defaultFinalHandler(response));
        return response;
      }
      if (typeof mapping.default === "function") {
        mapping.default.call(response, request, response, defaultFinalHandler(response));
        return response;
      }
      const error = new Error("Not Acceptable") as Error & { status: number; types: string[] };
      error.status = 406;
      error.types = options.map((option) => option.mime);
      throw error;
    };
  }
  if (typeof response.redirect !== "function") {
    response.redirect = (first: number | string, second?: string) => {
      const status = typeof first === "number" ? first : 302;
      const rawUrl = typeof first === "number" ? second || "/" : first;
      const encodedUrl = encodeRedirectUrl(rawUrl);
      response.status?.(status);
      response.setHeader?.("Location", encodedUrl);
      const accept = requestHeader(request, "accept");
      if (request.method?.toUpperCase() === "HEAD") {
        response.end?.("");
        return response;
      }
      const title = STATUS_TEXT[status] || "Redirect";
      if (!accept || accept.includes("text/html")) {
        response.setHeader?.("Content-Type", "text/html; charset=utf-8");
        const html = `<!DOCTYPE html><head><title>${title}</title></head><body><p>${title}. Redirecting to ${escapeHtml(encodedUrl)}</p></body>`;
        response.end?.(html);
        return response;
      }
      if (accept.includes("text/plain") || accept.includes("*/*")) {
        response.setHeader?.("Content-Type", "text/plain; charset=utf-8");
        response.end?.(`${title}. Redirecting to ${encodedUrl}`);
        return response;
      }
      response.removeHeader?.("Content-Type");
      response.end?.("");
      return response;
    };
  }
}

function createJsonMiddleware(options?: MiddlewareFactoryOptions): Handler {
  const strict = options?.strict !== false;
  return (req: RequestLike, _res: ResponseLike, next: Next) => {
    const contentType = requestContentType(req);
    if (!contentType || (contentType !== "application/json" && !contentType.endsWith("+json"))) {
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
  return (req: RequestLike, _res: ResponseLike, next: Next) => {
    if (requestContentType(req) !== "application/x-www-form-urlencoded") {
      next();
      return;
    }
    req.body = parseQueryString(requestBodyText(req), parserSetting);
    next();
  };
}

function createTextMiddleware(): Handler {
  return (req: RequestLike, _res: ResponseLike, next: Next) => {
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
  return (req: RequestLike, _res: ResponseLike, next: Next) => {
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
  return (req: RequestLike, res: ResponseLike, next: Next) => {
    const method = (req.method || "GET").toUpperCase();
    if (method !== "GET" && method !== "HEAD") {
      next();
      return;
    }

    let pathname = pathnameFromUrl(req.url || req.originalUrl || "/");
    try {
      pathname = decodeURIComponent(pathname || "/");
    } catch {
      next(createHttpError(400, "failed to decode path"));
      return;
    }

    let relativePath = pathname;
    if ((relativePath === "/" || relativePath === "") && indexFile) {
      relativePath = `/${indexFile}`;
    }
    const normalized = path.posix.normalize(relativePath);
    const candidate = path.resolve(resolvedRoot, `.${normalized}`);
    if (candidate !== resolvedRoot && !candidate.startsWith(`${resolvedRoot}${path.sep}`)) {
      next();
      return;
    }

    let stats: fs.Stats;
    try {
      stats = fs.statSync(candidate);
    } catch {
      next();
      return;
    }

    let filePath = candidate;
    if (stats.isDirectory()) {
      if (!indexFile) {
        next();
        return;
      }
      filePath = path.join(candidate, indexFile);
      try {
        stats = fs.statSync(filePath);
      } catch {
        next();
        return;
      }
    }

    if (!stats.isFile()) {
      next();
      return;
    }

    res.type?.(path.extname(filePath));
    const body = fs.readFileSync(filePath);
    res.send?.(body);
  };
}

function registerMount(parent: ApplicationLike | undefined, child: unknown): void {
  if (!parent || !child || typeof child !== "function") {
    return;
  }
  const maybeApp = child as ApplicationLike;
  if (!maybeApp.__isExpressApp) {
    return;
  }
  maybeApp.parent = parent;
  maybeApp.__eventEmitter?.emit("mount", parent);
}

function createRouteBuilder(route: RouteLayer): RouteBuilder {
  const add = (method: string, callbacks: unknown[]) => {
    route.entries.push({ method, callbacks: flattenCallbacks(callbacks) });
    return builder;
  };
  const builder: RouteBuilder = {
    all: (...callbacks) => add("ALL", callbacks),
    get: (...callbacks) => add("GET", callbacks),
    post: (...callbacks) => add("POST", callbacks),
    delete: (...callbacks) => add("DELETE", callbacks),
  };
  return builder;
}

function createStandaloneRoute(path: string): StandaloneRoute {
  const route: RouteLayer = { path, entries: [] };
  const builder = createRouteBuilder(route) as StandaloneRoute;
  builder.dispatch = (request: RequestLike, response: ResponseLike, out?: Next) => {
    decorateRequest(request);
    decorateResponse(response, request);
    const finalHandler = out || defaultFinalHandler(response);
    const method = (request.method || "").toUpperCase();
    const entries = route.entries.filter((entry) => entry.method === "ALL" || entry.method === method);
    let entryIndex = 0;
    let callbackIndex = 0;
    let currentError: unknown = undefined;
    const run = (): void => {
      while (entryIndex < entries.length) {
        const entry = entries[entryIndex]!;
        while (callbackIndex < entry.callbacks.length) {
          const callback = entry.callbacks[callbackIndex++]!;
          if (currentError !== undefined && callback.length !== 4) {
            continue;
          }
          if (currentError === undefined && callback.length === 4) {
            continue;
          }
          try {
            if (currentError !== undefined) {
              callback(currentError, request, response, (arg?: unknown) => {
                currentError = arg;
                run();
              });
            } else {
              callback(request, response, (arg?: unknown) => {
                currentError = arg;
                run();
              });
            }
          } catch (error) {
            currentError = error;
            run();
          }
          return;
        }
        entryIndex += 1;
        callbackIndex = 0;
      }
      finalHandler(currentError);
    };
    run();
  };
  return builder;
}

function invokeWithErrorMode(
  callback: Handler,
  err: unknown,
  req: RequestLike,
  res: ResponseLike,
  next: Next,
): void {
  if (err !== undefined && callback.length === 4) {
    callback(err, req, res, next);
    return;
  }
  if (err === undefined && callback.length < 4) {
    callback(req, res, next);
    return;
  }
  next(err);
}

function createRouter(parentApp?: ApplicationLike): RouterLike {
  const layers: Layer[] = [];
  const paramCallbacks = new Map<string, ParamCallback[]>();

  const router = ((req: RequestLike, res: ResponseLike, next?: Next) => {
    router.handle(req, res, next);
  }) as RouterLike;

  router.__isExpressRouter = true;
  router.__isExpressApp = false;

  const routeMethod = (method: string, args: unknown[]): RouterLike => {
    const [path, ...callbacks] = args;
    if (typeof path !== "string") {
      throw new TypeError("route path must be a string");
    }
    const route = router.route(path);
    if (method === "GET") {
      route.get(...callbacks);
    } else if (method === "POST") {
      route.post(...callbacks);
    } else if (method === "DELETE") {
      route.delete(...callbacks);
    } else {
      route.all(...callbacks);
    }
    return router;
  };

  router.use = (...args: unknown[]) => {
    let path = "/";
    let callbacks = args;
    if (typeof args[0] === "string") {
      path = args[0];
      callbacks = args.slice(1);
    }
    for (const callback of flattenCallbacks(callbacks)) {
      layers.push({ kind: "use", path, handler: callback });
      registerMount(parentApp, callback);
    }
    return router;
  };

  router.route = (path: string) => {
    const route: RouteLayer = { path, entries: [] };
    layers.push({ kind: "route", path, route });
    return createRouteBuilder(route);
  };

  router.get = (...args: unknown[]) => routeMethod("GET", args);
  router.post = (...args: unknown[]) => routeMethod("POST", args);
  router.delete = (...args: unknown[]) => routeMethod("DELETE", args);
  router.all = (...args: unknown[]) => routeMethod("ALL", args);

  router.param = (nameOrNames: string | string[], callback: Handler) => {
    if (typeof callback !== "function") {
      throw new TypeError("param callback must be a function");
    }
    const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
    for (const name of names) {
      const list = paramCallbacks.get(name) || [];
      list.push(callback as ParamCallback);
      paramCallbacks.set(name, list);
    }
    return router;
  };

  router.handle = (request: RequestLike, response: ResponseLike, out?: Next) => {
    if (typeof request.url !== "string" || request.url === "") {
      (out || defaultFinalHandler(response))();
      return;
    }
    decorateRequest(request);
    decorateResponse(response, request);
    const finalHandler = out || defaultFinalHandler(response);
    let index = 0;
    let pending = true;
    let active = false;
    let currentError: unknown = undefined;

    const pump = (): void => {
      if (active) {
        return;
      }
      active = true;
      while (pending) {
        pending = false;
        let advanced = false;
        while (index < layers.length) {
          const layer = layers[index++]!;
          const pathname = pathnameFromUrl(request.url || "/");
          const match = matchPath(layer.path, pathname, layer.kind === "route");
          if (!match.matched) {
            continue;
          }
          advanced = true;
          try {
            if (layer.kind === "use") {
              const callback = layer.handler!;
              const previousUrl = request.url;
              const previousBaseUrl = request.baseUrl || "";
              const previousParams = request.params || {};
              if (match.matchedPath) {
                request.baseUrl = `${previousBaseUrl}${match.matchedPath}`;
                request.url = `${match.remainingPath}${searchFromUrl(previousUrl) ? `?${searchFromUrl(previousUrl)}` : ""}`;
              }
              request.params = { ...previousParams, ...match.params };
              const next: Next = (arg?: unknown) => {
                request.url = previousUrl;
                request.baseUrl = previousBaseUrl;
                request.params = previousParams;
                currentError = arg;
                pending = true;
                if (!active) {
                  pump();
                }
              };
              if ((callback as RouterLike).__isExpressRouter || (callback as ApplicationLike).__isExpressApp) {
                (callback as RouterLike)(request, response, next);
              } else {
                invokeWithErrorMode(callback, currentError, request, response, next);
              }
              break;
            }

            const route = layer.route!;
            const method = (request.method || "").toUpperCase();
            const relevantEntries = route.entries.filter(
              (entry) => entry.method === "ALL" || entry.method === method,
            );
            if (relevantEntries.length === 0 && currentError === undefined) {
              continue;
            }
            const previousParams = request.params || {};
            request.params = { ...previousParams, ...match.params };

            const paramState = new Map<string, string>(
              ((request.__paramCache as Map<string, string> | undefined) || new Map()).entries(),
            );
            request.__paramCache = paramState;

            const paramNames = Object.keys(match.params);
            let paramIndex = 0;
            let entryIndex = 0;
            let callbackIndex = 0;

            const finishRoute: Next = (arg?: unknown) => {
              request.params = previousParams;
              currentError = arg;
              pending = true;
              if (!active) {
                pump();
              }
            };

            const runRoute = (): void => {
              while (paramIndex < paramNames.length) {
                const name = paramNames[paramIndex]!;
                const value = match.params[name]!;
                const key = `${name}:${value}`;
                if (paramState.get(key) === value) {
                  paramIndex += 1;
                  continue;
                }
                const callbacks = paramCallbacks.get(name) || [];
                let paramCallbackIndex = 0;
                const runParamCallbacks = (): void => {
                  while (paramCallbackIndex < callbacks.length) {
                    const callback = callbacks[paramCallbackIndex++]!;
                    try {
                      callback(request, response, (arg?: unknown) => {
                        if (arg === "route") {
                          finishRoute();
                          return;
                        }
                        if (arg !== undefined) {
                          finishRoute(arg);
                          return;
                        }
                        runParamCallbacks();
                      }, value, name);
                    } catch (error) {
                      finishRoute(error);
                    }
                    return;
                  }
                  paramState.set(key, value);
                  paramIndex += 1;
                  runRoute();
                };
                runParamCallbacks();
                return;
              }

              while (entryIndex < relevantEntries.length) {
                const entry = relevantEntries[entryIndex]!;
                while (callbackIndex < entry.callbacks.length) {
                  const callback = entry.callbacks[callbackIndex++]!;
                  if (currentError !== undefined && callback.length !== 4) {
                    continue;
                  }
                  if (currentError === undefined && callback.length === 4) {
                    continue;
                  }
                  try {
                    if (currentError !== undefined) {
                      callback(currentError, request, response, (arg?: unknown) => {
                        if (arg === "route") {
                          finishRoute();
                          return;
                        }
                        currentError = arg;
                        runRoute();
                      });
                    } else {
                      if ((callback as RouterLike).__isExpressRouter || (callback as ApplicationLike).__isExpressApp) {
                        const nestedPreviousParams = request.params;
                        request.params = previousParams;
                        (callback as RouterLike)(request, response, (arg?: unknown) => {
                          request.params = nestedPreviousParams;
                          if (arg === "route") {
                            finishRoute();
                            return;
                          }
                          currentError = arg;
                          runRoute();
                        });
                      } else {
                        callback(request, response, (arg?: unknown) => {
                          if (arg === "route") {
                            finishRoute();
                            return;
                          }
                          currentError = arg;
                          runRoute();
                        });
                      }
                    }
                  } catch (error) {
                    currentError = error;
                    runRoute();
                  }
                  return;
                }
                entryIndex += 1;
                callbackIndex = 0;
              }
              finishRoute(currentError);
            };

            runRoute();
            break;
          } catch (error) {
            currentError = error;
            pending = true;
            break;
          }
        }
        if (!advanced) {
          active = false;
          finalHandler(currentError);
          return;
        }
      }
      active = false;
    };

    pump();
  };

  return router;
}

function createApplication(): ApplicationLike {
  const emitter = new EventEmitter();
  const settings = new Map<string, unknown>();
  settings.set("query parser", "simple");
  const router = createRouter(undefined);

  const app = ((request: RequestLike, response: ResponseLike, next?: Next) => {
    app.handle(request, response, next);
  }) as ApplicationLike;

  Object.assign(app, router);
  app.__isExpressRouter = true;
  app.__isExpressApp = true;
  app.__eventEmitter = emitter;
  app.__settings = settings;
  app.on = emitter.on.bind(emitter);
  app.once = emitter.once.bind(emitter);
  app.emit = emitter.emit.bind(emitter);
  app.route = router.route;
  app.enable = (name: string) => {
    settings.set(name, true);
    return app;
  };
  app.use = (...args: unknown[]) => {
    let path = "/";
    let callbacks = args;
    if (typeof args[0] === "string") {
      path = args[0];
      callbacks = args.slice(1);
    }
    for (const callback of flattenCallbacks(callbacks)) {
      registerMount(app, callback);
      (router.use as (...useArgs: unknown[]) => RouterLike)(path, callback);
    }
    return app;
  };
  app.get = (...args: unknown[]) => {
    if (args.length === 1 && typeof args[0] === "string") {
      return settings.get(args[0]);
    }
    router.get(...args);
    return app;
  };
  app.post = (...args: unknown[]) => {
    router.post(...args);
    return app;
  };
  app.delete = (...args: unknown[]) => {
    router.delete(...args);
    return app;
  };
  app.all = (...args: unknown[]) => {
    router.all(...args);
    return app;
  };
  app.param = (nameOrNames: string | string[], callback: Handler) => {
    router.param(nameOrNames, callback);
    return app;
  };
  app.handle = (request: RequestLike, response: ResponseLike, next?: Next) => {
    request.app = app;
    delete request.query;
    router.handle(request, response, next);
  };
  app.set = (name: string, value: unknown) => {
    if (name === "query parser") {
      settings.set(name, normalizeQueryParserSetting(value));
      return app;
    }
    settings.set(name, value);
    return app;
  };
  app.listen = () => ({ close: () => undefined });
  return app;
}

function express(): ApplicationLike {
  return createApplication();
}

function Router(): RouterLike {
  return createRouter(undefined);
}

function Route(path: string): StandaloneRoute {
  return createStandaloneRoute(path);
}

(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).Router = Router;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).Route = Route;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).json = createJsonMiddleware;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).urlencoded = createUrlencodedMiddleware;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).text = createTextMiddleware;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).raw = createRawMiddleware;
(express as unknown as {
  Router: typeof Router;
  Route: typeof Route;
  json: (options?: MiddlewareFactoryOptions) => Handler;
  urlencoded: (options?: MiddlewareFactoryOptions) => Handler;
  text: () => Handler;
  raw: () => Handler;
  static: (root: string, options?: StaticMiddlewareOptions) => Handler;
}).static = createStaticMiddleware;

export { Route, Router };
export default express;
