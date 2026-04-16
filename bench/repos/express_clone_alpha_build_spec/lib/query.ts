type QueryParserSetting = "simple" | "extended" | true | false | ((input: string) => unknown);

function decodePart(value: string): string {
  return decodeURIComponent(value.replace(/\+/g, " "));
}

function parseFlat(input: string): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  if (!input.trim()) {
    return result;
  }
  for (const segment of input.split("&")) {
    if (!segment) {
      continue;
    }
    const [rawKey, rawValue = ""] = segment.split("=", 2);
    const key = decodePart(rawKey);
    const value = decodePart(rawValue);
    const existing = result[key];
    if (existing === undefined) {
      result[key] = value;
    } else if (Array.isArray(existing)) {
      existing.push(value);
    } else {
      result[key] = [existing, value];
    }
  }
  return result;
}

export function normalizeQueryParserSetting(input: unknown): QueryParserSetting {
  if (input === undefined || input === true) {
    return "simple";
  }
  if (input === false) {
    return false;
  }
  if (typeof input === "function") {
    return input;
  }
  if (input === "simple" || input === "extended") {
    return input;
  }
  throw new Error(`unknown value for query parser: ${String(input)}`);
}

export function parseQueryString(input: string, setting: QueryParserSetting): unknown {
  if (setting === false) {
    return {};
  }
  if (typeof setting === "function") {
    return setting(input);
  }
  // TODO(build-spec): implement Express-compatible nested parsing for "extended" mode.
  return parseFlat(input);
}
