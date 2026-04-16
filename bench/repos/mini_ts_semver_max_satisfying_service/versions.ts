import { compareVersions, normalizeVersion } from "./compare.ts";
import { matchesRange } from "./range.ts";

export function maxSatisfying(
  versions: Array<string | null | undefined>,
  rangeText: string,
): string | null {
  let best: string | null = null;
  for (const raw of versions) {
    const version = normalizeVersion(raw);
    if (!version || !matchesRange(version, rangeText)) {
      continue;
    }
    if (best === null || compareVersions(version, best) > 0) {
      best = version;
    }
  }
  return best;
}
