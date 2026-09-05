
import random as _rng
import json as _json
import copy as _copy
import unicodedata as _unicodedata
import atexit as _atexit
from urllib.parse import parse_qsl as _parse_qsl
import sys as _sys
_rng.seed(42)
_fuzz_failures = 0
_FUZZ_RESULTS = []
_CJ_SEQUENCE = 0
_CJ_COMPLETED_UNITS = 0
_CJ_ACTIVE_UNITS = set()
_CJ_HARNESS_COMPLETED = False
def _cj_event(event, data=None):
    global _CJ_SEQUENCE
    payload = {"protocol_version": 2, "sequence": _CJ_SEQUENCE, "event": event}
    if data is not None:
        payload["data"] = data
    print("__COURT_JESTER_EVENT_JSON__" + _json.dumps(payload, ensure_ascii=False), flush=True)
    _CJ_SEQUENCE += 1
def _cj_bootstrap():
    _cj_event("bootstrap_started")
    _cj_event("target_resolved", {"module": "generated"})
    _cj_event("target_ready")
_cj_bootstrap()
def _cj_unit_started(surface_id, iteration, input_origin="generated"):
    _CJ_ACTIVE_UNITS.add((str(surface_id), int(iteration)))
    _cj_event("unit_started", {
        "surface_id": str(surface_id),
        "iteration": int(iteration),
        "input_classification": "valid",
        "input_origin": str(input_origin),
    })
def _cj_unit_completed(surface_id, iteration, outcome):
    global _CJ_COMPLETED_UNITS
    _cj_event("unit_completed", {
        "surface_id": str(surface_id),
        "iteration": int(iteration),
        "outcome": outcome,
    })
    _CJ_ACTIVE_UNITS.discard((str(surface_id), int(iteration)))
    _CJ_COMPLETED_UNITS += 1
def _cj_checked(oracle_id, condition):
    passed = bool(condition)
    if len(_CJ_ACTIVE_UNITS) == 1:
        surface_id, iteration = next(iter(_CJ_ACTIVE_UNITS))
        _cj_event("oracle_evaluated", {"surface_id": surface_id, "iteration": iteration,
                                      "oracle_id": oracle_id, "passed": passed})
    return passed
class _PropertyFailure(AssertionError):
    def __init__(self, oracle_id, message):
        super().__init__(message)
        self.oracle_id = oracle_id
def _cj_require(oracle_id, condition, message):
    if not _cj_checked(oracle_id, condition):
        raise _PropertyFailure(oracle_id, message())
def _replace_args(args, replacements):
    result = list(args)
    for index, value in replacements.items():
        result[index] = value
    return result
def _cj_complete_harness():
    global _CJ_HARNESS_COMPLETED
    if _CJ_HARNESS_COMPLETED:
        return
    # Interpreter shutdown does not prove the outcome of an interrupted call.
    # Leave its unit open so the reducer retains partial execution evidence.
    if _CJ_ACTIVE_UNITS:
        return
    _CJ_HARNESS_COMPLETED = True
    _cj_event("harness_completed", {"completed_units": _CJ_COMPLETED_UNITS})
_atexit.register(_cj_complete_harness)
def _target_entered(surface_id, iteration=None):
    if iteration is not None:
        input_origin = (
            "safe_dependency_substitute"
            if str(surface_id) in _CJ_SAFE_DEPENDENCY_SURFACES
            else "generated"
        )
        _cj_unit_started(surface_id, iteration, input_origin)
    print(_json.dumps({"event": "target_entered", "surface_id": str(surface_id)}), file=_sys.stderr, flush=True)
_FINDING_ORDINALS = {}
def _sanitize_symbol(value):
    return "".join(ch if (ch.isalnum() or ch in "._-") else "_" for ch in str(value))
def _finding_id(function):
    symbol = _sanitize_symbol(function)
    ordinal = _FINDING_ORDINALS.get(symbol, 0) + 1
    _FINDING_ORDINALS[symbol] = ordinal
    return f"fuzz:{symbol}:{ordinal}"
