// Runs without target code. Preserve runtime constructors and Unicode scalars.
function* _cjShrink(value: any, depth = 0): Generator<any> {
  if (depth > 16) return;
  if (typeof value === 'boolean') {
    if (value) yield false;
  } else if (typeof value === 'number') {
    if (value !== 0 || Object.is(value, -0)) {
      yield 0;
      if (Number.isFinite(value) && Math.abs(value) > 1) {
        yield Math.sign(value);
        yield Number.isInteger(value) ? Math.trunc(value / 2) : value / 2;
      }
    }
  } else if (typeof value === 'bigint') {
    if (value !== 0n) {
      yield 0n;
      if (value > 1n || value < -1n) {
        yield value > 0n ? 1n : -1n;
        yield value / 2n;
      }
    }
  } else if (value instanceof Date) {
    for (const smaller of _cjShrink(value.getTime(), depth + 1)) yield new Date(smaller);
  } else if (typeof value === 'string' || Array.isArray(value) || value instanceof Uint8Array) {
    const items: any[] = Array.from(value as any);
    const restore = (parts: any[]) => typeof value === 'string' ? parts.join('')
      : Buffer.isBuffer(value) ? Buffer.from(parts)
      : value instanceof Uint8Array ? new Uint8Array(parts) : parts;
    if (items.length) yield restore([]);
    for (let chunk = Math.max(1, Math.floor(items.length / 2)); items.length && chunk; chunk = Math.floor(chunk / 2)) {
      for (let start = 0; start < items.length; start += chunk)
        yield restore([...items.slice(0, start), ...items.slice(start + chunk)]);
    }
    if (typeof value !== 'string') {
      for (let index = 0; index < items.length; index++) {
        for (const smaller of _cjShrink(items[index], depth + 1)) {
          const parts = items.slice(); parts[index] = smaller;
          yield restore(parts);
        }
      }
    }
  }
}
function _cjCandidates() {
  const seen = new Set([JSON.stringify(_cj_args.map(_cjNativeValue))]);
  const candidates: unknown[][] = [];
  for (let index = 0; index < _cj_args.length; index++) {
    for (const smaller of _cjShrink(_cj_args[index])) {
      const candidate = _cj_args.slice(); candidate[index] = smaller;
      const snapshots = candidate.map(_cjNativeValue), key = JSON.stringify(snapshots);
      if (seen.has(key)) continue;
      seen.add(key);
      if (candidates.length === 32) return {candidates, truncated: true};
      candidates.push(snapshots);
    }
  }
  return {candidates, truncated: false};
}
console.log('__COURT_JESTER_NATIVE_CANDIDATES__');
console.log(JSON.stringify(_cjCandidates()));
