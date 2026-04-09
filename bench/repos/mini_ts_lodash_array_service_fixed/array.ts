import { normalizeChunkSize, sameValueZero } from "./internals.ts";

export function chunk<T>(values: T[], size?: number): T[][] {
  const normalized = normalizeChunkSize(size);
  if (!Array.isArray(values) || values.length === 0 || normalized < 1) {
    return [];
  }
  const result: T[][] = [];
  for (let index = 0; index < values.length; index += normalized) {
    result.push(values.slice(index, index + normalized));
  }
  return result;
}

export function flatten<T>(values: unknown): Array<T | T[]> {
  if (!Array.isArray(values)) {
    return [];
  }
  const result: Array<T | T[]> = [];
  for (const value of values) {
    if (Array.isArray(value)) {
      result.push(...value);
    } else {
      result.push(value as T);
    }
  }
  return result;
}

export function uniq<T>(values: T[]): T[] {
  if (!Array.isArray(values) || values.length === 0) {
    return [];
  }
  const result: T[] = [];
  for (const value of values) {
    if (!result.some((existing) => sameValueZero(existing, value))) {
      result.push(value);
    }
  }
  return result;
}
