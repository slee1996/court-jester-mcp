export type PathValue = string | number | Array<string | number>;

const objectProto = Object.prototype;

export function flattenPathArgs(paths: Array<PathValue | Array<PathValue>>): PathValue[] {
  return paths.slice();
}

export function toPath(path: PathValue, object: unknown): string[] {
  if (Array.isArray(path)) {
    return path.map((part) => String(part));
  }
  return String(path).split(".");
}

export function getAtPath(object: unknown, path: string[]): unknown {
  let current = object;
  for (const part of path) {
    if (current == null || !(part in Object(current))) {
      return MISSING;
    }
    current = (current as Record<string, unknown>)[part];
  }
  return current;
}

export function setAtPath(target: Record<string, unknown>, path: string[], value: unknown): void {
  let current: Record<string, unknown> = target;
  for (let index = 0; index < path.length - 1; index += 1) {
    const part = path[index]!;
    const next = current[part];
    if (!isRecord(next)) {
      current[part] = {};
    }
    current = current[part] as Record<string, unknown>;
  }
  current[path[path.length - 1]!] = value;
}

export function cloneWithInherited<T>(value: T): T {
  if (Array.isArray(value)) {
    return value.map((item) => cloneWithInherited(item)) as T;
  }
  if (!isRecord(value)) {
    return value;
  }
  const result: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    result[key] = cloneWithInherited(item);
  }
  return result as T;
}

export function unsetAtPath(target: Record<string, unknown>, path: string[]): void {
  let current: Record<string, unknown> = target;
  for (let index = 0; index < path.length - 1; index += 1) {
    const part = path[index]!;
    const next = current[part];
    if (!isRecord(next)) {
      return;
    }
    current = next;
  }
  delete current[path[path.length - 1]!];
}

export function keysIn(value: unknown): string[] {
  const result: string[] = [];
  if (value == null) {
    return result;
  }
  for (const key in Object(value)) {
    result.push(key);
  }
  return result;
}

export function shouldAssignDefault(object: Record<string, unknown>, key: string): boolean {
  const value = object[key];
  return value == null || (value === objectProto[key] && !Object.hasOwn(object, key));
}

export const MISSING = Symbol("missing");

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
