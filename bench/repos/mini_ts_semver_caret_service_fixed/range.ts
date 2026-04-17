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

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base || candidate.prerelease != null) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  if (base.major > 0) {
    return candidate.major === base.major;
  }
  if (base.minor > 0) {
    return candidate.major === 0 && candidate.minor === base.minor;
  }
  return candidate.major === 0 && candidate.minor === 0 && candidate.patch === base.patch;
}
