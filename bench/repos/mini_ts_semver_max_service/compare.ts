type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

export function normalizeVersion(input: string | null | undefined): string | null {
  if (typeof input !== "string") {
    return null;
  }
  const normalized = input.trim().replace(/^v/i, "");
  if (!normalized) {
    return null;
  }
  return normalized.split("+", 1)[0];
}

function parseVersion(input: string): ParsedVersion | null {
  const normalized = normalizeVersion(input);
  if (!normalized) {
    return null;
  }
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

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  return 0;
}
