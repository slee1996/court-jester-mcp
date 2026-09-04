
_cj_corpus_payload = {}
for _surface_id, _rows in _CJ_CORPORA.items():
    _serializable = []
    for _row in _rows:
        try:
            _json.dumps(_row, ensure_ascii=False, allow_nan=False)
            _serializable.append(_row)
        except Exception:
            pass
    if _serializable:
        _cj_corpus_payload[_surface_id] = _serializable[:64]
print("__COURT_JESTER_CORPUS_JSON__" + _json.dumps(_cj_corpus_payload, ensure_ascii=False, allow_nan=False))
_cj_complete_harness()
if _FUZZ_RESULTS:
    print("__COURT_JESTER_FINDINGS_JSON__")
    print(_json.dumps(_FUZZ_RESULTS, ensure_ascii=False, allow_nan=False))
if _FUZZ_RESULTS:
    raise AssertionError(f"Fuzz testing failed: {_fuzz_failures} function(s) had failures")
elif _fuzz_failures > 0:
    print(f"Fuzz campaign completed with {_fuzz_failures} all-rejected function(s)")
else:
    print(f"All fuzz tests passed")
