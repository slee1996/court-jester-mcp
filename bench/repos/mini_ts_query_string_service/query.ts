export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const value = params[key];
    if (value == null) {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(item).trim())}`);
      }
    } else {
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value).trim())}`);
    }
  }
  return entries.join("&");
}
