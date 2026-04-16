import { EventEmitter } from "node:events";

import { normalizeQueryParserSetting } from "./query.ts";
import { decorateRequest, decorateResponse, defaultFinalHandler, pathnameFromUrl } from "./http.ts";
import type {
  ApplicationLike,
  Handler,
  Next,
  RequestLike,
  ResponseLike,
  RouteBuilder,
  RouteLayer,
  RouterLike,
  StandaloneRoute,
} from "./types.ts";

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
    const entries = route.entries.filter((entry) => entry.method === method);
    let index = 0;
    const run = (err?: unknown): void => {
      if (err) {
        finalHandler(err);
        return;
      }
      const entry = entries[index++];
      if (!entry) {
        finalHandler();
        return;
      }
      for (const callback of entry.callbacks) {
        callback(request, response, run);
        return;
      }
      run();
    };
    run();
  };
  return builder;
}

type Layer = {
  kind: "use" | "route";
  path: string;
  handler?: Handler;
  route?: RouteLayer;
};

export function createRouter(_parentApp?: ApplicationLike): RouterLike {
  const layers: Layer[] = [];

  const router = ((req: RequestLike, res: ResponseLike, next?: Next) => {
    router.handle(req, res, next);
  }) as RouterLike;

  router.__isExpressRouter = true;
  router.__isExpressApp = false;

  router.use = (...args: unknown[]) => {
    let path = "/";
    let callbacks = args;
    if (typeof args[0] === "string") {
      path = args[0];
      callbacks = args.slice(1);
    }
    for (const callback of flattenCallbacks(callbacks)) {
      layers.push({ kind: "use", path, handler: callback });
    }
    return router;
  };

  router.route = (path: string) => {
    const route: RouteLayer = { path, entries: [] };
    layers.push({ kind: "route", path, route });
    return createRouteBuilder(route);
  };

  router.get = (...args: unknown[]) => {
    const [path, ...callbacks] = args;
    return router.route(path as string).get(...callbacks), router;
  };
  router.post = (...args: unknown[]) => {
    const [path, ...callbacks] = args;
    return router.route(path as string).post(...callbacks), router;
  };
  router.delete = (...args: unknown[]) => {
    const [path, ...callbacks] = args;
    return router.route(path as string).delete(...callbacks), router;
  };
  router.all = (...args: unknown[]) => {
    const [path, ...callbacks] = args;
    return router.route(path as string).all(...callbacks), router;
  };
  router.param = (_nameOrNames: string | string[], _callback: Handler) => router;

  router.handle = (request: RequestLike, response: ResponseLike, out?: Next) => {
    decorateRequest(request);
    decorateResponse(response, request);
    const finalHandler = out || defaultFinalHandler(response);
    const method = (request.method || "GET").toUpperCase();
    const pathname = pathnameFromUrl(request.url || "/");
    let index = 0;

    const run = (err?: unknown): void => {
      if (err) {
        finalHandler(err);
        return;
      }
      const layer = layers[index++];
      if (!layer) {
        finalHandler();
        return;
      }
      if (layer.kind === "use") {
        if (layer.path === "/" || pathname.startsWith(layer.path)) {
          const callback = layer.handler!;
          if ((callback as RouterLike).__isExpressRouter || (callback as ApplicationLike).__isExpressApp) {
            // TODO(build-spec): mounted apps/routers should trim req.url and update req.baseUrl.
            (callback as RouterLike)(request, response, run);
            return;
          }
          callback(request, response, run);
          return;
        }
        run();
        return;
      }

      const route = layer.route!;
      if (route.path !== pathname) {
        run();
        return;
      }
      const entry = route.entries.find((candidate) => candidate.method === method);
      if (!entry) {
        run();
        return;
      }
      let callbackIndex = 0;
      const runEntry = (nextErr?: unknown): void => {
        if (nextErr) {
          finalHandler(nextErr);
          return;
        }
        const callback = entry.callbacks[callbackIndex++];
        if (!callback) {
          finalHandler();
          return;
        }
        callback(request, response, runEntry);
      };
      runEntry();
    };

    run();
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
    router.use(...args);
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
  app.param = (_nameOrNames: string | string[], _callback: Handler) => app;
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
