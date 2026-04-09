export function normalizeChunkSize(size: number | undefined): number {
  if (size == null || !Number.isFinite(size)) {
    return 1;
  }
  return Math.max(1, Math.trunc(size));
}

export function sameValueZero(left: unknown, right: unknown): boolean {
  return left === right;
}
