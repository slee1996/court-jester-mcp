# Runs without target code. Snapshots use the native adapter's shared codec.
def _cj_shrink(value, depth=0):
    if depth > 16:
        return
    kind = type(value)
    if kind is bool:
        if value:
            yield False
    elif kind in (int, float):
        if value != 0 or (kind is float and repr(value) == '-0.0'):
            yield kind(0)
            if abs(value) > 1 and abs(value) != float('inf'):
                yield kind(1 if value > 0 else -1)
                yield (abs(value) // 2 * (1 if value > 0 else -1)) if kind is int else value / 2
    elif kind in (str, list, bytes, bytearray):
        size = len(value)
        if size:
            yield kind()
        chunk = max(1, size // 2)
        while size and chunk:
            for start in range(0, size, chunk):
                yield value[:start] + value[start + chunk:]
            chunk //= 2
        if kind is not str:
            for index, child in enumerate(value):
                for smaller in _cj_shrink(child, depth + 1):
                    candidate = list(value)
                    candidate[index] = smaller
                    yield kind(candidate)

def _cj_candidates():
    original = [_cj_native_value(value) for value in _cj_args]
    seen = {_cj_native_json.dumps(original, sort_keys=True)}
    candidates = []
    for index, value in enumerate(_cj_args):
        for smaller in _cj_shrink(value):
            candidate = list(_cj_args)
            candidate[index] = smaller
            snapshots = [_cj_native_value(item) for item in candidate]
            key = _cj_native_json.dumps(snapshots, sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            if len(candidates) == 32:
                return dict(candidates=candidates, truncated=True)
            candidates.append(snapshots)
    return dict(candidates=candidates, truncated=False)

print('__COURT_JESTER_NATIVE_CANDIDATES__')
print(_cj_native_json.dumps(_cj_candidates()))
