
let _seed = 42;
let _cjSequence = 0;
let _cjCompletedUnits = 0;
function _cjEvent(event: string, data?: unknown): void {
  const payload: Record<string, unknown> = { protocol_version: 2, sequence: _cjSequence, event };
  if (data !== undefined) payload.data = data;
  console.log("__COURT_JESTER_EVENT_JSON__" + JSON.stringify(payload));
  _cjSequence++;
}
_cjEvent("bootstrap_started");
_cjEvent("target_resolved", { module: "generated" });
_cjEvent("target_ready");
function _cjUnitStarted(surfaceId: string, iteration: number, inputOrigin: string = "generated"): void {
  _cjEvent("unit_started", {
    surface_id: surfaceId,
    iteration,
    input_classification: "valid",
    input_origin: inputOrigin,
  });
}
function _cjUnitCompleted(surfaceId: string, iteration: number, outcome: string): void {
  _cjEvent("unit_completed", { surface_id: surfaceId, iteration, outcome });
  _cjCompletedUnits++;
}
function _targetEntered(surfaceId: string, iteration?: number): void {
  if (iteration !== undefined) {
    const inputOrigin = _CJ_SAFE_DEPENDENCY_SURFACES.has(surfaceId)
      ? "safe_dependency_substitute"
      : "generated";
    _cjUnitStarted(surfaceId, iteration, inputOrigin);
  }
  console.error(JSON.stringify({ event: "target_entered", surface_id: surfaceId }));
}
function _fuzzRand(): number { _seed = (_seed * 1103515245 + 12345) & 0x7fffffff; return _seed / 0x7fffffff; }
function _fuzzIntRange(lo: number, hi: number): number { return lo + Math.floor(_fuzzRand() * (hi - lo + 1)); }
function _fuzzNum(): number { return (_fuzzRand() - 0.5) * 2000; }
function _fuzzSemverPart(): number { return _fuzzIntRange(0, 1000); }
function _fuzzSemverIdentifier(): string {
  const pools = ["alpha", "beta", "rc", "0", "1", "build", "exp", "preview", "canary"];
  return pools[_fuzzIntRange(0, pools.length - 1)];
}
function _fuzzSemverVersion(): { major: number; minor: number; patch: number; prerelease: string[] | null } {
  const ids = [null, [], [_fuzzSemverIdentifier()], [_fuzzSemverIdentifier(), _fuzzSemverIdentifier()]];
  return {
    major: _fuzzSemverPart(),
    minor: _fuzzSemverPart(),
    patch: _fuzzSemverPart(),
    prerelease: ids[_fuzzIntRange(0, ids.length - 1)],
  };
}
function _fuzzBool(): boolean { return _fuzzRand() > 0.5; }
function _fuzzUndef(): undefined { return undefined; }
function _fuzzUnicodeScalar(): string {
  const value = _fuzzIntRange(0, 0xF7FF);
  return String.fromCodePoint(value >= 0xD800 ? value + 0x800 : value);
}
function _fuzzStr(): string {
  const pools = [
    "", "hello world", "café résumé", "  whitespace  ", "\t\nnewlines",
    "UPPER", "lower", "MiXeD", "special!@#$%^&*()", "12345", "-1.5",
    "a".repeat(200), "\xa0\xa0\xa0", "with\nnewlines\n",
    String.fromCharCode(...Array.from({length: _fuzzIntRange(0,20)}, () => _fuzzIntRange(32, 126))),
    Array.from({length: _fuzzIntRange(0,10)}, _fuzzUnicodeScalar).join(""),
  ];
  return pools[_fuzzIntRange(0, pools.length - 1)];
}
function _fuzzAny(): unknown {
  const v = [_fuzzNum(), _fuzzStr(), _fuzzBool(), null, undefined, [], _fuzzObject()];
  return v[_fuzzIntRange(0, v.length - 1)];
}
function _fuzzObject(): unknown {
  // Concrete object shapes come from the repository-derived domain plan.
  return {};
}

function _fuzzHeaders(): Headers {
  const headerSets: Array<Record<string, string>> = [
    {},
    { authorization: "Bearer token-123" },
    { authorization: "bearer token-123" },
    { authorization: "Bearer   token-123   " },
    { authorization: "Bearer " },
    { authorization: "Basic Zm9vOmJhcg==" },
    { authorization: "Bearer token-123, Bearer token-456" },
    { "content-type": "application/json", authorization: "Bearer token-123" },
  ];
  return new Headers(headerSets[_fuzzIntRange(0, headerSets.length - 1)]);
}

function _fuzzUrlSearchParams(): URLSearchParams {
  const pairs: Array<Array<[string, string]>> = [
    [],
    [["token", "abc123"]],
    [["tag", "pro"], ["tag", "beta"]],
    [["q", "naïve café"]],
    [["authorization", "Bearer token-123"]],
  ];
  return new URLSearchParams(pairs[_fuzzIntRange(0, pairs.length - 1)]);
}

function _fuzzRequest(): Request {
  const methods = ["GET", "POST", "PUT"];
  const headers = _fuzzHeaders();
  const body = [_fuzzStr(), JSON.stringify({ token: _fuzzStr() }), ""][_fuzzIntRange(0, 2)];
  const url = `https://example.com/${_fuzzStr().replace(/[^a-z0-9]/gi, "").slice(0, 12)}`;
  const method = methods[_fuzzIntRange(0, methods.length - 1)];
  return new Request(url || "https://example.com", {
    method,
    headers,
    body: method === "GET" ? undefined : body,
  });
}

function _fuzzResponse(): Response {
  const statuses = [200, 201, 400, 401, 403, 500];
  return new Response(_fuzzStr(), {
    status: statuses[_fuzzIntRange(0, statuses.length - 1)],
    headers: _fuzzHeaders(),
  });
}

const _EDGE_NUMS = [0, -0, Infinity, -Infinity, NaN, Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER + 1, 1e-300, 1e300];
const _EDGE_STRS = ["", "   ", "\0", "\u00A0", "\u00A0\u00A0\u00A0", "\uFFFF", "a".repeat(10000), "true", "null", "0", "-1", "\r\n", "\u200F", "\u200D", "${...}", "<script>"];
const _EDGE_STR_ARRAYS = [
  [],
  [""],
  ["   "],
  ["primary"],
  ["primary", ""],
  ["primary", "   "],
  ["primary", "Secondary"],
  ["zulu", "alpha"],
  ["secondary", "primary", "tertiary"],
];
const _EDGE_OBJECTS = [{}];
const _EDGE_UNKNOWNS = [undefined, null, NaN, Infinity, -Infinity, "", 0, false, {}, []];
const _EDGE_UNKNOWN_ARRAYS = [[], [undefined], [null], [NaN], [Infinity], [-Infinity], [""], [0]];
const _EDGE_SEMVER_OBJECTS = [
  { major: 0, minor: 0, patch: 0, prerelease: null },
  { major: 1, minor: 2, patch: 3, prerelease: [] },
  { major: 1, minor: 2, patch: 3, prerelease: ["alpha"] },
  { major: 0, minor: 1, patch: 0, prerelease: ["beta", "2"] },
];


