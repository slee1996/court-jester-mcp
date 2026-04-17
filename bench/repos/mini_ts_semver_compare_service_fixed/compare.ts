type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareIdentifiers(left: string, right: string): number {
  const leftNumeric = /^\d+$/.test(left);
  const rightNumeric = /^\d+$/.test(right);
  if (leftNumeric && rightNumeric) {
    const a = Number.parseInt(left, 10);
    const b = Number.parseInt(right, 10);
    return a === b ? 0 : a < b ? -1 : 1;
  }
  if (leftNumeric) return -1;
  if (rightNumeric) return 1;
  return left === right ? 0 : left < right ? -1 : 1;
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  if (a.prerelease == null && b.prerelease == null) return 0;
  if (a.prerelease == null) return 1;
  if (b.prerelease == null) return -1;
  for (let i = 0; i < Math.min(a.prerelease.length, b.prerelease.length); i++) {
    const cmp = compareIdentifiers(a.prerelease[i], b.prerelease[i]);
    if (cmp !== 0) return cmp;
  }
  if (a.prerelease.length === b.prerelease.length) return 0;
  return a.prerelease.length < b.prerelease.length ? -1 : 1;
}