def _json_value(value):
    try:
        _json.dumps(value, ensure_ascii=False, allow_nan=False).encode("utf-8")
        return value
    except Exception:
        return None
def _repro_expression(value, ancestors=None):
    if type(value) in (type(None), bool, int, str, bytes):
        return repr(value)
    if type(value) is float:
        import math
        if math.isnan(value): return "float('nan')"
        if math.isinf(value): return "float('inf')" if value > 0 else "float('-inf')"
        return repr(value)
    if type(value) not in (list, tuple, dict, set, frozenset):
        raise ValueError("runtime-only repro value")
    ancestors = set() if ancestors is None else ancestors
    if id(value) in ancestors or len(ancestors) >= 64:
        raise ValueError("cyclic or deeply nested repro value")
    ancestors.add(id(value))
    try:
        if type(value) is dict:
            return "{" + ", ".join(_repro_expression(key, ancestors) + ": " + _repro_expression(item, ancestors) for key, item in value.items()) + "}"
        items = ", ".join(_repro_expression(item, ancestors) for item in value)
        if type(value) is list: return "[" + items + "]"
        if type(value) is tuple: return "(" + items + ("," if len(value) == 1 else "") + ")"
        if type(value) is frozenset: return "frozenset([" + items + "])"
        return "{" + items + "}" if value else "set()"
    finally:
        ancestors.remove(id(value))
def _repro_value_expression(value):
    try: return _repro_expression(value)
    except ValueError: return repr(value)
def _repro_case(args, input_text=None):
    values = list(args) if isinstance(args, (list, tuple)) else [args]
    return {"arguments": [{"expression": _repro_value_expression(value), "json_value": _json_value(value)} for value in values], "input_text": input_text}
def _shrink_candidates(value):
    seen = set()
    def add(candidate):
        try: key = _json.dumps(candidate, sort_keys=True, default=repr, ensure_ascii=False)
        except Exception: key = repr(candidate)
        if key not in seen:
            seen.add(key); yield candidate
    if isinstance(value, str):
        for candidate in ("", value[:1], value.strip(), "\u00a0" if any(char == "\u00a0" for char in value) else value.strip()): yield from add(candidate)
        step = len(value) // 2
        while step > 0:
            for start in range(0, len(value), step): yield from add(value[:start] + value[start + step:])
            step //= 2
    elif isinstance(value, bool):
        yield from add(False)
    elif isinstance(value, int) and not isinstance(value, bool):
        for candidate in (0, 1, -1): yield from add(candidate)
        current = value
        while current:
            current = int(current / 2); yield from add(current)
    elif isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")): yield from add(value)
        else:
            for candidate in (0.0, 1.0, -1.0): yield from add(candidate)
            current = value
            while current and abs(current) > 1e-15:
                current /= 2; yield from add(current)
    elif isinstance(value, (list, tuple)):
        for index, item in enumerate(value):
            for shrunk in _shrink_candidates(item):
                candidate = list(value)
                candidate[index] = shrunk
                yield from add(candidate if isinstance(value, list) else tuple(candidate))
    elif isinstance(value, dict):
        for key, item in value.items():
            for shrunk in _shrink_candidates(item):
                candidate = dict(value)
                candidate[key] = shrunk
                yield from add(candidate)
def _shrink_rank(value):
    try:
        rendered = _json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False, default=repr)
    except Exception:
        rendered = repr(value)
    return (len(rendered), rendered)
