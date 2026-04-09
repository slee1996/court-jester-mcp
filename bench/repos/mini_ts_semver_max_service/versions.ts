import { compareVersions, normalizeVersion } from "./compare.ts";

export function maxStableVersion(versions: Array<string | null | undefined>): string | null {
  let best: string | null = null;
  for (const raw of versions) {
    const version = normalizeVersion(raw);
    if (!version) {
      continue;
    }
    if (best === null || compareVersions(version, best) > 0) {
      best = version;
    }
  }
  return best;
}