function _edgeCasesFor(typeName: string, _template?: unknown): unknown[] {
  const m: Record<string, unknown[]> = {
    "number": _EDGE_NUMS,
    "string": _EDGE_STRS,
    "string_array": _EDGE_STR_ARRAYS,
    "unknown": _EDGE_UNKNOWNS,
    "unknown_array": _EDGE_UNKNOWN_ARRAYS,
    "object": _EDGE_OBJECTS,
    "semver_version": _EDGE_SEMVER_OBJECTS,
  };
  return m[typeName] || [];
}

function _nanSafeEq(a: unknown, b: unknown): boolean {
  if (typeof a === "number" && typeof b === "number") return Object.is(a, b);
  return JSON.stringify(a) === JSON.stringify(b);
}

function _containsNullish(value: unknown): boolean {
  if (value === null || value === undefined) return true;
  if (Array.isArray(value)) return value.some(_containsNullish);
  if (value && typeof value === "object") {
    return Object.values(value as Record<string, unknown>).some(_containsNullish);
  }
  return false;
}

function _stringLeaksNullish(value: string): boolean {
  const lower = value.toLowerCase();
  return lower.includes("null") || lower.includes("undefined");
}

function _asciiFold(value: string): string {
  return value.normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
}

function _cmpSign(value: unknown): number {
  if (typeof value !== "number" || Number.isNaN(value)) {
    throw new _PropertyFailure(`Comparator returned non-numeric value: ${JSON.stringify(value)}`);
  }
  if (value < 0) return -1;
  if (value > 0) return 1;
  return 0;
}

function _isPrimitiveSortableArray(value: unknown): value is Array<number | string> {
  return Array.isArray(value) && value.every((item) => typeof item === "number" || typeof item === "string");
}

function _samePrimitiveMultiset(a: unknown[], b: unknown[]): boolean {
  const counts = new Map<string, number>();
  const keyFor = (value: unknown): string => `${typeof value}:${JSON.stringify(value)}`;
  for (const item of a) {
    const key = keyFor(item);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  for (const item of b) {
    const key = keyFor(item);
    const next = (counts.get(key) ?? 0) - 1;
    if (next < 0) return false;
    if (next === 0) {
      counts.delete(key);
    } else {
      counts.set(key, next);
    }
  }
  return counts.size === 0;
}

const _FUZZ_TEXT_LIMIT = 240;
function _clipText(value: unknown, limit = _FUZZ_TEXT_LIMIT): string {
  const text = typeof value === "string" ? value : String(value);
  if (text.length <= limit) return text;
  return `${text.slice(0, limit)}... [truncated ${text.length - limit} chars]`;
}

function _isReservedReproObject(value: Record<string, unknown>): boolean {
  const keys = Object.keys(value);
  if (keys.length === 1 && value.type === "undefined") return true;
  if (
    keys.length === 2
    && value.type === "number"
    && keys.includes("value")
    && (value.value === "NaN" || value.value === "Infinity" || value.value === "-Infinity" || value.value === "-0")
  ) return true;
  return keys.length === 2 && value.type === "object" && keys.includes("value");
}

function _reproJsonValue(value: unknown, ancestors = new Set<object>()): unknown {
  if (value === undefined) return { type: "undefined" };
  if (typeof value === "number") {
    if (Number.isNaN(value)) return { type: "number", value: "NaN" };
    if (value === Infinity) return { type: "number", value: "Infinity" };
    if (value === -Infinity) return { type: "number", value: "-Infinity" };
    if (Object.is(value, -0)) return { type: "number", value: "-0" };
    return value;
  }
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (value instanceof URL) throw new TypeError("URL values are not JSON-serializable");
  if (Array.isArray(value)) {
    if (ancestors.has(value)) throw new TypeError("circular repro value");
    ancestors.add(value);
    try { return value.map((item) => _reproJsonValue(item, ancestors)); }
    finally { ancestors.delete(value); }
  }
  if (typeof value === "object") {
    if (ancestors.has(value)) throw new TypeError("circular repro value");
    ancestors.add(value);
    try {
      const encoded = Object.fromEntries(Object.keys(value).map((key) => [key, _reproJsonValue((value as Record<string, unknown>)[key], ancestors)]));
      return _isReservedReproObject(encoded) ? { type: "object", value: encoded } : encoded;
    } finally { ancestors.delete(value); }
  }
  return null;
}

function _reproExpression(value: unknown, ancestors = new Set<object>()): string {
  if (value === undefined) return "undefined";
  if (typeof value === "number") {
    if (Number.isNaN(value)) return "NaN";
    if (value === Infinity) return "Infinity";
    if (value === -Infinity) return "-Infinity";
    if (Object.is(value, -0)) return "-0";
    return String(value);
  }
  if (value === null) return "null";
  if (typeof value === "string" || typeof value === "boolean") return JSON.stringify(value);
  if (value instanceof URL) return `new URL(${JSON.stringify(value.href)})`;
  if (typeof value === "bigint") return `${value}n`;
  if (Array.isArray(value)) {
    if (ancestors.has(value)) throw new TypeError("circular repro value");
    ancestors.add(value);
    try { return `[${value.map((item) => _reproExpression(item, ancestors)).join(",")}]`; }
    finally { ancestors.delete(value); }
  }
  if (typeof value === "object") {
    if (ancestors.has(value)) throw new TypeError("circular repro value");
    ancestors.add(value);
    try {
      return `{${Object.keys(value).map((key) => `[${JSON.stringify(key)}]:${_reproExpression((value as Record<string, unknown>)[key], ancestors)}`).join(",")}}`;
    } finally { ancestors.delete(value); }
  }
  return String(value);
}

function _shortJson(value: unknown, limit = _FUZZ_TEXT_LIMIT): string {
  try {
    return _clipText(_reproExpression(value), limit);
  } catch {
    return _clipText(value, limit);
  }
}

function _cloneSeedFallback<T>(value: T, seen: Map<object, unknown>): T {
  if (value === null || typeof value !== "object") return value;
  if (value instanceof URL) return new URL(value.href) as T;
  if (seen.has(value)) return seen.get(value) as T;
  if (Array.isArray(value)) {
    const clone: unknown[] = [];
    seen.set(value, clone);
    for (const item of value) clone.push(_cloneSeedFallback(item, seen));
    return clone as T;
  }
  const clone = Object.create(Object.getPrototypeOf(value)) as Record<string, unknown>;
  seen.set(value, clone);
  for (const key of Object.keys(value)) {
    clone[key] = _cloneSeedFallback((value as Record<string, unknown>)[key], seen);
  }
  return clone as T;
}

function _cloneSeed<T>(value: T): T {
  // structuredClone turns platform URL instances into plain empty objects in
  // Node runtimes. Clone the argument vector recursively so URLs retain their
  // internal slots before using the native clone for other values.
  if (value instanceof URL) return new URL(value.href) as T;
  if (
    Array.isArray(value)
    && value.some((item) => item instanceof URL || (Array.isArray(item) && item.some((nested) => nested instanceof URL)))
  ) {
    return value.map((item) => _cloneSeed(item)) as T;
  }
  if (typeof structuredClone === "function") {
    try {
      return structuredClone(value);
    } catch {
      // Runtime collaborators may contain functions, which structuredClone rejects.
    }
  }
  return _cloneSeedFallback(value, new Map());
}

function _isSortedNumericArray(value: unknown): value is number[] {
  return Array.isArray(value)
    && value.every((item) => typeof item === "number" && !Number.isNaN(item))
    && value.every((item, index) => index === 0 || value[index - 1] <= item);
}

function _fuzzLikeSeed(value: unknown): unknown {
  if (typeof value === "boolean") return [value, !value][_fuzzIntRange(0, 1)];
  if (typeof value === "number") return [value, value - 1, value + 1, 0, -0][_fuzzIntRange(0, 4)];
  if (typeof value === "string") return [value, value.trim(), value.toUpperCase(), value.toLowerCase()][_fuzzIntRange(0, 3)];
  if (value === null || value === undefined) return value;
  if (value instanceof URL) return new URL(value.href);
  if (Array.isArray(value)) {
    if (value.length === 0) return [];
    if (_isSortedNumericArray(value)) {
      const delta = [-1, 0, 1][_fuzzIntRange(0, 2)];
      return value.map((item) => item + delta);
    }
    return value.map(_fuzzLikeSeed);
  }
  if (typeof value === "object") {
    return Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, item]) => [key, _fuzzLikeSeed(item)]));
  }
  return _cloneSeed(value);
}

