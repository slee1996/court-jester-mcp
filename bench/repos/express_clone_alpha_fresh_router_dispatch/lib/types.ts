import type { EventEmitter } from "node:events";

export type Next = (arg?: unknown) => void;
export type Handler = (...args: unknown[]) => void;

export type RequestLike = {
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

export type ResponseLike = {
  statusCode?: number;
  statusMessage?: string;
  headersSent?: boolean;
  setHeader?: (name: string, value: string) => void;
  getHeader?: (name: string) => string | undefined;
  removeHeader?: (name: string) => void;
  end?: (body?: unknown) => void;
  [key: string]: unknown;
};

export type MatchResult = {
  matched: boolean;
  matchedPath: string;
  remainingPath: string;
  params: Record<string, string>;
};

export type Layer = {
  kind: "use" | "route";
  path: string;
  handler?: Handler;
  route?: RouteLayer;
};

export type RouteEntry = {
  method: string;
  callbacks: Handler[];
};

export type RouteLayer = {
  path: string;
  entries: RouteEntry[];
};

export type RouterLike = ((req: RequestLike, res: ResponseLike, next?: Next) => void) & {
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

export type ApplicationLike = RouterLike & {
  set: (name: string, value: unknown) => ApplicationLike;
  listen: (...args: unknown[]) => { close: () => void };
  __isExpressApp: true;
  __settings: Map<string, unknown>;
  parent?: ApplicationLike;
};

export type MiddlewareFactoryOptions = {
  extended?: boolean;
  strict?: boolean;
};

export type StaticMiddlewareOptions = {
  index?: string | false;
};

export type RouteBuilder = {
  all: (...callbacks: unknown[]) => RouteBuilder;
  get: (...callbacks: unknown[]) => RouteBuilder;
  post: (...callbacks: unknown[]) => RouteBuilder;
  delete: (...callbacks: unknown[]) => RouteBuilder;
};

export type StandaloneRoute = RouteBuilder & {
  dispatch: (req: RequestLike, res: ResponseLike, next?: Next) => void;
};

export type ParamCallback = (req: RequestLike, res: ResponseLike, next: Next, value: string, name: string) => void;

export const STATUS_TEXT: Record<number, string> = {
  200: "OK",
  201: "Created",
  204: "No Content",
  301: "Moved Permanently",
  302: "Found",
  303: "See Other",
  406: "Not Acceptable",
  500: "Internal Server Error",
};
