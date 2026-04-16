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
    result[decodePart(rawKey)] = decodePart(rawValue);
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
  // TODO(build-spec): implement duplicate-key and nested extended parsing.
  return parseFlat(input);
}
