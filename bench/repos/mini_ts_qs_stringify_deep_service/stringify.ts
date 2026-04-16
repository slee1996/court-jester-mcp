function encodePart(value: string): string {
  return encodeURIComponent(value);
}

function appendScalar(entries: string[], key: string, value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }
  entries.push(`${encodePart(key)}=${encodePart(String(value))}`);
}

export function stringifyQuery(input: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(input).sort()) {
    const value = input[key];
    if (Array.isArray(value)) {
      for (const item of value) {
        appendScalar(entries, key, item);
      }
      continue;
    }
    if (value && typeof value === "object") {
      for (const childKey of Object.keys(value as Record<string, unknown>).sort()) {
        appendScalar(entries, `${key}[${childKey}]`, (value as Record<string, unknown>)[childKey]);
      }
      continue;
    }
    appendScalar(entries, key, value);
  }
  return entries.join("&");
}
