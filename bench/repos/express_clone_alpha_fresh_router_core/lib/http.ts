import { normalizeQueryParserSetting, parseQueryString } from "./query.ts";
import type { Next, RequestLike, ResponseLike } from "./types.ts";
import { STATUS_TEXT } from "./types.ts";

export function normalizeHeaderName(name: string): string {
  return name.toLowerCase();
}

export function pathnameFromUrl(value: string | undefined): string {
  if (!value) {
    return "/";
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

export function requestHeader(request: RequestLike, name: string): string | undefined {
  const target = normalizeHeaderName(name);
  for (const [headerName, headerValue] of Object.entries(request.headers || {})) {
    if (normalizeHeaderName(headerName) === target) {
      return headerValue;
    }
  }
  return undefined;
}

export function requestContentType(request: RequestLike): string | undefined {
  const value = requestHeader(request, "content-type");
  return value?.split(";", 1)[0]?.trim().toLowerCase();
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

export function defaultFinalHandler(res: ResponseLike): Next {
  return (err?: unknown) => {
    if (!err) {
      return;
    }
    const status =
      typeof err === "object" &&
      err !== null &&
      "status" in err &&
      typeof (err as { status?: unknown }).status === "number"
        ? (err as { status: number }).status
        : 500;
    res.statusCode = status;
    res.setHeader?.("Content-Type", "text/plain; charset=utf-8");
    res.end?.(err instanceof Error ? (err.stack || err.message) : String(err));
  };
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

export function decorateRequest(request: RequestLike): void {
  if (request.originalUrl === undefined) {
    request.originalUrl = request.url || "/";
  }
  if (request.baseUrl === undefined) {
    request.baseUrl = "";
  }
  request.path = pathnameFromUrl(request.url || request.originalUrl);
  request.params = request.params || {};
  if (!Object.prototype.hasOwnProperty.call(request, "query")) {
    const parser = normalizeQueryParserSetting(request.app?.__settings.get("query parser"));
    request.query = parseQueryString(searchFromUrl(request.url || request.originalUrl), parser);
  }
  if (typeof request.get !== "function") {
    request.get = (name: string) => requestHeader(request, name);
  }
  if (typeof request.header !== "function") {
    request.header = request.get;
  }
  if (typeof request.protocol !== "string") {
    request.protocol = request.encrypted === true ? "https" : "http";
  }
  if (typeof request.secure !== "boolean") {
    request.secure = request.protocol === "https";
  }
  if (typeof request.xhr !== "boolean") {
    request.xhr = requestHeader(request, "x-requested-with")?.toLowerCase() === "xmlhttprequest";
  }
}

export function decorateResponse(response: ResponseLike, request: RequestLike): void {
  ensureResponseInfrastructure(response);
  if (typeof response.status !== "function") {
    response.status = (code: number) => {
      response.statusCode = code;
      response.statusMessage = STATUS_TEXT[code] || String(code);
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
      response.setHeader?.("Content-Type", value);
      return response;
    };
  }
  if (typeof response.send !== "function") {
    response.send = (body: unknown) => {
      if (body !== null && typeof body === "object" && !ArrayBuffer.isView(body)) {
        return response.json?.(body);
      }
      if (!response.getHeader?.("content-type") && typeof body === "string") {
        response.setHeader?.("Content-Type", "text/plain; charset=utf-8");
      }
      if (request.method?.toUpperCase() === "HEAD") {
        response.end?.("");
        return response;
      }
      response.end?.(body ?? "");
      return response;
    };
  }
  if (typeof response.json !== "function") {
    response.json = (body: unknown) => {
      response.setHeader?.("Content-Type", "application/json; charset=utf-8");
      const payload = JSON.stringify(body);
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
      return response.send?.(String(code));
    };
  }
  if (typeof response.location !== "function") {
    response.location = (value: string) => {
      const target =
        value === "back" ? requestHeader(request, "referer") || requestHeader(request, "referrer") || "/" : value;
      response.setHeader?.("Location", target);
      return response;
    };
  }
  if (typeof response.links !== "function") {
    response.links = (mapping: Record<string, string>) => {
      const serialized = Object.entries(mapping)
        .map(([rel, url]) => `<${url}>; rel="${rel}"`)
        .join(", ");
      response.setHeader?.("Link", serialized);
      return response;
    };
  }
  if (typeof response.vary !== "function") {
    response.vary = (field: string) => {
      response.setHeader?.("Vary", field);
      return response;
    };
  }
}
