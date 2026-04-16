import assert from "node:assert/strict";

import type { default as ExpressFactory } from "../index.ts";

type App = ReturnType<typeof ExpressFactory>;

type InvokeOptions = {
  method?: string;
  url?: string;
  headers?: Record<string, string>;
  body?: string;
};

export type CapturedResponse = {
  statusCode: number;
  headers: Record<string, string>;
  body: string;
};

export function createResponseCapture() {
  const headers = new Map<string, string>();
  const response = {
    statusCode: 200,
    headersSent: false,
    setHeader(name: string, value: string) {
      headers.set(name.toLowerCase(), value);
    },
    getHeader(name: string) {
      return headers.get(name.toLowerCase());
    },
    removeHeader(name: string) {
      headers.delete(name.toLowerCase());
    },
    end(body?: unknown) {
      response.headersSent = true;
      response.__body = body ?? "";
    },
    __body: "",
  };
  return {
    response,
    capture(): CapturedResponse {
      return {
        statusCode: response.statusCode,
        headers: Object.fromEntries(headers.entries()),
        body: String(response.__body ?? ""),
      };
    },
  };
}

export async function invoke(app: App, options: InvokeOptions = {}): Promise<CapturedResponse> {
  const request = {
    method: options.method || "GET",
    url: options.url || "/",
    headers: options.headers || {},
    rawBody: options.body,
  };
  const { response, capture } = createResponseCapture();
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = () => {
      if (settled) {
        return;
      }
      settled = true;
      resolve();
    };
    app.handle(request, response, (err?: unknown) => {
      if (err) {
        const status =
          typeof err === "object" &&
          err !== null &&
          "status" in err &&
          typeof (err as { status?: unknown }).status === "number"
            ? (err as { status: number }).status
            : 500;
        if (typeof response.statusCode !== "number" || response.statusCode < 400) {
          response.statusCode = status;
        }
        response.setHeader("Content-Type", "text/plain; charset=utf-8");
        const message = err instanceof Error ? String(err.stack || err) : String(err);
        response.end(message);
        finish();
        return;
      }
      finish();
    });
    if (response.headersSent) {
      finish();
    }
  });
  return capture();
}

export function expectHeader(response: CapturedResponse, name: string, expected: string): void {
  assert.equal(response.headers[name.toLowerCase()], expected);
}
