import { normalizeQueryParserSetting, parseQueryString } from "./query.ts";
import type {
  ApplicationLike,
  Handler,
  Next,
  RequestLike,
  ResponseLike,
} from "./types.ts";
import { STATUS_TEXT } from "./types.ts";

export function normalizeHeaderName(name: string): string {
  return name.toLowerCase();
}

export function pathnameFromUrl(value: string | undefined): string {
  if (!value) {
    return "";
  }
  const [pathname] = value.split("?", 1);
  return pathname || "/";
}

export function searchFromUrl(value: string | undefined): string {
  if (!value || !value.includes("?")) {
    return "";
  }
  return value.slice(value.indexOf("?") + 1);
}

export function flattenCallbacks(values: unknown[]): Handler[] {
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

export function defaultFinalHandler(res: ResponseLike): Next {
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

export function requestHeader(request: RequestLike, name: string): string | undefined {
  // TODO(build-spec): Express header lookup is case-insensitive and aliases referer/referrer.
  return request.headers?.[name];
}

export function requestContentType(request: RequestLike): string | undefined {
  const value = requestHeader(request, "content-type");
  if (!value) {
    return undefined;
  }
  return value.split(";", 1)[0]!.trim().toLowerCase();
}

export function requestBodyText(request: RequestLike): string {
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

export function createHttpError(status: number, message: string): Error & { status: number } {
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

export function decorateRequest(request: RequestLike): void {
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
    const encrypted =
      request.socket?.encrypted === true ||
      request.connection?.encrypted === true ||
      request.encrypted === true;
    // TODO(build-spec): trust proxy should affect protocol when x-forwarded-proto is set.
    request.protocol = encrypted ? "https" : "http";
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

export function decorateResponse(response: ResponseLike, request: RequestLike): void {
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
      let payload = JSON.stringify(
        body,
        typeof replacer === "function" ? replacer : undefined,
        typeof spaces === "number" ? spaces : undefined,
      );
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
      const text = String(code);
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
      // TODO(build-spec): location() should preserve "back" semantics and encode unsafe URL bytes.
      response.setHeader?.("Location", target);
      return response;
    };
  }
  if (typeof response.links !== "function") {
    response.links = (mapping: Record<string, string>) => {
      const serialized = Object.entries(mapping)
        .map(([rel, url]) => `<${url}>; rel="${rel}"`)
        .join(", ");
      // TODO(build-spec): links() should append rather than overwrite existing Link headers.
      response.setHeader?.("Link", serialized);
      return response;
    };
  }
  if (typeof response.vary !== "function") {
    response.vary = (field: string) => {
      // TODO(build-spec): vary() should merge and dedupe fields case-insensitively.
      response.setHeader?.("Vary", field);
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
