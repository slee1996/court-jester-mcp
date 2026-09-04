
const _cjCorpusPayload: Record<string, unknown[][]> = {};
for (const [surfaceId, rows] of _cjCorpora.entries()) {
  const serializable: unknown[][] = [];
  for (const row of rows) {
    try {
      const encoded = row.map((value) => _reproJsonValue(value));
      JSON.stringify(encoded);
      serializable.push(encoded);
    } catch {
      // Runtime-only values are useful during this campaign but cannot persist.
    }
  }
  if (serializable.length > 0) _cjCorpusPayload[surfaceId] = serializable.slice(0, 64);
}
console.log("__COURT_JESTER_CORPUS_JSON__" + JSON.stringify(_cjCorpusPayload));
_cjEvent("harness_completed", { completed_units: _cjCompletedUnits });
if (_fuzzResults.length > 0) {
  console.log("__COURT_JESTER_FINDINGS_JSON__");
  console.log(JSON.stringify(_fuzzResults));
}
if (_fuzzResults.length > 0) {
  console.error(`Fuzz testing failed: ${_fuzzTotalFailures} function(s) had failures`);
  process.exit(1);
} else if (_fuzzTotalFailures > 0) {
  console.log(`Fuzz campaign completed with ${_fuzzTotalFailures} all-rejected function(s)`);
  process.exit(0);
} else {
  console.log("All fuzz tests passed");
  process.exit(0);
}
