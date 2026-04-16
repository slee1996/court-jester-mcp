import { EventEmitter } from "node:events";

import { normalizeQueryParserSetting } from "./query.ts";
import {
  decorateRequest,
  decorateResponse,
  defaultFinalHandler,
  flattenCallbacks,
  pathnameFromUrl,
  searchFromUrl,
} from "./http.ts";
import type {
  ApplicationLike,
  Handler,
  Layer,
  MatchResult,
  Next,
  ParamCallback,
  RequestLike,
  ResponseLike,
  RouteBuilder,
  RouteLayer,
  RouterLike,
  StandaloneRoute,
} from "./types.ts";

function splitPathname(value: string): string[] {
  return value.split("/").filter(Boolean);
}

function decodeSegment(value: string): string {
  return decodeURIComponent(value);
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

export function createStandaloneRoute(path: string): StandaloneRoute {
  const route: RouteLayer = { path, entries: [] };
  const builder = createRouteBuilder(route) as StandaloneRoute;
  builder.dispatch = (request: RequestLike, response: ResponseLike, out?: Next) => {
    decorateRequest(request);
    decorateResponse(response, request);
    const finalHandler = out || defaultFinalHandler(response);
    const method = (request.method || "").toUpperCase();
    // TODO(build-spec): Route#all should run for every method, not only exact matches.
    const entries = route.entries.filter((entry) => entry.method === method);
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

export function createRouter(parentApp?: ApplicationLike): RouterLike {
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
              const next: Next = (arg?: unknown) => {
                currentError = arg;
                pending = true;
                if (!active) {
                  pump();
                }
              };
              // TODO(build-spec): mounted routers/apps should receive trimmed req.url and updated baseUrl/params.
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

export function createApplication(): ApplicationLike {
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

export function Router(): RouterLike {
  return createRouter(undefined);
}

export function Route(path: string): StandaloneRoute {
  return createStandaloneRoute(path);
}