def _minimize_failure(original, reproduce, severity, oracle_id):
    import time as _time
    current = _copy.deepcopy(original); attempts = 0; deadline = _time.monotonic() + 0.250
    while attempts < 100 and _time.monotonic() < deadline:
        current_rank = _shrink_rank(current)
        improved = False
        for candidate in _shrink_candidates(current):
            if attempts >= 100 or _time.monotonic() >= deadline: break
            if _shrink_rank(candidate) >= current_rank: continue
            attempts += 1
            try:
                if reproduce(candidate):
                    current = _copy.deepcopy(candidate)
                    improved = True
                    break
            except Exception:
                pass
        if not improved:
            break
    if isinstance(current, list):
        nb_space_candidate = ["\u00a0" if isinstance(value, str) and "\u00a0" in value else value for value in current]
        if _shrink_rank(nb_space_candidate) <= _shrink_rank(current) and nb_space_candidate != current:
            try:
                if reproduce(nb_space_candidate):
                    current = nb_space_candidate
            except Exception:
                pass
    try: preserved = bool(reproduce(current))
    except Exception: preserved = False
    return ("preserved" if preserved else "failed", attempts, current if preserved else original)
def _replay_snippet(function, args, severity, oracle_kind, category, error_type):
    try:
        rendered = ", ".join(_repro_expression(value) for value in (args if isinstance(args, (list, tuple)) else [args]))
    except ValueError:
        return "raise RuntimeError('Court Jester cannot replay this runtime-only input')"
    payload = {"severity": severity, "oracle_kind": oracle_kind, "category": category}
    return ("import json as _replay_json\n_reproduced = False\ntry:\n"
            + f"    {function}({rendered})\n"
            + "except Exception as _replay_error:\n"
            + f"    _reproduced = type(_replay_error).__name__ == {error_type!r}\n"
            + "print('__COURT_JESTER_REPLAY_JSON__')\n"
            + f"print(_replay_json.dumps(dict({payload!r}, reproduced=_reproduced), ensure_ascii=False))")
def _invocation_replay_snippet(source, args, error, severity, oracle_kind, category, evaluate):
    try: arguments = _repro_expression(args)
    except ValueError: return "raise RuntimeError('Court Jester cannot replay this runtime-only input')"
    import inspect as _inspect
    helpers = (_PropertyFailure, _python_failure_identity, _cj_checked, _cj_require, _replace_args, _materialize_if_iterator,
               _nan_eq, _callable_consistency_key, _consistency_eq, _contains_nullish,
               _string_leaks_nullish, _cmp_sign, _multiset_counts, _is_palindrome_sequence)
    definitions = "\n".join(_inspect.getsource(helper) for helper in helpers)
    payload = {"severity": severity, "oracle_kind": oracle_kind, "category": category}
    match = f"_python_failure_identity(_error) == {_python_failure_identity(error)!r}"
    return ("import copy as _copy\nimport json as _json\n_CJ_ACTIVE_UNITS = set()\n"
            + definitions + "\n" + source + f"\n_args = {arguments}\n_reproduced = False\n_checking = False\ntry:\n"
            + "    _result = _cj_invoke(_args)\n"
            + ("    _checking = True\n    _cj_evaluate(_args, _result)\n" if evaluate else "")
            + "except Exception as _error:\n"
            + f"    _reproduced = {evaluate!r} == _checking and ({match})\n"
            + "print('__COURT_JESTER_REPLAY_JSON__')\n"
            + f"print(_json.dumps(dict({payload!r}, reproduced=_reproduced), ensure_ascii=False))")

def _resolve_factory_action(result, action, single):
    if callable(result) and single: return result
    if isinstance(result, dict): return result.get(action)
    return getattr(result, action, None)

