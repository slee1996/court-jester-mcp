function _cjNativeValue(value: unknown): unknown {
  function expression(item: unknown, depth = 0): string {
    if (depth > 16) throw new Error("native snapshot exceeds supported depth");
    if (item === undefined) return "undefined";
    if (item === null) return "null";
    if (typeof item === "bigint") return item.toString() + "n";
    if (typeof item === "number") {
      if (Object.is(item, -0)) return "-0";
      return String(item);
    }
    if (typeof item === "string" || typeof item === "boolean") return JSON.stringify(item);
    if (item instanceof Date) return "new Date(" + expression(item.getTime()) + ")";
    if (item instanceof Uint8Array) {
      const bytes = JSON.stringify(Array.from(item));
      return Buffer.isBuffer(item) ? "Buffer.from(" + bytes + ")" : "new Uint8Array(" + bytes + ")";
    }
    if (Array.isArray(item)) return "[" + item.map(child => expression(child, depth + 1)).join(", ") + "]";
    throw new Error("unsupported native snapshot value");
  }
  // Expressions retain runtime types; optional JSON is only faithful JSON data.
  function jsonSafe(item: unknown): boolean {
    if (item === null || typeof item === "string" || typeof item === "boolean") return true;
    if (typeof item === "number") return Number.isFinite(item) && !Object.is(item, -0);
    return Array.isArray(item) && item.every(jsonSafe);
  }
  const snapshot: {expression: string, json_value?: unknown} = {expression: expression(value)};
  if (jsonSafe(value)) snapshot.json_value = JSON.parse(JSON.stringify(value));
  return snapshot;
}
