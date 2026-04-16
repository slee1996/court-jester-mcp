type QueryParserSetting = "simple" | "extended" | true | false | ((input: string) => unknown);

function decodePart(value: string): string {
  return decodeURIComponent(value.replace(/\+/g, " "));
}

function assignNested(target: Record<string, unknown>, key: string, value: string): void {
  const parts = key
    .replace(/\]/g, "")
    .split("[")
    .filter(Boolean);
  if (parts.length === 0) {
    return;
  }
  let cursor: Record<string, unknown> | unknown[] = target;
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index]!;
    const isLast = index === parts.length - 1;
    if (Array.isArray(cursor)) {
      if (part === "") {
        if (isLast) {
          cursor.push(value);
          return;
        }
        const child: Record<string, unknown> = {};
        cursor.push(child);
        cursor = child;
        continue;
      }
      const numeric = Number(part);
      const slot =
        Number.isInteger(numeric) && numeric >= 0 ? numeric : cursor.length;
      if (isLast) {
        cursor[slot] = value;
        return;
      }
      const existing = cursor[slot];
      if (existing && typeof existing === "object") {
        cursor = existing as Record<string, unknown> | unknown[];
      } else {
        const child: Record<string, unknown> = {};
        cursor[slot] = child;
        cursor = child;
      }
      continue;
    }
    const nextPart = parts[index + 1];
    if (isLast) {
      const existing = cursor[part];
      if (existing === undefined) {
        cursor[part] = value;
        return;
      }
      if (Array.isArray(existing)) {
        existing.push(value);
        return;
      }
      cursor[part] = [existing, value];
      return;
    }
    const wantsArray = nextPart === "" || /^\d+$/.test(nextPart || "");
    const existing = cursor[part];
    if (existing && typeof existing === "object") {
      cursor = existing as Record<string, unknown> | unknown[];
      continue;
    }
    const child: Record<string, unknown> | unknown[] = wantsArray ? [] : {};
    cursor[part] = child;
    cursor = child;
  }
}

function parseSimple(input: string): Record<string, unknown> {
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

function parseExtended(input: string): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  if (!input.trim()) {
    return result;
  }
  for (const segment of input.split("&")) {
    if (!segment) {
      continue;
    }
    const [rawKey, rawValue = ""] = segment.split("=", 2);
    assignNested(result, decodePart(rawKey), decodePart(rawValue));
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
  if (setting === "extended") {
    return parseExtended(input);
  }
  return parseSimple(input);
}