def _factory_replay_snippet(factory, setup, trace, single, phase, error):
    if phase.startswith("arguments"):
        return "raise RuntimeError('Court Jester cannot replay factory input-construction failures')"
    try:
        inputs = _repro_expression((setup, trace))
    except ValueError:
        return "raise RuntimeError('Court Jester cannot replay this runtime-only factory input')"
    import inspect as _inspect
    resolver = _inspect.getsource(_resolve_factory_action)
    identity = (type(error).__name__, str(error))
    return ("import copy as _copy\nimport json as _json\n" + resolver
            + f"\n_setup, _trace = {inputs}\n_phase = 'factory'\n_reproduced = False\ntry:\n"
            + f"    _result = {factory}(*_copy.deepcopy(_setup['args']), **_copy.deepcopy(_setup['kwargs']))\n"
            + "    for _index, _entry in enumerate(_trace):\n"
            + "        _phase = 'resolve:' + str(_index)\n"
            + f"        _candidate = _resolve_factory_action(_result, _entry['action'], {single!r})\n"
            + "        if callable(_candidate) != _entry.get('callable'): break\n"
            + "        if not callable(_candidate): continue\n"
            + "        _phase = 'action:' + str(_index)\n"
            + "        _candidate(*_copy.deepcopy(_entry['args']), **_copy.deepcopy(_entry['kwargs']))\n"
            + "except Exception as _error:\n"
            + f"    _reproduced = _phase == {phase!r} and (type(_error).__name__, str(_error)) == {identity!r}\n"
            + "print('__COURT_JESTER_REPLAY_JSON__')\n"
            + "print(_json.dumps(dict(reproduced=_reproduced, severity='crash', oracle_kind='runtime_contract', category='exception')))\n")

def _emit_finding(function, args, error, severity="crash", oracle_kind="runtime_contract", oracle_provenance="language_runtime", confidence="high", category="exception", expected=None, actual=None, input_classification="valid", case_label=None, minimize=None, invocation_path="direct", replay_snippet=None, repro_kind="function_call"):
    oracle_id = f"{oracle_kind}:{_sanitize_symbol(function)}"
    status, attempts, minimized = ("not_needed", 0, args) if minimize is None else minimize
    original_case = _repro_case(args, case_label)
    minimized_case = None if status in ("not_needed", "failed") else _repro_case(minimized, case_label)
    repro_args = minimized if minimized_case is not None else args
    expectation = {"severity": severity, "oracle_kind": oracle_kind, "category": category}
    repro = {"kind": repro_kind, "function": str(function), "arguments": original_case["arguments"], "input_text": original_case["input_text"], "case_label": case_label, "snippet": replay_snippet if replay_snippet is not None else _replay_snippet(function, repro_args, severity, oracle_kind, category, type(error).__name__), "command": None, "expectation": expectation}
    record = {"id": _finding_id(function), "severity": severity, "confidence": confidence, "category": category, "location": {"source_file": "", "function": str(function), "line": 0, "invocation_path": invocation_path}, "oracle": {"id": oracle_id, "kind": oracle_kind, "provenance": oracle_provenance, "confidence": confidence, "expected": expected, "actual": actual if actual is not None else _clip_text(error)}, "input_classification": input_classification, "repro": repro, "minimization": {"status": status, "attempts": attempts, "original": original_case, "minimized": minimized_case}, "error_type": type(error).__name__, "message": _clip_text(error), "suppressed": False}
    _FUZZ_RESULTS.append(record)
    _cj_event("finding", {"finding": record})

