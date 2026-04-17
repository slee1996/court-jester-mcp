function canonicalScalar(value: unknown): string | null {
  if (value == null || Array.isArray(value) || (typeof value === "object" && value !== null)) {
    return null;
  }
  const text = String(value).trim().normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
  return text.length > 0 ? text : null;
}

export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const raw = params[key];
    const values = Array.isArray(raw) ? raw : [raw];
    for (const item of values) {
      const text = canonicalScalar(item);
      if (text == null) {
        continue;
      }
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(text)}`);
    }
  }
  return entries.join("&");
}
