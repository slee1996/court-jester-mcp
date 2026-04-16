function decodePart(value: string): string {
  return decodeURIComponent(value.replace(/\+/g, " "));
}

function pushValue(target: Record<string, unknown>, key: string, value: string): void {
  const existing = target[key];
  if (existing === undefined) {
    target[key] = value;
    return;
  }
  if (Array.isArray(existing)) {
    existing.push(value);
    return;
  }
  target[key] = [existing, value];
}

export function parseQuery(input: string): Record<string, unknown> {
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
    if (key.endsWith("[]")) {
      const baseKey = key.slice(0, -2);
      const existing = result[baseKey];
      if (existing === undefined) {
        result[baseKey] = [value];
      } else if (Array.isArray(existing)) {
        existing.push(value);
      } else {
        result[baseKey] = [existing, value];
      }
      continue;
    }
    pushValue(result, key, value);
  }
  return result;
}