def _semantic_check(name, target, args, expected, projection, label):
    """Own one immutable semantic case from invocation through persisted replay."""
    projections = {
        "identity": (lambda value: value, "_value"),
        "sign": (lambda value: 1 if value > 0 else (-1 if value < 0 else 0), "(1 if _value > 0 else (-1 if _value < 0 else 0))"),
        "bool": (bool, "bool(_value)"),
        "list": (list, "list(_value)"),
        "query_pairs": (lambda value: _parse_qsl(value, keep_blank_values=True), "_parse_qsl(_value, keep_blank_values=True)"),
    }
    project, expression = projections[projection]
    original = _copy.deepcopy(args)
    actual = None
    target_error = None
    try:
        _value = target(*_copy.deepcopy(original))
        actual = project(_value)
    except Exception as error:
        target_error = error
    if target_error is None and actual == expected:
        return 0
    error = target_error or AssertionError(f"{label}: {actual!r} != {expected!r}")
    payload = {"severity": "property_violation", "oracle_kind": "inferred_semantic", "category": "property"}
    # Re-execute the same observation, not merely a call that is expected to
    # throw. Literal case data is complete; display truncation is never code.
    snippet = ("import json as _replay_json\nfrom urllib.parse import parse_qsl as _parse_qsl\n"
               + "_reproduced = False\ntry:\n"
               + f"    _value = {name}(*{original!r})\n"
               + f"    _observed = {expression}\n"
               + (f"    _reproduced = _observed != {expected!r}\n" if target_error is None else "")
               + "except Exception as _error:\n"
               + (f"    _reproduced = type(_error).__name__ == {type(target_error).__name__!r}\n" if target_error is not None else "    pass\n")
               + "print('__COURT_JESTER_REPLAY_JSON__')\n"
               + f"print(_replay_json.dumps(dict({payload!r}, reproduced=_reproduced)))")
    _emit_finding(name, original, error, "property_violation", "inferred_semantic", "name_heuristic", "low", "property",
                  expected=_json.dumps(expected, ensure_ascii=False), actual=_clip_text(repr(actual)) if target_error is None else _clip_text(target_error),
                  case_label=label, replay_snippet=snippet)
    return 1
def _observe_roundtrip(encode, decode, value):
    phase = "copy_input"
    try:
        argument = _copy.deepcopy(value)
        phase = "encode"
        encoded = encode(argument)
        phase = "copy_encoded"
        argument = _copy.deepcopy(encoded)
        phase = "decode"
        decoded = decode(argument)
        phase = "compare"
        return bool(_nan_eq(value, decoded)), None, phase, decoded
    except Exception as error:
        return False, error, phase, None

