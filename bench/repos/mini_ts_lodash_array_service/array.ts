import { normalizeChunkSize, sameValueZero } from "./internals.ts";

export function chunk<T>(values: T[], size = 1): T[][] {
  const normalized = normalizeChunkSize(size);
  const result: T[][] = [];
  for (let index = 0; index < values.length; index += normalized) {
    result.push(values.slice(index, index + normalized));
  }
  return result;
}

export function flatten<T>(values: Array<T | T[]>): Array<T | T[]> {
  const result: Array<T | T[]> = [];
  for (const value of values) {
    if (Array.isArray(value)) {
      for (const inner of value) {
        if (Array.isArray(inner)) {
          result.push(...flatten(inner as Array<T | T[]>));
        } else {
          result.push(inner);
        }
      }
    } else {
      result.push(value);
    }
  }
  return result;
}

export function uniq<T>(values: T[]): T[] {
  const result: T[] = [];
  for (const value of values) {
    if (!result.some((existing) => sameValueZero(existing, value))) {
      result.push(value);
    }
  }
  return result;
}
