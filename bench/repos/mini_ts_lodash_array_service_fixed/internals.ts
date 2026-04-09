export function normalizeChunkSize(size: unknown): number {
  if (size === undefined) {
    return 1;
  }
  const coerced = Number(size);
  if (!Number.isFinite(coerced)) {
    return 0;
  }
  return Math.max(Math.trunc(coerced), 0);
}

export function sameValueZero(left: unknown, right: unknown): boolean {
  return left === right || (left !== left && right !== right);
}