def _roundtrip_case(name, source, value):
    encode, decode = eval(source)
    original = _copy.deepcopy(value)
    matched, error, phase, actual = _observe_roundtrip(encode, decode, original)
    if matched: return False
    import inspect as _inspect
    definitions = "\n".join(_inspect.getsource(helper) for helper in (_materialize_if_iterator, _nan_eq, _observe_roundtrip))
    try:
        argument = _repro_expression(original)
        match = "_error is None and not _matched" if error is None else f"_error is not None and _phase == {phase!r} and (type(_error).__name__, str(_error)) == {(type(error).__name__, str(error))!r}"
        body = ("import copy as _copy\nimport json as _json\n" + definitions
                + f"\n_matched, _error, _phase, _actual = _observe_roundtrip(_encode, _decode, {argument})\n"
                + "print('__COURT_JESTER_REPLAY_JSON__')\n"
                + f"print(_json.dumps(dict(reproduced=bool({match}), severity='property_violation', oracle_kind='inferred_semantic', category='property')))\n")
        snippet = "def _cj_roundtrip_replay(_encode, _decode):\n" + "\n".join("    " + line for line in body.splitlines()) + f"\n_cj_roundtrip_replay(*{source})\n"
    except ValueError:
        snippet = "raise RuntimeError('Court Jester cannot replay this runtime-only roundtrip input')"
    failure = error if error is not None else AssertionError(f"Roundtrip failed: {original!r} -> {actual!r}")
    _emit_finding(name, [original], failure, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", input_classification="unknown" if error is not None else "valid", case_label="roundtrip", replay_snippet=snippet)
    print(f"  ROUNDTRIP FAIL {name}: {_clip_text(failure)}")
    return True

def _is_generated_collaborator_mismatch(error):
    if not isinstance(error, AttributeError):
        return False
    message = str(error)
    return "object has no attribute 'execute'" in message or 'object has no attribute "execute"' in message

def _emit_uncertain_exception(function, args, error, case_label=None, invocation_path="direct", replay_source=None, evaluate=False, replay_snippet=None, repro_kind="function_call"):
    snippet = replay_snippet if replay_snippet is not None else (_invocation_replay_snippet(replay_source, args, error, "crash", "runtime_contract", "exception", evaluate) if replay_source is not None else None)
    _emit_finding(function, args, error, "crash", "runtime_contract", "observed_call", "low", "exception",
                  input_classification="unknown", case_label=case_label, invocation_path=invocation_path, replay_snippet=snippet, repro_kind=repro_kind)

def _outside_closed_domain(args, domains):
    return any(index < len(args) and not any(_same_input(args[index], value) for value in values)
               for index, values in domains)

def _emit_error(function, args, error, properties=(), reproduce=None, case_label=None, invocation_path="direct", target_exception=False, replay_source=None, evaluate=False):
    if _is_generated_collaborator_mismatch(error):
        return

    is_property = isinstance(error, _PropertyFailure)
    declared = is_property and (error.oracle_id in properties or (error.oracle_id == "comparator" and "antisymmetric" in properties))
    kind = "declared_property" if is_property and declared else ("generic_property" if is_property else "runtime_contract")
    provenance = "source_directive" if kind == "declared_property" else "language_runtime"
    confidence = "authoritative" if kind == "declared_property" else ("medium" if is_property else "high")
    category = "property" if is_property else "exception"
    severity = "property_violation" if is_property else "crash"
    minimized = _minimize_failure(args, reproduce, severity, f"{kind}:{function}") if reproduce is not None else None
    replay_args = minimized[2] if minimized is not None and minimized[0] == "preserved" else args
    snippet = _invocation_replay_snippet(replay_source, replay_args, error, severity, kind, category, evaluate) if replay_source is not None else None
    _emit_finding(function, args, error, severity, kind, provenance, confidence, category,
                  actual=_clip_text(error), case_label=case_label, minimize=minimized,
                  invocation_path=invocation_path, replay_snippet=snippet)
def _python_failure_identity(error):
    prefix = str(error).split(":", 1)[0]
    if isinstance(error, _PropertyFailure):
        return ("property", error.oracle_id, prefix)
    return ("exception", type(error).__name__, prefix)
def _reproduces_python(candidate, original, invoke):
    try:
        invoke()
    except Exception as error:
        return type(error) is type(original) and _python_failure_identity(error) == _python_failure_identity(original)
    return False

# Crash detection: these exception types indicate real bugs, not validation.
_CRASH_TYPES = (TypeError, AttributeError, KeyError, IndexError, RecursionError, MemoryError, ZeroDivisionError, UnicodeError)

_FUZZ_TEXT_LIMIT = 240
def _clip_text(value, limit=_FUZZ_TEXT_LIMIT):
    # Python strings can contain lone surrogates, which neither UTF-8 nor the
    # report's JSON string type can represent. Escape display text; the replay
    # expression retains the original Python value independently.
    text = str(value).encode("utf-8", errors="backslashreplace").decode("utf-8")
    if len(text) <= limit:
        return text
    return f"{text[:limit]}... [truncated {len(text) - limit} chars]"

def _short_repr(value, limit=_FUZZ_TEXT_LIMIT):
    return _clip_text(repr(value), limit)

def _materialize_if_iterator(value):
    if hasattr(value, "__next__"):
        return list(value)
    return value

def _is_crash(e):
    """Distinguish intentional validation errors from real bugs."""
    if isinstance(e, _CRASH_TYPES):
        return True
    if isinstance(e, _PropertyFailure):
        return True  # property violation (type check, idempotency, consistency)
    return False

def _fuzz_int(): return _rng.randint(-1000, 1000)
def _fuzz_int_range(lo, hi): return _rng.randint(lo, hi)
def _fuzz_float(): return _rng.uniform(-1000.0, 1000.0)
def _fuzz_bool(): return _rng.choice([True, False])
def _fuzz_none(): return None
def _fuzz_bytes(): return bytes(_rng.randint(0, 255) for _ in range(_rng.randint(0, 20)))
def _fuzz_str():
    length = _rng.randint(0, 50)
    pools = [
        "",
        "".join(chr(_rng.randint(32, 126)) for _ in range(length)),
        "".join(chr(_rng.randint(0, 0xFFFF)) for _ in range(length)),
        "   \t\n  ",
        "\xa0" * length,
        "hello world",
        "café résumé naïve",
        "a" * 200,
        " leading",
        "trailing ",
        "  both  ",
        "UPPER",
        "lower",
        "MiXeD cAsE",
        "with\nnewlines\n",
        "with\ttabs",
        "special!@#$%^&*()",
        "12345",
        "-1.5",
    ]
    return _rng.choice(pools)
def _fuzz_any():
    return _rng.choice([_fuzz_int(), _fuzz_float(), _fuzz_str(), _fuzz_bool(), None, [], _fuzz_dict()])

def _fuzz_dict():
    # Open mappings have no repository shape; concrete objects are generated
    # from DomainNode::Object by the planner.
    return {}

def _fuzz_like_seed(value):
    if isinstance(value, bool):
        return _rng.choice([value, not value])
    if isinstance(value, int) and not isinstance(value, bool):
        return _rng.choice([value, value - 1, value + 1, 0, -1])
    if isinstance(value, float):
        return _rng.choice([value, value - 1.0, value + 1.0, 0.0, -1.0])
    if isinstance(value, str):
        return _rng.choice([value, value.strip(), value.upper(), value.lower(), value[:max(0, len(value) // 2)]])
    if value is None:
        return None
    if isinstance(value, list):
        return [_fuzz_like_seed(item) for item in value]
    if isinstance(value, tuple):
        return tuple(_fuzz_like_seed(item) for item in value)
    if isinstance(value, dict):
        return {key: _fuzz_like_seed(item) for key, item in value.items()}
    return _copy.deepcopy(value)

def _fuzz_seed_row(seed_rows):
    row = _copy.deepcopy(_rng.choice(seed_rows))
    return row if _rng.random() < 0.65 else [_fuzz_like_seed(item) for item in row]

def _same_input(left, right, depth=0):
    """Admission proofs require exact data, not permissive output consistency."""
    if depth > 32 or type(left) is not type(right):
        return False
    if left is None or type(left) in (bool, int, str, bytes):
        return left == right
    if type(left) is float:
        return left.hex() == right.hex()
    if type(left) in (list, tuple):
        return len(left) == len(right) and all(_same_input(a, b, depth + 1) for a, b in zip(left, right))
    if type(left) is dict:
        return len(left) == len(right) and all(
            any(_same_input(key, other, depth + 1) and _same_input(value, right[other], depth + 1) for other in right)
            for key, value in left.items()
        )
    return False

_CJ_CORPORA = {}
def _behavior_signature(outcome, value):
    if isinstance(value, BaseException):
        return f"{outcome}:error:{type(value).__name__}:{str(value).split(':', 1)[0]}"
    if value is None:
        return f"{outcome}:none"
    if isinstance(value, (list, tuple)):
        return f"{outcome}:sequence:{min(len(value), 8)}:{','.join(type(item).__name__ for item in value[:4])}"
    if isinstance(value, dict):
        return f"{outcome}:mapping:{','.join(sorted(map(str, value.keys()))[:12])}"
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if isinstance(value, float) and value != value:
            bucket = "nan"
        elif value == 0:
            bucket = "zero"
        elif value < 0:
            bucket = "negative"
        else:
            bucket = "positive"
        return f"{outcome}:number:{bucket}"
    if isinstance(value, str):
        bucket = "empty" if not value else ("blank" if not value.strip() else str(min(len(value), 32)))
        return f"{outcome}:string:{bucket}"
    return f"{outcome}:{type(value).__name__}:{value!s}"

def _mutate_corpus_row(row):
    candidate = _copy.deepcopy(row)
    if not candidate:
        return candidate
    index = _rng.randrange(len(candidate))
    candidate[index] = _fuzz_like_seed(candidate[index])
    return candidate

def _retain_corpus_input(corpus, signatures, signature, args):
    if signature in signatures or len(corpus) >= 64:
        return False
    signatures.add(signature)
    corpus.append(_copy.deepcopy(args))
    return True
_EDGE_INTS = [0, 1, -1, 2**53, -(2**53), 2**53 + 1]
_EDGE_FLOATS = [0.0, -0.0, float('inf'), float('-inf'), float('nan'), 1e-300, 1e300]
_EDGE_STRS = ["", "\0", "\uFFFF", "\u00A0", "\u00A0\u00A0\u00A0", "a" * 10000,
              "true", "null", "0", "-1", "\r\n", "\u200F", "\u200D", "${...}", "<script>"]
_EDGE_BYTES = [b"", b"\x00", b"\xff" * 100, bytes(range(256))]
_EDGE_DICTS = [{}]

def _edge_cases_for(type_name):
    m = {"int": _EDGE_INTS, "float": _EDGE_FLOATS, "str": _EDGE_STRS, "bytes": _EDGE_BYTES, "dict": _EDGE_DICTS}
    return m.get(type_name, [])

def _nan_eq(a, b):
    a = _materialize_if_iterator(a)
    b = _materialize_if_iterator(b)
    if isinstance(a, float) and isinstance(b, float):
        import math
        if math.isnan(a) and math.isnan(b): return True
    return a == b

def _callable_consistency_key(value):
    return (
        getattr(value, "__module__", type(value).__module__),
        getattr(value, "__qualname__", getattr(value, "__name__", type(value).__qualname__)),
    )

def _consistency_eq(a, b):
    """Compare semantic values without treating fresh callable/object identity as output drift."""
    a = _materialize_if_iterator(a)
    b = _materialize_if_iterator(b)
    if type(a) is not type(b):
        return False
    if callable(a) or callable(b):
        return (
            callable(a)
            and callable(b)
            and _callable_consistency_key(a) == _callable_consistency_key(b)
        )
    if isinstance(a, (list, tuple)):
        return len(a) == len(b) and all(_consistency_eq(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(_consistency_eq(a[key], b[key]) for key in a)
    if isinstance(a, (set, frozenset)):
        if len(a) != len(b):
            return False
        unmatched = list(b)
        for item_a in a:
            for index, item_b in enumerate(unmatched):
                if _consistency_eq(item_a, item_b):
                    unmatched.pop(index)
                    break
            else:
                return False
        return True
    if getattr(type(a), "__eq__", None) is object.__eq__:
        return True
    try:
        return _nan_eq(a, b)
    except Exception:
        return True

def _contains_nullish(value):
    if value is None:
        return True
    if isinstance(value, dict):
        return any(_contains_nullish(v) for v in value.values())
    if isinstance(value, (list, tuple, set)):
        return any(_contains_nullish(v) for v in value)
    return False

def _string_leaks_nullish(value):
    if not isinstance(value, str):
        return False
    _lower = value.lower()
    return ("none" in _lower) or ("null" in _lower) or ("undefined" in _lower)

def _ascii_fold(value):
    text = value if isinstance(value, str) else str(value)
    return _unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode("ascii")

def _cmp_sign(value):
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, (int, float)):
        if value < 0:
            return -1
        if value > 0:
            return 1
        return 0
    _cj_require("comparator", False, lambda: f"Comparator returned non-numeric value: {repr(value)}")

def _multiset_counts(values):
    counts = {}
    for value in values:
        key = _json.dumps(value, sort_keys=True, ensure_ascii=False, default=repr)
        counts[key] = counts.get(key, 0) + 1
    return counts

def _is_palindrome_sequence(value):
    return isinstance(value, (list, tuple, str)) and list(value) == list(reversed(value))
