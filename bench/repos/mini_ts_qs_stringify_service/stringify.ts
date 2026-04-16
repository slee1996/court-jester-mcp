function encode(value: string): string {
  return encodeURIComponent(value);
}

export function stringifyQuery(input: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(input).sort()) {
    const value = input[key];
    if (Array.isArray(value)) {
      for (const item of value) {
        entries.push(`${encode(key)}=${encode(String(item))}`);
      }
      continue;
    }
    entries.push(`${encode(key)}=${encode(String(value))}`);
  }
  return entries.join("&");
}
