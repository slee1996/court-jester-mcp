import {
  cloneWithInherited,
  flattenPathArgs,
  getAtPath,
  keysIn,
  MISSING,
  PathValue,
  setAtPath,
  shouldAssignDefault,
  toPath,
  unsetAtPath,
} from "./internals.ts";

export function defaults<T extends object>(object: T, ...sources: Array<object | null | undefined>): T {
  const target = Object(object) as Record<string, unknown>;
  for (const source of sources) {
    if (source == null) {
      continue;
    }
    for (const key of keysIn(source)) {
      if (shouldAssignDefault(target, key)) {
        target[key] = (source as Record<string, unknown>)[key];
      }
    }
  }
  return target as T;
}

export function pick(object: unknown, ...paths: Array<PathValue | Array<PathValue>>): Record<string, unknown> {
  if (object == null) {
    return {};
  }
  const result: Record<string, unknown> = {};
  for (const rawPath of flattenPathArgs(paths)) {
    const path = toPath(rawPath, object);
    const value = getAtPath(object, path);
    if (value !== MISSING) {
      setAtPath(result, path, value);
    }
  }
  return result;
}

export function omit(object: unknown, ...paths: Array<PathValue | Array<PathValue>>): Record<string, unknown> {
  if (object == null) {
    return {};
  }
  const result = cloneWithInherited(object as Record<string, unknown>);
  for (const rawPath of flattenPathArgs(paths)) {
    unsetAtPath(result, toPath(rawPath, object));
  }
  return result;
}