type _FuzzInput = { args: unknown[]; contractValid: boolean };
function _sameInput(left: unknown, right: unknown, depth = 0): boolean {
  if (depth > 32) return false;
  if (Object.is(left, right)) return true;
  if (left === null || right === null || typeof left !== "object" || typeof right !== "object") return false;
  const prototype = Object.getPrototypeOf(left);
  if (prototype !== Object.getPrototypeOf(right) || (!Array.isArray(left) && prototype !== Object.prototype && prototype !== null)) return false;
  const keys = Reflect.ownKeys(left);
  if (keys.length !== Reflect.ownKeys(right).length) return false;
  return keys.every((key) => {
    const a = Object.getOwnPropertyDescriptor(left, key);
    const b = Object.getOwnPropertyDescriptor(right, key);
    return a !== undefined && b !== undefined && "value" in a && "value" in b && _sameInput(a.value, b.value, depth + 1);
  });
}
function _fuzzSeedRow(seedRows: _FuzzInput[]): _FuzzInput {
  const seed = seedRows[_fuzzIntRange(0, seedRows.length - 1)];
  const row = _cloneSeed(seed.args);
  return _fuzzRand() < 0.65
    ? { args: row, contractValid: seed.contractValid }
    : { args: row.map(_fuzzLikeSeed), contractValid: false };
}

const _cjCorpora = new Map<string, unknown[][]>();
function _behaviorSignature(outcome: string, value: unknown): string {
  if (value instanceof Error) {
    return `${outcome}:error:${value.constructor.name}:${value.message.split(":", 1)[0]}`;
  }
  if (value === null) return `${outcome}:null`;
  if (value === undefined) return `${outcome}:undefined`;
  if (Array.isArray(value)) {
    return `${outcome}:array:${Math.min(value.length, 8)}:${value.slice(0, 4).map((item) => typeof item).join(",")}`;
  }
  if (typeof value === "object") {
    return `${outcome}:object:${Object.keys(value as Record<string, unknown>).sort().slice(0, 12).join(",")}`;
  }
  if (typeof value === "number") {
    const bucket = Number.isNaN(value) ? "nan" : !Number.isFinite(value) ? String(value) : value === 0 ? "zero" : value < 0 ? "negative" : "positive";
    return `${outcome}:number:${bucket}`;
  }
  if (typeof value === "string") {
    return `${outcome}:string:${value.length === 0 ? "empty" : value.trim().length === 0 ? "blank" : Math.min(value.length, 32)}`;
  }
  return `${outcome}:${typeof value}:${String(value)}`;
}
function _mutateCorpusRow(row: unknown[]): unknown[] {
  const candidate = _cloneSeed(row);
  if (candidate.length === 0) return candidate;
  const index = _fuzzIntRange(0, candidate.length - 1);
  const value = candidate[index];
  // Campaign rows are classified as valid inputs. Mutate values recursively while
  // preserving required object keys and tuple/array shape.
  candidate[index] = _fuzzLikeSeed(value);
  return candidate;
}
function _retainCorpusInput(
  corpus: unknown[][],
  signatures: Set<string>,
  signature: string,
  args: unknown[],
): boolean {
  if (signatures.has(signature) || corpus.length >= 64) return false;
  signatures.add(signature);
  corpus.push(_cloneSeed(args));
  return true;
}

// Crash detection: real bugs vs intentional validation errors
function _isMalformedUriError(e: unknown): boolean {
  return e instanceof URIError && /malformed uri|uri malformed/i.test(e.message);
}

function _isEngineTypeError(e: TypeError): boolean {
  return /Cannot (read|set) propert(y|ies) of |Cannot convert undefined or null to object| is not a function| is not iterable| is not a constructor|Cannot destructure property|Cannot use 'in' operator|Assignment to constant variable|Cannot assign to read only property|cannot be invoked without 'new'|Right-hand side of 'instanceof' is not|Reduce of empty array with no initial value/i.test(e.message);
}

function _isEngineRangeError(e: RangeError): boolean {
  return /Maximum call stack|Invalid array length|Array buffer allocation failed|Invalid typed array length/i.test(e.message);
}

