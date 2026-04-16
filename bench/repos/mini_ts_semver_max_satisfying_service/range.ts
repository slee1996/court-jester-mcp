import { normalizeVersion, parseVersion } from "./compare.ts";

function compareCore(
  left: ReturnType<typeof parseVersion>,
  right: ReturnType<typeof parseVersion>,
): number {
  if (!left || !right) {
    return 0;
  }
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesRange(version: string, rangeText: string): boolean {
  const range = rangeText.trim();
  if (!range) {
    return false;
  }
  const candidate = parseVersion(version);
  if (!candidate) {
    return false;
  }
  if (range.startsWith("^")) {
    const base = parseVersion(range.slice(1));
    if (!base) {
      return false;
    }
    if (compareCore(candidate, base) < 0) {
      return false;
    }
    return candidate.major === base.major;
  }
  const normalizedCandidate = normalizeVersion(version);
  const normalizedRange = normalizeVersion(range);
  if (!normalizedCandidate || !normalizedRange) {
    return false;
  }
  return normalizedCandidate === normalizedRange;
}