class _PropertyFailure extends Error {}

function _isCrash(e: unknown): boolean {
  if (e instanceof _PropertyFailure) return true;
  if (_isMalformedUriError(e)) return false;
  if (e instanceof TypeError) return _isEngineTypeError(e);
  if (e instanceof RangeError) return _isEngineRangeError(e);
  if (e instanceof ReferenceError) return true;
  if (e instanceof URIError) return true;
  // Stack overflow
  if (e instanceof Error && e.message.includes("Maximum call stack")) return true;
  return false;
}

let _fuzzTotalFailures = 0;
const _fuzzResults: Array<Record<string, unknown>> = [];
const _findingOrdinals = new Map<string, number>();
function _sanitizeSymbol(value: unknown): string { return String(value).replace(/[^A-Za-z0-9._-]/g, "_"); }
function _findingId(name: string): string {
  const symbol = _sanitizeSymbol(name); const ordinal = (_findingOrdinals.get(symbol) ?? 0) + 1; _findingOrdinals.set(symbol, ordinal);
  return `fuzz:${symbol}:${ordinal}`;
}
function _reproCase(args: unknown[], inputText: string | null = null): Record<string, unknown> {
  return {
    arguments: args.map((value) => ({
      expression: _reproExpression(value),
      json_value: (() => { try { return _reproJsonValue(value); } catch { return null; } })(),
    })),
    input_text: inputText,
  };
}
function _shrinkCandidates(value: unknown): unknown[] {
  const out: unknown[] = []; const seen = new Set<string>();
  const add = (candidate: unknown): void => { let key: string; try { key = JSON.stringify(_reproJsonValue(candidate)); } catch { key = String(candidate); } if (!seen.has(key)) { seen.add(key); out.push(candidate); } };
  if (value instanceof URL) { add(new URL("https://example.test/")); }
  else if (typeof value === "string") { add(""); add(value.slice(0, 1)); add(value.trim()); for (let step = Math.floor(value.length / 2); step > 0; step = Math.floor(step / 2)) for (let start = 0; start < value.length; start += step) add(value.slice(0, start) + value.slice(start + step)); }
  else if (typeof value === "number") { if (Number.isNaN(value) || !Number.isFinite(value)) add(value); else { add(0); add(1); add(-1); for (let current = value; current && Math.abs(current) > Number.EPSILON; current /= 2) add(current); } }
  else if (typeof value === "boolean") { add(false); }
  else if (Array.isArray(value)) {
    value.forEach((item, index) => {
      _shrinkCandidates(item).forEach((shrunk) => {
        const copy = value.slice();
        copy[index] = shrunk;
        add(copy);
      });
    });
  }
  else if (value && typeof value === "object"
      && (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null)) {
    const objectValue = value as Record<string, unknown>;
    for (const key of Object.keys(objectValue)) {
      _shrinkCandidates(objectValue[key]).forEach((shrunk) => {
        add({ ...objectValue, [key]: shrunk });
      });
    }
  }
  return out;
}
function _shrinkRank(value: unknown): [number, string] {
  let rendered: string;
  try { rendered = JSON.stringify(_reproJsonValue(value)); } catch { rendered = String(value); }
  return [rendered.length, rendered];
}
function _rankLess(candidate: unknown, current: unknown): boolean {
  const candidateRank = _shrinkRank(candidate); const currentRank = _shrinkRank(current);
  return candidateRank[0] < currentRank[0] || (candidateRank[0] === currentRank[0] && candidateRank[1] < currentRank[1]);
}
function _failureIdentity(error: unknown): string | null {
  const property = _declaredPropertyForFailure(error);
  if (property !== null) return `property:${property}`;
  if (error instanceof Error) return JSON.stringify(["exception", error.constructor.name, error.message]);
  if (error === null) return JSON.stringify(["null"]);
  if (typeof error === "number") return JSON.stringify(["number", Number.isNaN(error) ? "NaN" : Object.is(error, -0) ? "-0" : String(error)]);
  if (["undefined", "string", "boolean", "bigint"].includes(typeof error)) return JSON.stringify([typeof error, String(error)]);
  return null;
}
function _minimizeFailure(original: unknown[], reproduce: (candidate: unknown[]) => boolean): [string, number, unknown[]] {
  let current = _cloneSeed(original); let attempts = 0; const deadline = Date.now() + 250;
  while (attempts < 100 && Date.now() < deadline) {
    let improved = false;
    for (const candidateValue of _shrinkCandidates(current)) {
      if (attempts >= 100 || Date.now() >= deadline) break;
      const candidate = Array.isArray(candidateValue) ? candidateValue : [candidateValue];
      if (!_rankLess(candidate, current)) continue;
      attempts++;
      if (reproduce(candidate)) {
        current = _cloneSeed(candidate);
        improved = true;
        break;
      }
    }
    if (!improved) break;
  }
  return [reproduce(current) ? "preserved" : "failed", attempts, current];
}
function _emitFinding(name: string, args: unknown[], error: unknown, severity = "crash", oracleKind = "runtime_contract", provenance = "language_runtime", confidence = "high", category = "exception", minimize: [string, number, unknown[]] | null = null, invocationPath: unknown = "direct", caseLabel: string | null = null, sourceLine = 0, replaySnippet: string | null = null, inputClassification = "valid", expected: string | null = null, reproKind = "function_call", originalCase: Record<string, unknown> | null = null): void {
  const status = minimize?.[0] ?? "not_needed"; const attempts = minimize?.[1] ?? 0; const minimized = status === "not_needed" || status === "failed" ? null : _reproCase(minimize![2], caseLabel); const reproArgs = minimized ? minimize![2] : args;
  const expectation = { severity, oracle_kind: oracleKind, category }; const message = error instanceof Error ? error.message : String(error);
  const errorType = error instanceof Error ? error.constructor.name : "unknown";
  const primitiveException = error === null || ["undefined", "string", "number", "boolean", "bigint"].includes(typeof error);
  const replayMatch = error instanceof Error
    ? `_replayError instanceof Error && _replayError.constructor.name === ${JSON.stringify(errorType)} && _replayError.message === ${JSON.stringify(message)}`
    : primitiveException ? `Object.is(_replayError, ${_reproExpression(error)})` : null;
  const snippet = replaySnippet ?? (replayMatch === null ? `throw new Error("Court Jester cannot replay this runtime-only thrown value");` : `// Court Jester replay snippet\nlet _reproduced = false, _checkPassed = false;\ntry { (${name} as Function)(${reproArgs.map((value) => _reproExpression(value)).join(", ")}); _checkPassed = true; } catch (_replayError) { _reproduced = ${replayMatch}; }\nconsole.log("__COURT_JESTER_REPLAY_JSON__");\nconsole.log(JSON.stringify({reproduced:_reproduced,check_passed:_checkPassed,severity:${JSON.stringify(severity)},oracle_kind:${JSON.stringify(oracleKind)},category:${JSON.stringify(category)}}));`);
  const recordedCase = originalCase ?? _reproCase(args, caseLabel);
  const record: Record<string, unknown> = { id: _findingId(name), severity, confidence, category, location: { source_file: "", function: name, line: sourceLine, invocation_path: invocationPath }, oracle: { id: `${oracleKind}:${_sanitizeSymbol(name)}`, kind: oracleKind, provenance, confidence, expected, actual: message }, input_classification: inputClassification, repro: { kind: reproKind, function: name, arguments: recordedCase.arguments, case_label: caseLabel, snippet, command: null, expectation }, minimization: { status, attempts, original: recordedCase, minimized }, error_type: errorType, message, suppressed: false };
  _fuzzResults.push(record);
  _cjEvent("finding", { finding: record });
}
function _resolveFactoryAction(result: unknown, action: string, single: boolean): unknown {
  if (typeof result === "function" && single) return result;
  return result && (typeof result === "object" || typeof result === "function")
    ? (result as Record<string, unknown>)[action] : undefined;
}
function _factoryArgumentExpression(value: unknown): string | null {
  const ancestors = new Set<object>();
  const seen = new Set<object>();
  const validate = (item: unknown): void => {
    if (item === null || ["undefined", "string", "number", "boolean", "bigint"].includes(typeof item)) return;
    if (typeof item !== "object" || seen.has(item) || ancestors.size >= 64) throw new Error("unsupported factory input");
    seen.add(item);
    if (item instanceof URL && Object.getPrototypeOf(item) === URL.prototype && Reflect.ownKeys(item).length === 0) return;
    if (!Array.isArray(item) && Object.getPrototypeOf(item) !== Object.prototype) throw new Error("runtime-only factory input");
    if (Array.isArray(item) && (Object.getPrototypeOf(item) !== Array.prototype || Object.keys(item).length !== item.length)) throw new Error("nonstandard factory array");
    ancestors.add(item);
    try {
      for (const key of Reflect.ownKeys(item)) {
        if (Array.isArray(item) && key === "length") continue;
        const descriptor = Object.getOwnPropertyDescriptor(item, key)!;
        if (typeof key !== "string" || !descriptor.enumerable || !descriptor.writable || !descriptor.configurable || !("value" in descriptor)
            || (Array.isArray(item) && !/^(0|[1-9][0-9]*)$/.test(key))) throw new Error("runtime-only factory property");
        validate(descriptor.value);
      }
    } finally { ancestors.delete(item); }
  };
  try { validate(value); return _reproExpression(value); } catch { return null; }
}
function _factoryReplaySnippet(invoke: (args: unknown[]) => unknown, caseSource: string | null, single: boolean, phase: string, error: unknown): string {
  const primitive = error === null || ["undefined", "string", "number", "boolean", "bigint"].includes(typeof error);
  const match = error instanceof Error
    ? `_error instanceof Error && _error.constructor.name === ${JSON.stringify(error.constructor.name)} && _error.message === ${JSON.stringify(error.message)}`
    : primitive ? `Object.is(_error, ${_reproExpression(error)})` : null;
  if (caseSource === null || phase.startsWith("arguments") || match === null) return 'throw new Error("Court Jester cannot replay this runtime-only factory observation");';
  return `((_invoke) => {\n${_resolveFactoryAction.toString()}\nconst _case = ${caseSource};\nlet _phase = "factory", _reproduced = false, _checkPassed = false, _traceComplete = true;\ntry {\n const _result = _invoke(_case.factory);\n for (let _index = 0; _index < _case.actions.length; _index++) {\n  const _entry = _case.actions[_index];\n  _phase = "resolve:" + _index;\n  const _candidate = _resolveFactoryAction(_result, _entry.action, ${single});\n  if ((typeof _candidate === "function") !== _entry.callable) { _traceComplete = false; break; }\n  if (typeof _candidate !== "function") continue;\n  _phase = "action:" + _index;\n  _candidate.apply(_result, _entry.args);\n }\n _checkPassed = _traceComplete;\n} catch (_error) { _reproduced = _phase === ${JSON.stringify(phase)} && (${match}); }\nconsole.log("__COURT_JESTER_REPLAY_JSON__");\nconsole.log(JSON.stringify({reproduced:_reproduced,check_passed:_checkPassed,severity:"crash",oracle_kind:"runtime_contract",category:"exception"}));\n})(${invoke.toString()});`;
}
function _semanticProject(value: unknown, projection: string | { property: string } | ((value: any, step: (label: string) => void) => unknown), step: (label: string) => void): unknown {
  if (typeof projection === "function") return projection(value, step);
  if (typeof projection !== "string") return (value as Record<string, unknown>)[projection.property];
  switch (projection) {
    case "sign": return _cmpSign(value);
    case "bool": return Boolean(value);
    case "boolean_equal": {
      const values = value as unknown[];
      return values.length === 2 && Boolean(values[0]) === Boolean(values[1]);
    }
    case "query_pairs": return Array.from(new URLSearchParams(String(value)).entries());
    case "identity": return value;
    default: throw new Error(`Unknown semantic projection: ${projection}`);
  }
}
function _observeSemantic(invoke: (args: unknown[]) => unknown, args: unknown[], expected: unknown, projection: string | { property: string } | ((value: any, step: (label: string) => void) => unknown), sequence: boolean) {
  let actual: unknown;
  let error: unknown;
  let threw = false;
  let phase = "invoke";
  let matched = false;
  try {
    let value: unknown;
    if (sequence) {
      const values: unknown[] = [];
      for (let index = 0; index < args.length; index++) {
        phase = "invoke:" + index;
        values.push(invoke(args[index] as unknown[]));
      }
      value = values;
    } else {
      value = invoke(args);
    }
    phase = "project";
    actual = _semanticProject(value, projection, (label) => { phase = "project:" + label; });
    phase = "compare";
    matched = _nanSafeEq(actual, expected);
  } catch (caught) { threw = true; error = caught; }
  return { actual, error, threw, phase, matched };
}
function _semanticCase(name: string, invoke: (args: unknown[]) => unknown, args: unknown[] | (() => unknown[]), expected: unknown, projection: string | { property: string } | ((value: any, step: (label: string) => void) => unknown), label: string, sequence = false, exceptionInputClassification = "valid", sourceLine = 0): boolean {
  if (projection === "sign") expected = _cmpSign(expected);
  const recipe = typeof args === "function" ? args : null;
  const original = recipe ? recipe() : _cloneSeed(args as unknown[]);
  const projectionSource = typeof projection === "function" ? projection.toString() : JSON.stringify(projection);
  const createSource = recipe ? recipe.toString() : `() => (${_reproExpression(original)})`;
  const recordedCase = recipe ? { arguments: original.map((_, index) => ({ expression: `(${createSource})()[${index}]` })), input_text: label } : null;
  const { actual, error, threw, phase, matched } = _observeSemantic(invoke, recipe ? recipe() : _cloneSeed(original), expected, projection, sequence);
  if (!threw && matched) return false;
  const primitiveException = error === null || ["undefined", "string", "number", "boolean", "bigint"].includes(typeof error);
  const replayMatch = error instanceof Error
    ? `_error instanceof Error && _error.constructor.name === ${JSON.stringify(error.constructor.name)} && _error.message === ${JSON.stringify(error.message)}`
    : primitiveException ? `Object.is(_error, ${_reproExpression(error)})` : null;
  const snippet = replayMatch === null ? `throw new Error("Court Jester cannot replay this runtime-only thrown value");`
    : `((_semanticInvoke, _semanticArguments, _semanticProjection) => {\n${_PropertyFailure.toString()}\n${_cmpSign.toString()}\n${_nanSafeEq.toString()}\n${_semanticProject.toString()}\n${_observeSemantic.toString()}\nconst _observation = _observeSemantic(_semanticInvoke, _semanticArguments(), ${_reproExpression(expected)}, _semanticProjection, ${sequence});\nconst _error = _observation.error;\nconst _reproduced = ${threw ? `_observation.threw && _observation.phase === ${JSON.stringify(phase)} && (${replayMatch})` : "!_observation.threw && !_observation.matched"};\nconsole.log("__COURT_JESTER_REPLAY_JSON__");\nconsole.log(JSON.stringify({reproduced: _reproduced, check_passed: !_observation.threw && _observation.matched, severity: "property_violation", oracle_kind: "inferred_semantic", category: "property"}));\n})(${invoke.toString()}, ${createSource}, ${projectionSource});`;
  const failure = threw ? error : new Error(`${label}: ${JSON.stringify(actual)} !== ${JSON.stringify(expected)}`);
  _emitFinding(name, original, failure, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", null, "direct", label, sourceLine, snippet, threw ? exceptionInputClassification : "valid", JSON.stringify(expected), sequence ? "semantic_case" : "function_call", recordedCase);
  console.log(`  CRASH ${name}(${label}): ${_clipText(failure instanceof Error ? failure.message : String(failure))}`);
  _fuzzTotalFailures++;
  return true;
}
function _declaredPropertyForFailure(error: unknown): string | null {
  if (!(error instanceof _PropertyFailure)) return null;
  const mappings: Array<[string, string]> = [
    ["Not idempotent", "idempotent"], ["Not bounded", "bounded"],
    ["Non-negative", "nonneg"], ["Blank string output", "nonempty_string"],
    ["Nullish string leak", "no_nullish_string"], ["Not symmetric", "symmetric"],
    ["Not sorted", "sorted"], ["Permutation violated", "permutation"],
    ["Clamp bounds violated", "clamped"], ["Clamp passthrough violated", "clamped"],
    ["Comparator", "antisymmetric"],
    ["Involution violated", "involution"], ["Monotonicity violated", "monotonic"],
    ["Order invariance violated", "order_invariant"],
  ];
  return mappings.find(([prefix]) => error.message.startsWith(prefix))?.[1] ?? null;
}
function _evaluateProperties(fn: (args: unknown[]) => unknown, args: unknown[], result: unknown, expectedType: string | null, properties: string[], onCheck: (oracleId: string, passed: boolean) => void = () => {}): void {
  const violates = (oracleId: string, condition: boolean): boolean => { onCheck(oracleId, !condition); return condition; };
  // Type check
  if (expectedType !== null && violates("return_type", typeof result !== expectedType)) {
    throw new _PropertyFailure(`Return type mismatch: expected ${expectedType}, got ${typeof result}`);
  }
  // Consistency: same input → same output
  if (properties.includes("consistent")) {
    const result2 = fn(_cloneSeed(args));
    if (violates("consistent", !_nanSafeEq(result, result2))) {
      throw new _PropertyFailure(`Inconsistent: ${JSON.stringify(result)} !== ${JSON.stringify(result2)}`);
    }
  }
  // Idempotency: f(f(x)) === f(x)
  if (properties.includes("idempotent")) {
    const result3 = fn([result]);
    if (violates("idempotent", !_nanSafeEq(result, result3))) {
      throw new _PropertyFailure(`Not idempotent: ${JSON.stringify(result)} -> ${JSON.stringify(result3)}`);
    }
  }
  if (properties.includes("involution")) {
    const involutionResult = fn([_cloneSeed(result)]);
    if (violates("involution", !_nanSafeEq(args[0], involutionResult))) {
      throw new _PropertyFailure(`Involution violated: ${JSON.stringify(args[0])} -> ${JSON.stringify(result)} -> ${JSON.stringify(involutionResult)}`);
    }
  }
  if (properties.includes("monotonic") && typeof args[0] === "number" && Number.isFinite(args[0]) && typeof result === "number") {
    const monotonicArgs = _cloneSeed(args);
    monotonicArgs[0] = args[0] + 1;
    const monotonicResult = fn(monotonicArgs);
    if (violates("monotonic", typeof monotonicResult !== "number" || !(monotonicResult >= result))) {
      throw new _PropertyFailure(`Monotonicity violated: f(${JSON.stringify(args[0])})=${JSON.stringify(result)} > f(${JSON.stringify(monotonicArgs[0])})=${JSON.stringify(monotonicResult)}`);
    }
  }
  if (properties.includes("order_invariant") && Array.isArray(args[0])) {
    const reorderedArgs = _cloneSeed(args);
    reorderedArgs[0] = [...args[0]].reverse();
    const reorderedResult = fn(reorderedArgs);
    if (violates("order_invariant", !_nanSafeEq(result, reorderedResult))) {
      throw new _PropertyFailure(`Order invariance violated: ${JSON.stringify(result)} != ${JSON.stringify(reorderedResult)}`);
    }
  }
  // Boundedness: len(f(x)) <= len(x)
  if (properties.includes("bounded") && ((typeof args[0] === "string" && typeof result === "string") || (Array.isArray(args[0]) && Array.isArray(result)))) {
    const inp = args[0];
    if (violates("bounded", (result as any).length > (inp as any).length)) {
      throw new _PropertyFailure(`Not bounded: output length ${(result as any).length} > input length ${(inp as any).length}`);
    }
  }
  // Non-negativity: f(x) >= 0
  if (properties.includes("nonneg") && typeof result === "number" && violates("nonneg", result < 0)) {
    throw new _PropertyFailure(`Non-negative violation: ${result} < 0`);
  }
  // Non-empty string: identifier/display helpers should not emit blanks.
  if (properties.includes("nonempty_string") && typeof result === "string" && violates("nonempty_string", result.trim().length === 0)) {
    throw new _PropertyFailure(`Blank string output: ${JSON.stringify(result)}`);
  }
  if (properties.includes("sorted") && _isPrimitiveSortableArray(result)) {
    if (violates("sorted", result.some((value, index) => index > 0 && result[index - 1] > value))) {
      throw new _PropertyFailure(`Not sorted: ${JSON.stringify(result)}`);
    }
  }
  if (properties.includes("permutation") && args.length >= 1 && Array.isArray(args[0]) && Array.isArray(result)) {
    if (violates("permutation", !_samePrimitiveMultiset(args[0] as unknown[], result))) {
      throw new _PropertyFailure(`Permutation violated: ${JSON.stringify(result)} vs ${JSON.stringify(args[0])}`);
    }
  }
  if (properties.includes("clamped") && args.length >= 3 && typeof args[0] === "number" && typeof args[1] === "number" && typeof args[2] === "number" && typeof result === "number") {
    const lo = Math.min(args[1], args[2]);
    const hi = Math.max(args[1], args[2]);
    if (violates("clamped:bounds", result < lo || result > hi)) {
      throw new _PropertyFailure(`Clamp bounds violated: ${JSON.stringify(result)} not in [${JSON.stringify(lo)}, ${JSON.stringify(hi)}]`);
    }
    if (args[0] >= lo && args[0] <= hi && violates("clamped:passthrough", result !== args[0])) {
      throw new _PropertyFailure(`Clamp passthrough violated: ${JSON.stringify(result)} != ${JSON.stringify(args[0])}`);
    }
  }
  // Serialized/canonical string helpers should not emit nullish sentinel
  // text when the input structure contains null or undefined values.
  if (properties.includes("no_nullish_string") && typeof result === "string") {
    const firstArg = args[0];
    if (_containsNullish(firstArg) && violates("no_nullish_string", _stringLeaksNullish(result))) {
      throw new _PropertyFailure(`Nullish string leak: ${JSON.stringify(result)}`);
    }
  }
  // Symmetry: f(a,b) == f(b,a)
  if (properties.includes("symmetric") && args.length === 2) {
    const resultRev = fn([args[1], args[0]]);
    if (violates("symmetric", !_nanSafeEq(result, resultRev))) {
      throw new _PropertyFailure(`Not symmetric: f(a,b)=${JSON.stringify(result)} != f(b,a)=${JSON.stringify(resultRev)}`);
    }
  }
  // Comparator contract: compare(a,a) == 0 and sign(compare(a,b)) == -sign(compare(b,a))
  if (properties.includes("comparator") && args.length === 2) {
    const selfCmp = fn([args[0], args[0]]);
    if (violates("comparator:self", _cmpSign(selfCmp) !== 0)) {
      throw new _PropertyFailure(`Comparator self-compare should be zero: ${JSON.stringify(selfCmp)}`);
    }
    const resultRev = fn([args[1], args[0]]);
    if (violates("comparator:reverse", _cmpSign(result) !== -_cmpSign(resultRev))) {
      throw new _PropertyFailure(`Comparator antisymmetry violated: ${JSON.stringify(result)} vs ${JSON.stringify(resultRev)}`);
    }
  }
}
function _propertyReplaySnippet(name: string, args: unknown[], expectedType: string | null, properties: string[], failureIdentity: string | null, severity: string, oracleKind: string, category: string): string {
  if (failureIdentity === null) return 'throw new Error("Court Jester cannot replay this runtime-only thrown value");';
  // Persist the actual evaluator and its pure dependencies. Replay neither
  // regenerates inputs nor guesses an oracle from a diagnostic message.
  const helpers = [_PropertyFailure, _cloneSeedFallback, _cloneSeed, _nanSafeEq, _containsNullish,
    _stringLeaksNullish, _cmpSign, _isPrimitiveSortableArray, _samePrimitiveMultiset,
    _declaredPropertyForFailure, _failureIdentity, _evaluateProperties]
    .map((helper) => helper.toString()).join("\n");
  return `${helpers}\nconst _args = ${_reproExpression(args)};\nlet _reproduced = false, _checkPassed = false;\nlet _checking = false;\ntry {\n  const _invoke = (args: unknown[]) => (${name} as Function)(...args);\n  const _result = _invoke(_cloneSeed(_args));\n  _checking = true;\n  _evaluateProperties(_invoke, _args, _result, ${JSON.stringify(expectedType)}, ${JSON.stringify(properties)});\n  _checkPassed = true;\n} catch (_error) { _reproduced = _checking && (${severity !== "property_violation"} || _error instanceof _PropertyFailure) && _failureIdentity(_error) === ${JSON.stringify(failureIdentity)}; }\nconsole.log("__COURT_JESTER_REPLAY_JSON__");\nconsole.log(JSON.stringify({ reproduced: _reproduced, check_passed: _checkPassed, severity: ${JSON.stringify(severity)}, oracle_kind: ${JSON.stringify(oracleKind)}, category: ${JSON.stringify(category)} }));`;
}
function _fuzzOne(
  name: string,
  iters: number,
  genArgs: () => unknown[],
  fn: (args: unknown[]) => unknown,
  expectedType: string | null,
  paramTypes: string[] = [],
  properties: string[] = [],
  seedRows: _FuzzInput[] = [],
  defaultOmissionRows: unknown[][] = [],
  declaredProperties: string[] = [],
  sourceLine = 0,
  rejectionDomains: Array<[number, unknown[]]> = [],
): boolean {
  let pass = 0, reject = 0, crash = 0, unclassified = 0;
  let firstCrash = "";
  const allInputs: _FuzzInput[] = [];
  const corpus: unknown[][] = [];
  const behaviorSignatures = new Set<string>();
  for (const seed of seedRows) {
    allInputs.push(seed);
  }
  for (const omission of defaultOmissionRows) {
    allInputs.push({ args: omission, contractValid: false });
  }
  const edgePools: unknown[][] = [];
  for (let pi = 0; pi < paramTypes.length; pi++) {
    const template = genArgs()[pi];
    const edges = _edgeCasesFor(paramTypes[pi], template);
    edgePools.push(edges);
    for (const ev of edges) {
      const row = genArgs();
      row[pi] = _cloneSeed(ev);
      allInputs.push({ args: row, contractValid: false });
    }
  }
  if (seedRows.length === 0) {
    let pairwiseEdgeRows = 0;
    pairwiseEdges: for (let left = 0; left < edgePools.length; left++) {
      for (let right = left + 1; right < edgePools.length; right++) {
        for (const leftEdge of edgePools[left].slice(0, 8)) {
          for (const rightEdge of edgePools[right].slice(0, 8)) {
            const row = genArgs();
            row[left] = _cloneSeed(leftEdge);
            row[right] = _cloneSeed(rightEdge);
            allInputs.push({ args: row, contractValid: false });
            pairwiseEdgeRows++;
            if (pairwiseEdgeRows >= 128) break pairwiseEdges;
          }
        }
      }
    }
  }
  for (let i = 0; i < iters; i++) {
    allInputs.push(seedRows.length > 0 ? _fuzzSeedRow(seedRows) : { args: genArgs(), contractValid: false });
  }
  const maxCampaignInputs = allInputs.length + iters;
  for (let i = 0; i < allInputs.length; i++) {
    const { args, contractValid } = allInputs[i];
    let contractTargetException = false;
    let checkingProperties = false;
    try {
      _targetEntered(`${name}:${sourceLine}`, i);
      let result: unknown;
      try {
        result = fn(_cloneSeed(args));
      } catch (error) {
        contractTargetException = contractValid;
        throw error;
      }
      checkingProperties = true;
      _evaluateProperties(fn, args, result, expectedType, properties, (oracleId, passed) => {
        _cjEvent("oracle_evaluated", { surface_id: `${name}:${sourceLine}`, iteration: i, oracle_id: oracleId, passed });
      });
      if (_retainCorpusInput(corpus, behaviorSignatures, _behaviorSignature("passed", result), args)
          && allInputs.length < maxCampaignInputs) {
        allInputs.push({ args: _mutateCorpusRow(args), contractValid: false });
      }
      _cjUnitCompleted(`${name}:${sourceLine}`, i, "passed");
      pass++;
    } catch (e: unknown) {
      const outsideContract = rejectionDomains.some(([index, values]) => index < args.length && !values.some((value) => _sameInput(args[index], value)));
      const targetException = !outsideContract && (_isCrash(e) || contractTargetException);
      if (_retainCorpusInput(corpus, behaviorSignatures, _behaviorSignature(targetException ? "crash" : "rejected", e), args)
          && allInputs.length < maxCampaignInputs) {
        allInputs.push({ args: _mutateCorpusRow(args), contractValid: false });
      }
      if (targetException) {
        crash++;
        _cjUnitCompleted(`${name}:${sourceLine}`, i, "target_exception");
        const propertyFailure = e instanceof _PropertyFailure;
        const failedProperty = _declaredPropertyForFailure(e);
        const declared = propertyFailure && failedProperty !== null && declaredProperties.includes(failedProperty);
        const oracleKind = declared ? "declared_property" : (propertyFailure ? "generic_property" : "runtime_contract");
        const provenance = declared ? "source_directive" : (propertyFailure ? "language_runtime" : "observed_call");
        const confidence = declared ? "authoritative" : (propertyFailure ? "medium" : "high");
        const severity = propertyFailure ? "property_violation" : "crash";
        const failureIdentity = _failureIdentity(e);
        const minimized = _minimizeFailure(args, (candidate) => {
          // Shrinking cannot carry the original input's admission proof to a
          // different value. Restrict contract exceptions to admitted seed rows.
          if (contractTargetException && !seedRows.some((seed) => seed.contractValid && _sameInput(candidate, seed.args))) return false;
          let candidateChecking = false;
          try {
            const candidateResult = fn(_cloneSeed(candidate));
            candidateChecking = true;
            if (checkingProperties) _evaluateProperties(fn, candidate, candidateResult, expectedType, properties);
            return false;
          } catch (candidateError) {
            return failureIdentity !== null && (!checkingProperties || candidateChecking) && (contractTargetException || _isCrash(candidateError)) && _failureIdentity(candidateError) === failureIdentity;
          }
        });
        const replayArgs = minimized[0] === "preserved" ? minimized[2] : args;
        const category = propertyFailure ? "property" : "exception";
        const replaySnippet = checkingProperties ? _propertyReplaySnippet(name, replayArgs, expectedType, properties, failureIdentity, severity, oracleKind, category) : null;
        _emitFinding(name, args, e, severity, oracleKind, provenance, confidence, category, minimized, "direct", null, sourceLine, replaySnippet);
        if (crash === 1) firstCrash = `  CRASH ${name}(${_shortJson(args)}): ${_clipText(e)}`;
      } else if (outsideContract) {
        reject++;
        _cjUnitCompleted(`${name}:${sourceLine}`, i, "rejected");
      } else {
        unclassified++;
        _cjUnitCompleted(`${name}:${sourceLine}`, i, "unclassified_exception");
        const replaySnippet = checkingProperties ? _propertyReplaySnippet(name, args, expectedType, properties, _failureIdentity(e), "crash", "runtime_contract", "exception") : null;
        _emitFinding(name, args, e, "crash", "runtime_contract", "observed_call", "low", "exception", null, "direct", null, sourceLine, replaySnippet, "unknown");
      }
    }
  }
  _cjCorpora.set(`${name}:${sourceLine}`, corpus.slice(0, 64));
  const total = pass + reject + crash + unclassified;
  if (crash > 0) {
    console.log(`FUZZ ${name}: ${pass} passed, ${reject} rejected, ${crash} CRASHED (of ${total})`);
    console.log(firstCrash);
    _fuzzTotalFailures++;
    return false;
  } else if (unclassified > 0) {
    console.log(`FUZZ ${name}: ${pass} passed, ${reject} rejected, 0 CRASHED, ${unclassified} unclassified (of ${total})`);
    _fuzzTotalFailures++;
    return false;
  } else if (pass === 0) {
    console.log(`FUZZ ${name}: all ${total} inputs rejected (nothing tested)`);
    _fuzzTotalFailures++;
    return false;
  } else {
    console.log(`FUZZ ${name}: ${pass} passed, ${reject} rejected (of ${total})`);
    return true;
}
}
