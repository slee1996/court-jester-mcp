use std::collections::HashMap;

use crate::types::*;

/// Number of random inputs to generate per function.
const FUZZ_ITERATIONS: usize = 30;

/// Generate a property-based fuzz harness that tests each function with
/// many random inputs and checks:
/// 1. No crashes on any valid input
/// 2. Return type matches annotation (where checkable)
/// 3. Idempotency where applicable (string→string, etc.)
/// 4. Consistency (same input → same output)
pub fn synthesize_calls(analysis: &AnalysisResult, language: &Language) -> String {
    synthesize_calls_for(
        &analysis.functions,
        &analysis.classes,
        &analysis.aliases,
        language,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractKind {
    StringTransform,
    MappingSerializer,
    Comparator,
}

#[derive(Clone, Copy, Debug)]
enum TsNamedTypeRef<'a> {
    Class(&'a ClassInfo),
    Alias(&'a TypeAliasInfo),
}

type TsNamedTypes<'a> = HashMap<&'a str, TsNamedTypeRef<'a>>;

pub fn synthesize_calls_for(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
) -> String {
    let class_defs: HashMap<&str, &ClassInfo> =
        classes.iter().map(|c| (c.name.as_str(), c)).collect();

    let pseudo_analysis = AnalysisResult {
        functions: functions.to_vec(),
        classes: classes.to_vec(),
        aliases: aliases.to_vec(),
        imports: vec![],
        complexity: 1,
        parse_error: false,
    };

    match language {
        Language::Python => synthesize_python(&pseudo_analysis, &class_defs),
        Language::TypeScript => {
            let named_types = build_ts_named_types(classes, aliases);
            synthesize_typescript(&pseudo_analysis, &named_types)
        }
    }
}

fn build_ts_named_types<'a>(
    classes: &'a [ClassInfo],
    aliases: &'a [TypeAliasInfo],
) -> TsNamedTypes<'a> {
    let mut defs = HashMap::new();
    for class in classes {
        defs.insert(class.name.as_str(), TsNamedTypeRef::Class(class));
    }
    for alias in aliases {
        defs.entry(alias.name.as_str())
            .or_insert(TsNamedTypeRef::Alias(alias));
    }
    defs
}

fn ts_class_def<'a>(name: &str, defs: &'a TsNamedTypes<'a>) -> Option<&'a ClassInfo> {
    match defs.get(name.trim()) {
        Some(TsNamedTypeRef::Class(class)) => Some(*class),
        _ => None,
    }
}

fn ts_resolve_alias_text(type_name: &str, defs: &TsNamedTypes<'_>) -> Option<String> {
    fn inner(type_name: &str, defs: &TsNamedTypes<'_>, stack: &mut Vec<String>) -> Option<String> {
        let trimmed = type_name.trim();
        let named = match defs.get(trimmed) {
            Some(TsNamedTypeRef::Alias(alias)) => alias,
            _ => return None,
        };
        if stack.iter().any(|item| item == trimmed) {
            return None;
        }
        stack.push(trimmed.to_string());
        let resolved = inner(&named.type_annotation, defs, stack)
            .unwrap_or_else(|| named.type_annotation.clone());
        stack.pop();
        Some(resolved)
    }

    inner(type_name, defs, &mut vec![])
}

fn ts_effective_type(type_name: &str, defs: &TsNamedTypes<'_>) -> String {
    ts_resolve_alias_text(type_name, defs).unwrap_or_else(|| type_name.trim().to_string())
}

// ── Python fuzz harness ─────────────────────────────────────────────────────

fn synthesize_python(analysis: &AnalysisResult, type_defs: &HashMap<&str, &ClassInfo>) -> String {
    let mut code = String::new();

    // Embed a tiny random generator (no imports needed)
    code.push_str(PYTHON_FUZZ_PRELUDE);

    let mut any_synthesized = false;

    for func in &analysis.functions {
        if func.name.starts_with('_') || func.is_method || func.is_nested {
            continue;
        }

        let callable_params: Vec<&ParamInfo> = func
            .params
            .iter()
            .filter(|p| !p.name.starts_with('*'))
            .collect();

        // Check if we can generate values for all params
        let generators: Vec<String> = callable_params
            .iter()
            .map(|p| python_generator(p.type_annotation.as_deref(), type_defs))
            .collect();

        // Build the call with keyword args where needed
        let call_args: Vec<String> = callable_params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if p.keyword_only {
                    format!("{}=_args[{}]", p.name, i)
                } else {
                    format!("_args[{}]", i)
                }
            })
            .collect();

        let gen_list = generators.join(", ");
        let call = call_args.join(", ");
        let ret_type = func.return_type.as_deref().unwrap_or("");
        let edge_case_setup = if should_inject_python_edge_cases(func, &callable_params) {
            let param_type_list: String = callable_params
                .iter()
                .map(|p| {
                    format!(
                        "\"{}\"",
                        python_edge_type_name(p.type_annotation.as_deref())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "for _pi, _ptype in enumerate([{param_type_list}]):\n    for _ev in _edge_cases_for(_ptype):\n        _row = [{gen_list}]; _row[_pi] = _ev; _all_inputs.append(_row)\n"
            )
        } else {
            String::new()
        };

        code.push_str(&format!(
            r#"
_all_inputs = []
{edge_case_setup}
for _ in range({FUZZ_ITERATIONS}):
    _all_inputs.append([{gen_list}])
_pass = 0
_reject = 0
_crash = 0
for _args in _all_inputs:
    try:
        _result = {name}({call})
        _pass += 1
{type_check}
{idempotency_check}
{consistency_check}
{boundedness_check}
{nonneg_check}
{nullish_string_leak_check}
{comparator_check}
{symmetry_check}
    except Exception as _e:
        if _is_crash(_e):
            _crash += 1
            _FUZZ_RESULTS.append({{"function": "{name}", "input": _short_repr(_args),
                "error_type": type(_e).__name__, "message": _clip_text(str(_e)),
                "severity": "crash" if isinstance(_e, _CRASH_TYPES) else "property_violation"}})
            if _crash == 1:
                print(f"  CRASH {name}({{_short_repr(_args)}}): {{type(_e).__name__}}: {{_clip_text(str(_e))}}")
        else:
            _reject += 1
_total = _pass + _reject + _crash
if _crash > 0:
    print(f"FUZZ {name}: {{_pass}} passed, {{_reject}} rejected, {{_crash}} CRASHED (of {{_total}})")
    _fuzz_failures += 1
elif _pass == 0:
    print(f"FUZZ {name}: all {{_total}} inputs rejected (nothing tested)")
    _fuzz_failures += 1
else:
    print(f"FUZZ {name}: {{_pass}} passed, {{_reject}} rejected (of {{_total}})")
"#,
            name = func.name,
            edge_case_setup = edge_case_setup,
            type_check = python_type_check(ret_type, type_defs),
            idempotency_check = python_idempotency_check(func, &callable_params, type_defs),
            consistency_check = python_consistency_check(func, &call_args),
            boundedness_check = python_boundedness_check(func, &callable_params),
            nonneg_check = python_nonneg_check(func),
            nullish_string_leak_check = python_nullish_string_leak_check(func, &callable_params),
            comparator_check = python_comparator_check(func, &callable_params),
            symmetry_check = python_symmetry_check(func, &callable_params),
        ));

        any_synthesized = true;
    }

    if !any_synthesized {
        return String::new();
    }

    // Factory exercise: for functions containing nested functions,
    // call the factory and exercise the returned object's callables
    for func in &analysis.functions {
        if func.name.starts_with('_') || func.is_method || func.is_nested {
            continue;
        }
        let has_nested = analysis
            .functions
            .iter()
            .any(|f| f.is_nested && f.line >= func.line && f.end_line <= func.end_line);
        if !has_nested {
            continue;
        }
        let callable_params: Vec<&ParamInfo> = func
            .params
            .iter()
            .filter(|p| !p.name.starts_with('*'))
            .collect();
        let generators: Vec<String> = callable_params
            .iter()
            .map(|p| python_generator(p.type_annotation.as_deref(), type_defs))
            .collect();
        let gen_list = generators.join(", ");
        let nested_names: Vec<&str> = analysis
            .functions
            .iter()
            .filter(|f| f.is_nested && f.line >= func.line && f.end_line <= func.end_line)
            .map(|f| f.name.as_str())
            .collect();
        code.push_str(&format!(
            r#"
# Factory exercise: {name} -> test returned callables
_factory_pass = 0
_factory_crash = 0
for _fi in range({iters}):
    try:
        _factory_result = {name}({gen_list})
        if callable(_factory_result):
            try:
                _factory_result({gen_list})
            except Exception:
                pass
        elif hasattr(_factory_result, '__dict__'):
            for _attr in dir(_factory_result):
                if not _attr.startswith('_') and callable(getattr(_factory_result, _attr, None)):
                    try:
                        getattr(_factory_result, _attr)({gen_list})
                    except Exception:
                        pass
        _factory_pass += 1
    except Exception as _e:
        if _is_crash(_e):
            _factory_crash += 1
            _FUZZ_RESULTS.append({{"function": "{name} (factory)", "input": "factory call",
                "error_type": type(_e).__name__, "message": _clip_text(str(_e)),
                "severity": "crash"}})
            if _factory_crash == 1:
                print(f"  CRASH {name}(factory): {{type(_e).__name__}}: {{_clip_text(str(_e))}}")
_factory_total = _factory_pass + _factory_crash
if _factory_crash > 0:
    print(f"FUZZ {name} (factory->nested): {{_factory_pass}} passed, {{_factory_crash}} CRASHED (of {{_factory_total}}) [exercises: {nested}]")
    _fuzz_failures += 1
else:
    print(f"FUZZ {name} (factory->nested): {{_factory_pass}} passed (of {{_factory_total}}) [exercises: {nested}]")
"#,
            name = func.name,
            iters = FUZZ_ITERATIONS,
            nested = nested_names.join(", "),
        ));
    }

    // Involution roundtrip checks
    code.push_str(&synthesize_python_involution_checks(analysis, type_defs));

    code.push_str(PYTHON_FUZZ_EPILOGUE);
    code
}

fn python_generator(type_ann: Option<&str>, type_defs: &HashMap<&str, &ClassInfo>) -> String {
    let t = match type_ann {
        Some(t) => t.trim(),
        // No type annotation: generate a mix of types instead of just None
        None => return "_fuzz_any()".to_string(),
    };

    match t {
        "int" => "_fuzz_int()".into(),
        "float" => "_fuzz_float()".into(),
        "str" => "_fuzz_str()".into(),
        "bool" => "_fuzz_bool()".into(),
        "bytes" => "_fuzz_bytes()".into(),
        "Any" => "_fuzz_any()".into(),
        "dict" | "Dict" => "_fuzz_dict()".into(),
        _ if starts_with_any(t, &["list[", "List["]) => {
            let inner = extract_generic_arg(t);
            let gen = python_generator(Some(&inner), type_defs);
            format!("[{gen} for _ in range(_fuzz_int_range(0, 5))]")
        }
        _ if starts_with_any(t, &["dict[", "Dict["]) => {
            let (k, v) = extract_two_generic_args(t);
            let kg = python_generator(Some(&k), type_defs);
            let vg = python_generator(Some(&v), type_defs);
            format!("{{{kg}: {vg} for _ in range(_fuzz_int_range(0, 3))}}")
        }
        _ if t.starts_with("Optional[") => {
            let inner = extract_generic_arg(t);
            let gen = python_generator(Some(&inner), type_defs);
            format!("(None if _fuzz_bool() else {gen})")
        }
        _ if starts_with_any(t, &["tuple[", "Tuple["]) => {
            let inner = extract_generic_arg(t);
            let gen = python_generator(Some(&inner), type_defs);
            format!("({gen},)")
        }
        _ if starts_with_any(t, &["set[", "Set["]) => {
            let inner = extract_generic_arg(t);
            let gen = python_generator(Some(&inner), type_defs);
            format!("{{{gen} for _ in range(_fuzz_int_range(0, 5))}}")
        }
        _ if t.contains(" | ") => {
            // Union: pick a random branch (include None as a fuzzable option)
            let has_none = t.split('|').any(|s| s.trim() == "None");
            let branches: Vec<&str> = t
                .split('|')
                .map(|s| s.trim())
                .filter(|s| *s != "None")
                .collect();
            if branches.is_empty() {
                "_fuzz_none()".into()
            } else {
                let mut gens: Vec<String> = branches
                    .iter()
                    .map(|b| python_generator(Some(b), type_defs))
                    .collect();
                if has_none {
                    gens.push("None".into());
                }
                if gens.len() == 1 {
                    gens[0].clone()
                } else {
                    format!(
                        "[{}][_fuzz_int_range(0, {})]",
                        gens.join(", "),
                        gens.len() - 1
                    )
                }
            }
        }
        // Callback / function-typed parameters
        _ if t == "Callable" || t.starts_with("Callable[") => "(lambda *args: None)".into(),
        // Built-in types
        "datetime" | "date" => "__import__('datetime').datetime(2020, 1, 1)".into(),
        _ if type_defs.contains_key(t) => {
            let class = type_defs[t];
            if class.fields.is_empty() {
                format!("{t}()")
            } else {
                let args: Vec<String> = class
                    .fields
                    .iter()
                    .filter(|f| !f.has_default)
                    .map(|f| python_generator(f.type_annotation.as_deref(), type_defs))
                    .collect();
                format!("{}({})", t, args.join(", "))
            }
        }
        _ => "_fuzz_none()".into(),
    }
}

fn python_type_check(ret_type: &str, _type_defs: &HashMap<&str, &ClassInfo>) -> String {
    let check = match ret_type.trim() {
        "str" => "isinstance(_result, str)",
        "int" => "isinstance(_result, int)",
        "float" => "isinstance(_result, (int, float))",
        "bool" => "isinstance(_result, bool)",
        "bytes" => "isinstance(_result, bytes)",
        "" => return String::new(),
        t if t.contains("None") => return String::new(), // optional return, skip
        _ => return String::new(),
    };
    format!("        assert {check}, f\"Return type mismatch: got {{type(_result).__name__}}\"")
}

/// Names that suggest idempotent behavior (f(f(x)) == f(x)).
const IDEMPOTENT_NAME_CUES: &[&str] = &[
    "clean",
    "normalize",
    "strip",
    "trim",
    "lower",
    "upper",
    "casefold",
    "dedupe",
    "dedup",
    "unique",
    "sort",
    "flatten",
    "compact",
    "squeeze",
    "canonicalize",
    "simplify",
    "collapse",
    "sanitize",
    "format",
];

fn likely_idempotent(name: &str) -> bool {
    let lower = name.to_lowercase();
    IDEMPOTENT_NAME_CUES.iter().any(|cue| lower.contains(cue))
}

fn is_idempotent_candidate_type(type_name: &str) -> bool {
    let trimmed = type_name.trim();
    trimmed == "str"
        || trimmed == "string"
        || trimmed == "bytes"
        || starts_with_any(
            trimmed,
            &["list[", "List[", "set[", "Set[", "tuple[", "Tuple["],
        )
        || trimmed.ends_with("[]")
        || trimmed.starts_with("Array<")
}

/// Names that suggest bounded behavior (len(f(x)) <= len(x)).
const BOUNDED_NAME_CUES: &[&str] = &[
    "normalize",
    "clean",
    "trim",
    "strip",
    "compact",
    "collapse",
    "truncate",
];

fn likely_bounded(name: &str) -> bool {
    let lower = name.to_lowercase();
    BOUNDED_NAME_CUES.iter().any(|cue| lower.contains(cue))
}

/// Names that suggest non-negative return values.
const NONNEG_NAME_CUES: &[&str] = &[
    "len", "count", "size", "distance", "abs", "norm", "score", "total",
];

fn likely_nonneg(name: &str) -> bool {
    let lower = name.to_lowercase();
    NONNEG_NAME_CUES.iter().any(|cue| lower.contains(cue))
}

/// Names that suggest a returned string should not be blank after trimming.
const NONEMPTY_STRING_NAME_CUES: &[&str] = &[
    "name", "label", "title", "city", "country", "domain", "email", "handle", "initial", "plan",
    "slug", "tagline", "timezone",
];

fn likely_nonempty_string(name: &str) -> bool {
    if likely_bounded(name) {
        return false;
    }
    let lower = name.to_lowercase();
    NONEMPTY_STRING_NAME_CUES
        .iter()
        .any(|cue| lower.contains(cue))
}

/// Names that suggest serialized/canonical string output.
const NULLISH_STRING_LEAK_NAME_CUES: &[&str] = &[
    "query",
    "serialize",
    "serialise",
    "canonical",
    "encode",
    "stringify",
];

fn likely_nullish_string_leak(name: &str) -> bool {
    let lower = name.to_lowercase();
    NULLISH_STRING_LEAK_NAME_CUES
        .iter()
        .any(|cue| lower.contains(cue))
}

/// Names that suggest symmetric behavior (f(a,b) == f(b,a)).
const SYMMETRIC_NAME_CUES: &[&str] = &["distance", "similarity", "hamming", "gcd"];

/// Names that suggest comparator-style ordering (antisymmetric, NOT symmetric).
/// Keep this list narrow: generic words like "order" create false positives for
/// ordinary business functions such as average_order_value(total, count).
const ANTISYMMETRIC_NAME_CUES: &[&str] = &["compare", "cmp", "sort", "asc", "desc"];

fn likely_symmetric(name: &str) -> bool {
    let lower = name.to_lowercase();
    if ANTISYMMETRIC_NAME_CUES
        .iter()
        .any(|cue| lower.contains(cue))
    {
        return false;
    }
    SYMMETRIC_NAME_CUES.iter().any(|cue| lower.contains(cue))
}

fn is_api_surface(func: &FunctionInfo) -> bool {
    !func.is_method && !func.is_nested && func.is_exported
}

fn ts_has_structured_input_type(type_name: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    let trimmed = type_name.trim();
    if trimmed.is_empty() {
        return false;
    }

    let union_branches = split_ts_top_level(trimmed, '|');
    if union_branches.len() > 1 {
        return union_branches.iter().any(|branch| {
            let branch = branch.trim();
            !matches!(branch, "null" | "undefined")
                && ts_has_structured_input_type(branch, type_defs)
        });
    }

    let intersection_branches = split_ts_top_level(trimmed, '&');
    if intersection_branches.len() > 1 {
        return intersection_branches
            .iter()
            .any(|branch| ts_has_structured_input_type(branch.trim(), type_defs));
    }

    let effective = ts_effective_type(trimmed, type_defs);
    let effective = effective.trim();

    is_ts_mapping_type(effective, type_defs)
        || effective.ends_with("[]")
        || effective.starts_with("Array<")
        || effective.starts_with("ReadonlyArray<")
        || effective.starts_with('[')
        || ts_class_def(effective, type_defs).is_some()
}

fn should_require_ts_nonempty_string(
    func: &FunctionInfo,
    param_types: &[String],
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    if !likely_nonempty_string(&func.name) {
        return false;
    }

    is_api_surface(func)
        || (!func.is_method
            && !func.is_nested
            && param_types
                .iter()
                .any(|param_type| ts_has_structured_input_type(param_type, type_defs)))
}

fn is_python_mapping_type(type_name: &str) -> bool {
    matches!(type_name.trim(), "dict" | "Dict")
        || starts_with_any(type_name.trim(), &["dict[", "Dict["])
}

fn is_ts_mapping_type(type_name: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    let effective = ts_effective_type(type_name, type_defs);
    let trimmed = effective.trim();
    trimmed.starts_with("Record<") || looks_like_ts_object_type(trimmed)
}

fn infer_python_contract(func: &FunctionInfo, params: &[&ParamInfo]) -> Option<ContractKind> {
    let ret_type = func.return_type.as_deref().unwrap_or("").trim();
    if params.len() == 1 {
        let param_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
        if param_type == "str" && ret_type == "str" {
            return Some(ContractKind::StringTransform);
        }
        if is_python_mapping_type(param_type) && ret_type == "str" {
            return Some(ContractKind::MappingSerializer);
        }
    }
    if params.len() == 2 {
        let left = params[0].type_annotation.as_deref().unwrap_or("").trim();
        let right = params[1].type_annotation.as_deref().unwrap_or("").trim();
        let lower = func.name.to_lowercase();
        if !left.is_empty()
            && left == right
            && matches!(ret_type, "int" | "float")
            && ANTISYMMETRIC_NAME_CUES
                .iter()
                .any(|cue| lower.contains(cue))
        {
            return Some(ContractKind::Comparator);
        }
    }
    None
}

fn infer_ts_contract(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> Option<ContractKind> {
    if param_types.len() == 1 {
        let param_type = param_types[0].trim();
        let ret_type = ret_type.trim();
        if param_type == "string" && ret_type == "string" {
            return Some(ContractKind::StringTransform);
        }
        if is_ts_mapping_type(param_type, type_defs) && ret_type == "string" {
            return Some(ContractKind::MappingSerializer);
        }
    }
    if param_types.len() == 2 {
        let left = param_types[0].trim();
        let right = param_types[1].trim();
        let lower = func.name.to_lowercase();
        if !left.is_empty()
            && left == right
            && ret_type.trim() == "number"
            && ANTISYMMETRIC_NAME_CUES
                .iter()
                .any(|cue| lower.contains(cue))
        {
            return Some(ContractKind::Comparator);
        }
    }
    None
}

fn should_inject_python_edge_cases(func: &FunctionInfo, params: &[&ParamInfo]) -> bool {
    is_api_surface(func) || infer_python_contract(func, params).is_some()
}

fn should_inject_ts_edge_cases(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    let contract = infer_ts_contract(func, param_types, ret_type, type_defs);
    is_api_surface(func)
        || contract.is_some()
        || params.iter().any(|param| {
            !ts_edge_type_name_for_param(contract, param.type_annotation.as_deref(), type_defs)
                .is_empty()
        })
}

fn python_idempotency_check(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    _type_defs: &HashMap<&str, &ClassInfo>,
) -> String {
    // Only check idempotency for single-arg functions where:
    // 1. Input and output types match
    // 2. Function name suggests idempotent behavior
    if params.len() != 1 {
        return String::new();
    }
    let param_type = params[0].type_annotation.as_deref().unwrap_or("");
    let ret_type = func.return_type.as_deref().unwrap_or("");
    if param_type == ret_type
        && !param_type.is_empty()
        && !param_type.contains("None")
        && is_idempotent_candidate_type(param_type)
        && is_api_surface(func)
        && likely_idempotent(&func.name)
    {
        format!(
            "        _result2 = {name}(_result)\n        assert _nan_eq(_result, _result2), f\"Not idempotent: {{repr(_result)}} -> {{repr(_result2)}}\"",
            name = func.name,
        )
    } else {
        String::new()
    }
}

fn python_consistency_check(func: &FunctionInfo, call_args: &[String]) -> String {
    // Run the same input twice, verify same output
    let call = call_args.join(", ");
    format!(
        "        _result_b = {name}({call})\n        assert _nan_eq(_result, _result_b), f\"Inconsistent: {{repr(_result)}} != {{repr(_result_b)}}\"",
        name = func.name,
    )
}

fn python_boundedness_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if params.len() != 1 {
        return String::new();
    }
    let param_type = params[0].type_annotation.as_deref().unwrap_or("");
    let ret_type = func.return_type.as_deref().unwrap_or("");
    let types_match = (param_type == "str" && ret_type == "str")
        || (starts_with_any(param_type, &["list", "List"])
            && starts_with_any(ret_type, &["list", "List"]));
    if types_match && is_api_surface(func) && likely_bounded(&func.name) {
        "        assert len(_result) <= len(_args[0]), f\"Not bounded: len({repr(_result)}) > len({repr(_args[0])})\"".to_string()
    } else {
        String::new()
    }
}

fn python_nonneg_check(func: &FunctionInfo) -> String {
    let ret_type = func.return_type.as_deref().unwrap_or("");
    if (ret_type == "int" || ret_type == "float")
        && is_api_surface(func)
        && likely_nonneg(&func.name)
    {
        "        assert _result >= 0, f\"Non-negative violation: {repr(_result)} < 0\"".to_string()
    } else {
        String::new()
    }
}

fn python_nullish_string_leak_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if params.len() != 1 {
        return String::new();
    }
    let param_type = params[0].type_annotation.as_deref().unwrap_or("");
    let ret_type = func.return_type.as_deref().unwrap_or("");
    let accepts_mapping = param_type == "dict"
        || param_type == "Dict"
        || starts_with_any(param_type, &["dict[", "Dict["]);
    if ret_type == "str"
        && accepts_mapping
        && (is_api_surface(func)
            || infer_python_contract(func, params) == Some(ContractKind::MappingSerializer))
        && likely_nullish_string_leak(&func.name)
    {
        "        if _contains_nullish(_args[0]) and _string_leaks_nullish(_result):\n            raise AssertionError(f\"Nullish string leak: {repr(_result)}\")".to_string()
    } else {
        String::new()
    }
}

fn python_comparator_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if infer_python_contract(func, params) == Some(ContractKind::Comparator) {
        format!(
            "        _self_cmp = {name}(_args[0], _args[0])\n        assert _cmp_sign(_self_cmp) == 0, f\"Comparator self-compare should be zero: {{repr(_self_cmp)}}\"\n        _rev_cmp = {name}(_args[1], _args[0])\n        assert _cmp_sign(_result) == -_cmp_sign(_rev_cmp), f\"Comparator antisymmetry violated: {{repr(_result)}} vs {{repr(_rev_cmp)}}\"",
            name = func.name,
        )
    } else {
        String::new()
    }
}

fn python_symmetry_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if params.len() != 2 {
        return String::new();
    }
    let t0 = params[0].type_annotation.as_deref().unwrap_or("");
    let t1 = params[1].type_annotation.as_deref().unwrap_or("");
    if t0 == t1 && !t0.is_empty() && is_api_surface(func) && likely_symmetric(&func.name) {
        format!(
            "        _result_sym = {name}(_args[1], _args[0])\n        assert _nan_eq(_result, _result_sym), f\"Not symmetric: {{repr(_result)}} != {{repr(_result_sym)}}\"",
            name = func.name,
        )
    } else {
        String::new()
    }
}

const PYTHON_FUZZ_PRELUDE: &str = r#"
import random as _rng
import json as _json
_rng.seed(42)
_fuzz_failures = 0
_FUZZ_RESULTS = []
_FUZZ_TEXT_LIMIT = 240

# Crash detection: these exception types indicate real bugs, not validation.
_CRASH_TYPES = (TypeError, AttributeError, KeyError, IndexError, RecursionError, MemoryError, ValueError, ZeroDivisionError, UnicodeError)

def _clip_text(value, limit=_FUZZ_TEXT_LIMIT):
    text = str(value)
    if len(text) <= limit:
        return text
    return f"{text[:limit]}... [truncated {len(text) - limit} chars]"

def _short_repr(value, limit=_FUZZ_TEXT_LIMIT):
    return _clip_text(repr(value), limit)

def _is_crash(e):
    """Distinguish intentional validation errors from real bugs."""
    if isinstance(e, _CRASH_TYPES):
        return True
    if isinstance(e, AssertionError):
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
    return _rng.choice([
        {},
        {"value": None},
        {"preferences": None},
        {"preferences": {"timezone": _fuzz_str()}},
        {"flags": None},
        {"flags": {"beta_checkout": None}},
        {"flags": {"beta_checkout": False}},
        {"flags": {"beta_checkout": True}},
        {"billing": None},
        {"billing": {"country": _fuzz_str()}},
        {"contacts": None},
        {"contacts": {"support_email": _fuzz_str()}},
        {"contacts": {"emails": [_fuzz_str()]}},
        {"contacts": {"emails": [_fuzz_str(), _fuzz_str()]}},
        {"profile": None},
        {"profile": {"handle": _fuzz_str()}},
        {"username": _fuzz_str()},
        {"titles": []},
        {"titles": [_fuzz_str()]},
        {"segments": []},
        {"segments": [_fuzz_str()]},
        {"plans": []},
        {"plans": [_fuzz_str()]},
        {"plans": [None, _fuzz_str()]},
        {"tag": ["pro", None, " beta "]},
        {"q": "  ", "page": 2},
        {"q": "naïve café"},
    ])

_EDGE_INTS = [0, 1, -1, 2**53, -(2**53), 2**53 + 1]
_EDGE_FLOATS = [0.0, -0.0, float('inf'), float('-inf'), float('nan'), 1e-300, 1e300]
_EDGE_STRS = ["", "\0", "\uFFFF", "a" * 10000, "true", "null", "0", "-1",
              "\r\n", "\u200F", "\u200D", "${...}", "<script>"]
_EDGE_BYTES = [b"", b"\x00", b"\xff" * 100, bytes(range(256))]
_EDGE_DICTS = [
    {},
    {"preferences": None},
    {"preferences": {"timezone": "   "}},
    {"flags": None},
    {"flags": {"beta_checkout": None}},
    {"flags": {"beta_checkout": False}},
    {"flags": {"beta_checkout": True}},
    {"billing": None},
    {"billing": {"country": "   "}},
    {"contacts": None},
    {"contacts": {"support_email": "ops"}},
    {"contacts": {"support_email": "   "}},
    {"contacts": {"emails": ["owner@example.com"]}},
    {"contacts": {"emails": ["owner@example.com", "   "]}},
    {"profile": None},
    {"profile": {"handle": "   "}},
    {"username": "   "},
    {"titles": []},
    {"titles": ["   "]},
    {"segments": []},
    {"segments": ["   "]},
    {"segments": ["   ", "Growth"]},
    {"plans": []},
    {"plans": ["   "]},
    {"plans": [None, " team "]},
    {"tag": ["pro", None, " beta "]},
    {"q": "  ", "page": 2},
    {"q": "naïve café"},
]

def _edge_cases_for(type_name):
    m = {"int": _EDGE_INTS, "float": _EDGE_FLOATS, "str": _EDGE_STRS, "bytes": _EDGE_BYTES, "dict": _EDGE_DICTS}
    return m.get(type_name, [])

def _nan_eq(a, b):
    if isinstance(a, float) and isinstance(b, float):
        import math
        if math.isnan(a) and math.isnan(b): return True
    return a == b

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

def _cmp_sign(value):
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, (int, float)):
        if value < 0:
            return -1
        if value > 0:
            return 1
        return 0
    raise AssertionError(f"Comparator returned non-numeric value: {repr(value)}")
"#;

const PYTHON_FUZZ_EPILOGUE: &str = r#"
if _FUZZ_RESULTS:
    print("__COURT_JESTER_FUZZ_JSON__")
    print(_json.dumps(_FUZZ_RESULTS))
if _fuzz_failures > 0:
    raise AssertionError(f"Fuzz testing failed: {_fuzz_failures} function(s) had failures")
else:
    print(f"All fuzz tests passed")
"#;

// ── TypeScript fuzz harness ─────────────────────────────────────────────────

fn synthesize_typescript(analysis: &AnalysisResult, type_defs: &TsNamedTypes<'_>) -> String {
    let mut code = String::new();

    code.push_str(TYPESCRIPT_FUZZ_PRELUDE);

    let mut any_synthesized = false;

    for func in &analysis.functions {
        if func.name.starts_with('_') || func.is_method || func.is_nested {
            continue;
        }

        let callable_params: Vec<&ParamInfo> = func
            .params
            .iter()
            .filter(|p| !p.name.starts_with('*'))
            .collect();
        if !ts_params_are_fuzzable(&callable_params, type_defs) {
            continue;
        }
        let ret_type = func.return_type.as_deref().unwrap_or("");
        let param_types: Vec<String> = callable_params
            .iter()
            .map(|p| {
                p.type_annotation
                    .as_deref()
                    .map(|t| ts_effective_type(t, type_defs))
                    .unwrap_or_default()
            })
            .collect();

        let contract = infer_ts_contract(func, &param_types, ret_type, type_defs);
        let generators: Vec<String> = callable_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                ts_generator_for_param(contract, p.type_annotation.as_deref(), type_defs, idx, func)
            })
            .collect();

        let gen_list = generators.join(", ");

        let mut properties: Vec<&str> = vec![];

        // Idempotency: single-arg, same in/out type, name suggests it
        if callable_params.len() == 1
            && !param_types[0].is_empty()
            && param_types[0].as_str() == ret_type
            && !ret_type.contains("null")
            && !ret_type.contains("undefined")
            && is_idempotent_candidate_type(ret_type)
            && is_api_surface(func)
            && likely_idempotent(&func.name)
        {
            properties.push("idempotent");
        }
        // Boundedness: single-arg, str→str or array→array, name suggests it
        if callable_params.len() == 1
            && ((param_types[0].as_str() == "string" && ret_type == "string")
                || (param_types[0].ends_with("[]") && ret_type.ends_with("[]")))
            && is_api_surface(func)
            && likely_bounded(&func.name)
        {
            properties.push("bounded");
        }
        // Non-negativity: returns number, name suggests it
        if ret_type == "number" && is_api_surface(func) && likely_nonneg(&func.name) {
            properties.push("nonneg");
        }
        // Non-empty strings: string-returning identifier/display helpers should
        // not silently return blank output.
        if ret_type == "string" && should_require_ts_nonempty_string(func, &param_types, type_defs)
        {
            properties.push("nonempty_string");
        }
        // Serialized/canonical string helpers should not leak nullish sentinel
        // text into the output when the input contains null/undefined values.
        if callable_params.len() == 1
            && ret_type == "string"
            && is_ts_mapping_type(&param_types[0], type_defs)
            && (is_api_surface(func)
                || infer_ts_contract(func, &param_types, ret_type, type_defs)
                    == Some(ContractKind::MappingSerializer))
            && likely_nullish_string_leak(&func.name)
        {
            properties.push("no_nullish_string");
        }
        if infer_ts_contract(func, &param_types, ret_type, type_defs)
            == Some(ContractKind::Comparator)
        {
            properties.push("comparator");
        }
        // Symmetry: two params same type, name suggests it
        if callable_params.len() == 2
            && !param_types[0].is_empty()
            && param_types[0] == param_types[1]
            && is_api_surface(func)
            && likely_symmetric(&func.name)
        {
            properties.push("symmetric");
        }

        let properties_list: String = properties
            .iter()
            .map(|p| format!("\"{}\"", p))
            .collect::<Vec<_>>()
            .join(", ");

        let param_type_list: String = if should_inject_ts_edge_cases(
            func,
            &callable_params,
            &param_types,
            ret_type,
            type_defs,
        ) {
            callable_params
                .iter()
                .map(|p| {
                    format!(
                        "\"{}\"",
                        ts_edge_type_name_for_param(
                            contract,
                            p.type_annotation.as_deref(),
                            type_defs
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        };

        code.push_str(&format!(
            r#"
_fuzzOne("{name}", {iters}, () => [{gen_list}], (args: unknown[]) => ({name} as Function)(...args), {typecheck}, [{param_type_list}], [{properties_list}]);
"#,
            name = func.name,
            iters = FUZZ_ITERATIONS,
            typecheck = ts_type_check_fn(ret_type),
        ));

        any_synthesized = true;
    }

    if !any_synthesized {
        return String::new();
    }

    // Factory exercise: for functions that contain nested functions,
    // call the factory and fuzz the returned object's methods
    code.push_str(&synthesize_typescript_factory_exercise(analysis, type_defs));

    // Involution roundtrip checks
    code.push_str(&synthesize_typescript_involution_checks(
        analysis, type_defs,
    ));

    code.push_str(TYPESCRIPT_FUZZ_EPILOGUE);
    code
}

/// For factory functions (functions containing nested function definitions),
/// call the factory with fuzzed args, then exercise any callable properties
/// on the returned object.
fn synthesize_typescript_factory_exercise(
    analysis: &AnalysisResult,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    let mut code = String::new();

    for func in &analysis.functions {
        if func.name.starts_with('_') || func.is_method || func.is_nested {
            continue;
        }

        // Check if this function contains nested functions
        let has_nested = analysis
            .functions
            .iter()
            .any(|f| f.is_nested && f.line >= func.line && f.end_line <= func.end_line);
        if !has_nested {
            continue;
        }

        // Build args for the factory call
        let callable_params: Vec<&ParamInfo> = func
            .params
            .iter()
            .filter(|p| !p.name.starts_with('*'))
            .collect();
        if !ts_params_are_fuzzable(&callable_params, type_defs) {
            continue;
        }
        let generators: Vec<String> = callable_params
            .iter()
            .map(|p| ts_generator(p.type_annotation.as_deref(), type_defs))
            .collect();
        let gen_list = generators.join(", ");

        // Collect the nested function names for reporting
        let nested_names: Vec<&str> = analysis
            .functions
            .iter()
            .filter(|f| f.is_nested && f.line >= func.line && f.end_line <= func.end_line)
            .map(|f| f.name.as_str())
            .collect();

        code.push_str(&format!(
            r#"
// Factory exercise: {name} -> test returned methods
{{
  let _factoryPass = 0, _factoryCrash = 0;
  for (let _fi = 0; _fi < {iters}; _fi++) {{
    try {{
      const _factory = ({name} as Function)({gen_list});
      if (_factory && typeof _factory === "object") {{
        for (const _key of Object.keys(_factory)) {{
          if (typeof _factory[_key] === "function") {{
            try {{
              _factory[_key]({gen_list});
            }} catch (_inner) {{
              // inner method errors are expected (we're passing random args)
            }}
          }}
        }}
      }}
      if (typeof _factory === "function") {{
        try {{ _factory({gen_list}); }} catch (_inner) {{}}
      }}
      _factoryPass++;
    }} catch (_e: unknown) {{
      if (_isCrash(_e)) {{
        _factoryCrash++;
        _fuzzResults.push({{function: "{name} (factory)", input: "factory call",
          error_type: _e instanceof Error ? _e.constructor.name : "unknown",
          message: _clipText(_e instanceof Error ? _e.message : String(_e)),
          severity: "crash"}});
        if (_factoryCrash === 1) console.log(`  CRASH {name}(factory): ${{_clipText(_e)}}`);
      }}
    }}
  }}
  const _ftotal = _factoryPass + _factoryCrash;
  if (_factoryCrash > 0) {{
    console.log(`FUZZ {name} (factory->nested): ${{_factoryPass}} passed, ${{_factoryCrash}} CRASHED (of ${{_ftotal}}) [exercises: {nested}]`);
    _fuzzTotalFailures++;
  }} else {{
    console.log(`FUZZ {name} (factory->nested): ${{_factoryPass}} passed (of ${{_ftotal}}) [exercises: {nested}]`);
  }}
}}
"#,
            name = func.name,
            iters = FUZZ_ITERATIONS,
            nested = nested_names.join(", "),
        ));
    }

    code
}

fn ts_generator(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> String {
    let t = match type_ann {
        Some(t) => t.trim(),
        // No type annotation: generate a mix of types instead of just undefined
        None => return "_fuzzAny()".to_string(),
    };
    if let Some(resolved) = ts_resolve_alias_text(t, type_defs) {
        return ts_generator(Some(&resolved), type_defs);
    }

    match t {
        "number" => "_fuzzNum()".into(),
        "string" => "_fuzzStr()".into(),
        "boolean" => "_fuzzBool()".into(),
        "any" | "unknown" => "_fuzzAny()".into(),
        _ if t.ends_with("[]") => {
            let inner = &t[..t.len() - 2];
            let gen = ts_generator(Some(inner), type_defs);
            format!("Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen})")
        }
        _ if t.starts_with("Array<") => {
            let inner = extract_generic_arg(t);
            let gen = ts_generator(Some(&inner), type_defs);
            format!("Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen})")
        }
        _ if t.starts_with("Record<") => {
            let (_k, v) = extract_two_generic_args(t);
            let vg = ts_generator(Some(&v), type_defs);
            format!("Object.fromEntries(Array.from({{length: _fuzzIntRange(0,3)}}, (_, i) => [`k${{i}}`, {vg}]))")
        }
        _ if looks_like_ts_object_type(t) => ts_inline_object_generator(t, type_defs),
        _ => {
            let union_branches = split_ts_top_level(t, '|');
            if union_branches.len() > 1 {
                let has_null = union_branches.iter().any(|s| {
                    let s = s.trim();
                    s == "null" || s == "undefined"
                });
                let branches: Vec<&str> = union_branches
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| *s != "null" && *s != "undefined")
                    .collect();
                if branches.is_empty() {
                    "null".into()
                } else {
                    let mut gens: Vec<String> = branches
                        .iter()
                        .map(|b| ts_generator(Some(b), type_defs))
                        .collect();
                    // Include null/undefined as a fuzzable branch if the type allows it
                    if has_null {
                        gens.push("null".into());
                    }
                    if gens.len() == 1 {
                        gens[0].clone()
                    } else {
                        format!(
                            "[{}][_fuzzIntRange(0, {})]",
                            gens.join(", "),
                            gens.len() - 1
                        )
                    }
                }
            } else if t.contains(" & ") {
                let intersection = split_ts_top_level(t, '&');
                let first = intersection.first().map(|s| s.trim()).unwrap_or(t);
                ts_generator(Some(first), type_defs)
            } else if t.contains("=>") {
                "(() => undefined)".into()
            } else if t == "Date" {
                "new Date(_fuzzNum() * 1e6)".into()
            } else if t == "RegExp" {
                "/test/i".into()
            } else if t == "Map" {
                "new Map()".into()
            } else if t == "Set" {
                "new Set()".into()
            } else if t == "Error" {
                "new Error(_fuzzStr())".into()
            } else if t == "Buffer" {
                "Buffer.from(_fuzzStr())".into()
            } else if t == "Uint8Array" {
                "new Uint8Array(0)".into()
            } else if t == "ArrayBuffer" {
                "new ArrayBuffer(0)".into()
            } else if t == "URL" {
                "new URL('https://example.com/' + _fuzzStr().replace(/[^a-z0-9]/gi, ''))".into()
            } else if t == "Request" {
                "new Request('https://example.com', {headers: {'content-type': 'application/json'}})".into()
            } else if t == "Response" {
                "new Response('ok')".into()
            } else if t == "Headers" {
                "new Headers({'content-type': 'text/plain'})".into()
            } else if t == "FormData" {
                "new FormData()".into()
            } else if t == "AbortController" {
                "new AbortController()".into()
            } else if t.starts_with("Promise<") {
                let inner = extract_generic_arg(t);
                let gen = ts_generator(Some(&inner), type_defs);
                format!("Promise.resolve({gen})")
            } else if t == "Promise" {
                "Promise.resolve(_fuzzAny())".into()
            } else if let Some(class) = ts_class_def(t, type_defs) {
                if class.fields.is_empty() {
                    "({})".into()
                } else {
                    let props: Vec<String> = class
                        .fields
                        .iter()
                        .map(|f| {
                            let field_gen = ts_field_generator(
                                f.name.as_str(),
                                f.type_annotation.as_deref(),
                                type_defs,
                            );
                            let val = if f.optional {
                                format!("_fuzzBool() ? null : {}", field_gen)
                            } else {
                                field_gen
                            };
                            format!("{}: {}", f.name, val)
                        })
                        .collect();
                    format!("({{ {} }})", props.join(", "))
                }
            } else if t.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                "({})".into()
            } else {
                "undefined".into()
            }
        }
    }
}

fn ts_params_are_fuzzable(params: &[&ParamInfo], type_defs: &TsNamedTypes<'_>) -> bool {
    params
        .iter()
        .all(|param| ts_type_is_fuzzable(param.type_annotation.as_deref(), type_defs))
}

fn ts_type_is_fuzzable(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> bool {
    let t = match type_ann {
        Some(t) => t.trim(),
        None => return true,
    };
    if let Some(resolved) = ts_resolve_alias_text(t, type_defs) {
        return ts_type_is_fuzzable(Some(&resolved), type_defs);
    }

    match t {
        "number" | "string" | "boolean" | "any" | "unknown" | "null" | "undefined" | "Date"
        | "RegExp" | "Map" | "Set" | "Error" | "Buffer" | "Uint8Array" | "ArrayBuffer" | "URL"
        | "Request" | "Response" | "Headers" | "FormData" | "AbortController" | "Promise" => true,
        _ if t.ends_with("[]") => {
            let inner = &t[..t.len() - 2];
            ts_type_is_fuzzable(Some(inner), type_defs)
        }
        _ if t.starts_with("Array<") => {
            let inner = extract_generic_arg(t);
            ts_type_is_fuzzable(Some(&inner), type_defs)
        }
        _ if t.starts_with("Record<") => {
            let (k, v) = extract_two_generic_args(t);
            ts_type_is_fuzzable(Some(&k), type_defs) && ts_type_is_fuzzable(Some(&v), type_defs)
        }
        _ if looks_like_ts_object_type(t) => extract_ts_object_type_fields_from_text(t)
            .iter()
            .all(|field| ts_type_is_fuzzable(field.type_annotation.as_deref(), type_defs)),
        _ => {
            let union_branches = split_ts_top_level(t, '|');
            if union_branches.len() > 1 {
                union_branches.iter().all(|branch| {
                    matches!(branch.trim(), "null" | "undefined")
                        || ts_type_is_fuzzable(Some(branch.trim()), type_defs)
                })
            } else if t.contains(" & ") {
                split_ts_top_level(t, '&')
                    .iter()
                    .all(|branch| ts_type_is_fuzzable(Some(branch.trim()), type_defs))
            } else if t.contains("=>") {
                true
            } else if t.starts_with("Promise<") {
                let inner = extract_generic_arg(t);
                ts_type_is_fuzzable(Some(&inner), type_defs)
            } else if let Some(class) = ts_class_def(t, type_defs) {
                class
                    .fields
                    .iter()
                    .all(|field| ts_type_is_fuzzable(field.type_annotation.as_deref(), type_defs))
            } else if t.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                false
            } else {
                true
            }
        }
    }
}

fn ts_generator_for_param(
    contract: Option<ContractKind>,
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
    _index: usize,
    _func: &FunctionInfo,
) -> String {
    if contract == Some(ContractKind::Comparator)
        && is_semver_like_version_type(type_ann, type_defs)
    {
        return "_fuzzSemverVersion()".to_string();
    }
    ts_generator(type_ann, type_defs)
}

fn ts_edge_type_name_for_param(
    contract: Option<ContractKind>,
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
) -> &'static str {
    if contract == Some(ContractKind::Comparator)
        && is_semver_like_version_type(type_ann, type_defs)
    {
        return "semver_version";
    }
    ts_edge_type_name(type_ann, type_defs)
}

fn is_semver_like_version_type(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> bool {
    let resolved = match type_ann {
        Some(t) => ts_effective_type(t, type_defs),
        None => return false,
    };
    let class = match ts_class_def(&resolved, type_defs) {
        Some(class) => class,
        None => return false,
    };

    let mut has_major = false;
    let mut has_minor = false;
    let mut has_patch = false;
    let mut has_prerelease = false;
    let mut prerelease_type_ok = false;

    for field in &class.fields {
        match field.name.as_str() {
            "major" => {
                has_major = field
                    .type_annotation
                    .as_deref()
                    .map_or(false, |ann| ann.trim() == "number");
            }
            "minor" => {
                has_minor = field
                    .type_annotation
                    .as_deref()
                    .map_or(false, |ann| ann.trim() == "number");
            }
            "patch" => {
                has_patch = field
                    .type_annotation
                    .as_deref()
                    .map_or(false, |ann| ann.trim() == "number");
            }
            "prerelease" => {
                has_prerelease = true;
                prerelease_type_ok = field
                    .type_annotation
                    .as_deref()
                    .map(|ann| {
                        let normalized = ann.trim();
                        normalized.starts_with("string[]")
                            || normalized.starts_with("Array<string>")
                    })
                    .unwrap_or(false);
            }
            _ => {}
        }
    }

    has_major && has_minor && has_patch && has_prerelease && prerelease_type_ok
}

fn ts_field_generator(
    field_name: &str,
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    if is_semver_part_field(field_name, type_ann) {
        return "_fuzzSemverPart()".into();
    }
    if is_semver_prerelease_field(field_name, type_ann) {
        return "[null, [], [_fuzzSemverIdentifier()], [_fuzzSemverIdentifier(), _fuzzSemverIdentifier()]][_fuzzIntRange(0, 3)]".into();
    }
    let base = ts_generator(type_ann, type_defs);
    let is_string_like = type_ann.map(|t| t.contains("string")).unwrap_or(false);

    if is_string_like && likely_nonempty_string(field_name) {
        format!("[{}, \"\", \"   \"][_fuzzIntRange(0, 2)]", base)
    } else {
        base
    }
}

fn is_semver_part_field(field_name: &str, type_ann: Option<&str>) -> bool {
    let normalized = field_name.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "major" | "minor" | "patch")
        && matches!(type_ann.map(|t| t.trim()), Some("number"))
}

fn is_semver_prerelease_field(field_name: &str, type_ann: Option<&str>) -> bool {
    let normalized = field_name.trim().to_ascii_lowercase();
    normalized == "prerelease"
        && type_ann
            .map(|t| {
                let trimmed = t.trim();
                trimmed.contains("string[]") || trimmed.starts_with("Array<string>")
            })
            .unwrap_or(false)
}

fn looks_like_ts_object_type(type_ann: &str) -> bool {
    let trimmed = type_ann.trim();
    trimmed.starts_with('{') && trimmed.ends_with('}')
}

fn ts_inline_object_generator(type_ann: &str, type_defs: &TsNamedTypes<'_>) -> String {
    let fields = extract_ts_object_type_fields_from_text(type_ann);
    if fields.is_empty() {
        return "({})".into();
    }

    let props: Vec<String> = fields
        .iter()
        .map(|field| {
            let field_gen = ts_field_generator(
                field.name.as_str(),
                field.type_annotation.as_deref(),
                type_defs,
            );
            let val = if field.optional {
                format!("_fuzzBool() ? null : {}", field_gen)
            } else {
                field_gen
            };
            format!("{}: {}", field.name, val)
        })
        .collect();

    format!("({{ {} }})", props.join(", "))
}

fn extract_ts_object_type_fields_from_text(type_ann: &str) -> Vec<FieldInfo> {
    let trimmed = type_ann.trim();
    if !looks_like_ts_object_type(trimmed) {
        return vec![];
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;

    for (idx, ch) in inner.char_indices() {
        match ch {
            '{' | '[' | '<' | '(' => depth += 1,
            '}' | ']' | '>' | ')' => depth -= 1,
            ';' | ',' if depth == 0 => {
                let segment = inner[start..idx].trim();
                if !segment.is_empty() {
                    segments.push(segment.to_string());
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = inner[start..].trim();
    if !tail.is_empty() {
        segments.push(tail.to_string());
    }

    segments
        .into_iter()
        .filter_map(|segment| {
            let colon_idx = segment.find(':')?;
            let raw_name = segment[..colon_idx].trim();
            let type_part = segment[colon_idx + 1..].trim();
            if raw_name.is_empty() || type_part.is_empty() {
                return None;
            }

            let optional = raw_name.ends_with('?');
            let name = raw_name.trim_end_matches('?').trim().to_string();
            if name.is_empty() {
                return None;
            }

            Some(FieldInfo {
                name,
                type_annotation: Some(type_part.to_string()),
                optional,
                has_default: false,
            })
        })
        .collect()
}

fn ts_type_check_fn(ret_type: &str) -> &str {
    match ret_type.trim() {
        "string" => "\"string\"",
        "number" => "\"number\"",
        "boolean" => "\"boolean\"",
        _ => "null",
    }
}

const TYPESCRIPT_FUZZ_PRELUDE: &str = r#"
let _seed = 42;
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
function _fuzzStr(): string {
  const pools = [
    "", "hello world", "café résumé", "  whitespace  ", "\t\nnewlines",
    "UPPER", "lower", "MiXeD", "special!@#$%^&*()", "12345", "-1.5",
    "a".repeat(200), "\xa0\xa0\xa0", "with\nnewlines\n",
    String.fromCharCode(...Array.from({length: _fuzzIntRange(0,20)}, () => _fuzzIntRange(32, 126))),
    String.fromCharCode(...Array.from({length: _fuzzIntRange(0,10)}, () => _fuzzIntRange(0, 0xFFFF))),
  ];
  return pools[_fuzzIntRange(0, pools.length - 1)];
}
function _fuzzAny(): unknown {
  const v = [_fuzzNum(), _fuzzStr(), _fuzzBool(), null, undefined, [], _fuzzObject()];
  return v[_fuzzIntRange(0, v.length - 1)];
}
function _fuzzObject(): unknown {
  const pools = [
    {},
    { preferences: null },
    { preferences: { timezone: _fuzzStr() } },
    { billing: null },
    { billing: { country: _fuzzStr() } },
    { contacts: null },
    { contacts: { support_email: _fuzzStr() } },
    { contacts: { emails: [_fuzzStr()] } },
    { contacts: { emails: [_fuzzStr(), _fuzzStr()] } },
    { profile: null, username: _fuzzStr() },
    { profile: { handle: _fuzzStr() }, username: _fuzzStr() },
    { titles: [_fuzzStr()] },
    { segments: [_fuzzStr()] },
    { plans: [null, _fuzzStr()] },
  ];
  return pools[_fuzzIntRange(0, pools.length - 1)];
}

const _EDGE_NUMS = [0, -0, Infinity, -Infinity, NaN, Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER + 1, 1e-300, 1e300];
const _EDGE_STRS = ["", "   ", "\0", "\uFFFF", "a".repeat(10000), "true", "null", "0", "-1", "\r\n", "\u200F", "\u200D", "${...}", "<script>"];
const _EDGE_STR_ARRAYS = [
  [],
  [""],
  ["   "],
  ["primary"],
  ["primary", ""],
  ["primary", "   "],
  ["primary", "Secondary"],
];
const _EDGE_OBJECTS = [
  {},
  { preferences: null },
  { preferences: { timezone: "   " } },
  { billing: null },
  { billing: { country: "   " } },
  { contacts: null },
  { contacts: { support_email: "   " } },
  { contacts: { emails: ["owner@example.com"] } },
  { contacts: { emails: ["owner@example.com", "   "] } },
  { profile: null, username: " Admin " },
  { profile: { handle: "   " }, username: " Admin " },
  { segments: [] },
  { segments: ["   ", "Growth"] },
  { plans: [] },
  { plans: [null, " team "] },
];
const _EDGE_SEMVER_OBJECTS = [
  { major: 0, minor: 0, patch: 0, prerelease: null },
  { major: 1, minor: 2, patch: 3, prerelease: [] },
  { major: 1, minor: 2, patch: 3, prerelease: ["alpha"] },
  { major: 0, minor: 1, patch: 0, prerelease: ["beta", "2"] },
];

function _edgeCasesFor(typeName: string): unknown[] {
  const m: Record<string, unknown[]> = {
    "number": _EDGE_NUMS,
    "string": _EDGE_STRS,
    "string_array": _EDGE_STR_ARRAYS,
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

function _cmpSign(value: unknown): number {
  if (typeof value !== "number" || Number.isNaN(value)) {
    throw new Error(`Comparator returned non-numeric value: ${JSON.stringify(value)}`);
  }
  if (value < 0) return -1;
  if (value > 0) return 1;
  return 0;
}

const _FUZZ_TEXT_LIMIT = 240;
function _clipText(value: unknown, limit = _FUZZ_TEXT_LIMIT): string {
  const text = typeof value === "string" ? value : String(value);
  if (text.length <= limit) return text;
  return `${text.slice(0, limit)}... [truncated ${text.length - limit} chars]`;
}

function _shortJson(value: unknown, limit = _FUZZ_TEXT_LIMIT): string {
  try {
    return _clipText(JSON.stringify(value), limit);
  } catch {
    return _clipText(value, limit);
  }
}

// Crash detection: real bugs vs intentional validation errors
function _isCrash(e: unknown): boolean {
  if (e instanceof TypeError) return true;
  if (e instanceof RangeError) return true;
  if (e instanceof ReferenceError) return true;
  if (e instanceof URIError) return true;
  // Property check violations (type, idempotency, consistency)
  if (e instanceof Error && (
    e.message.startsWith("Return type mismatch") ||
    e.message.startsWith("Not idempotent") ||
    e.message.startsWith("Inconsistent") ||
    e.message.startsWith("Not bounded") ||
    e.message.startsWith("Non-negative") ||
    e.message.startsWith("Blank string output") ||
    e.message.startsWith("Nullish string leak") ||
    e.message.startsWith("Not symmetric") ||
    e.message.startsWith("Comparator") ||
    e.message.startsWith("Roundtrip")
  )) return true;
  // Stack overflow
  if (e instanceof Error && e.message.includes("Maximum call stack")) return true;
  return false;
}

let _fuzzTotalFailures = 0;
const _fuzzResults: {function: string, input: string, error_type: string, message: string, severity: string}[] = [];
function _fuzzOne(
  name: string,
  iters: number,
  genArgs: () => unknown[],
  fn: (args: unknown[]) => unknown,
  expectedType: string | null,
  paramTypes: string[] = [],
  properties: string[] = [],
) {
  let pass = 0, reject = 0, crash = 0;
  let firstCrash = "";
  const allInputs: unknown[][] = [];
  for (let pi = 0; pi < paramTypes.length; pi++) {
    for (const ev of _edgeCasesFor(paramTypes[pi])) {
      const row = genArgs(); row[pi] = ev; allInputs.push(row);
    }
  }
  for (let i = 0; i < iters; i++) {
    allInputs.push(genArgs());
  }
  for (const args of allInputs) {
    try {
      const result = fn(args);
      pass++;
      // Type check
      if (expectedType !== null && typeof result !== expectedType) {
        throw new Error(`Return type mismatch: expected ${expectedType}, got ${typeof result}`);
      }
      // Consistency: same input → same output
      const result2 = fn(args);
      if (!_nanSafeEq(result, result2)) {
        throw new Error(`Inconsistent: ${JSON.stringify(result)} !== ${JSON.stringify(result2)}`);
      }
      // Idempotency: f(f(x)) === f(x)
      if (properties.includes("idempotent")) {
        const result3 = fn([result]);
        if (!_nanSafeEq(result, result3)) {
          throw new Error(`Not idempotent: ${JSON.stringify(result)} -> ${JSON.stringify(result3)}`);
        }
      }
      // Boundedness: len(f(x)) <= len(x)
      if (properties.includes("bounded")) {
        const inp = args[0];
        if ((typeof inp === "string" && typeof result === "string" && (result as string).length > (inp as string).length) ||
            (Array.isArray(inp) && Array.isArray(result) && (result as unknown[]).length > (inp as unknown[]).length)) {
          throw new Error(`Not bounded: output length ${(result as any).length} > input length ${(inp as any).length}`);
        }
      }
      // Non-negativity: f(x) >= 0
      if (properties.includes("nonneg") && typeof result === "number" && result < 0) {
        throw new Error(`Non-negative violation: ${result} < 0`);
      }
      // Non-empty string: identifier/display helpers should not emit blanks.
      if (properties.includes("nonempty_string") && typeof result === "string" && result.trim().length === 0) {
        throw new Error(`Blank string output: ${JSON.stringify(result)}`);
      }
      // Serialized/canonical string helpers should not emit nullish sentinel
      // text when the input structure contains null or undefined values.
      if (properties.includes("no_nullish_string") && typeof result === "string") {
        const firstArg = args[0];
        if (_containsNullish(firstArg) && _stringLeaksNullish(result)) {
          throw new Error(`Nullish string leak: ${JSON.stringify(result)}`);
        }
      }
      // Symmetry: f(a,b) == f(b,a)
      if (properties.includes("symmetric") && args.length === 2) {
        const resultRev = fn([args[1], args[0]]);
        if (!_nanSafeEq(result, resultRev)) {
          throw new Error(`Not symmetric: f(a,b)=${JSON.stringify(result)} != f(b,a)=${JSON.stringify(resultRev)}`);
        }
      }
      // Comparator contract: compare(a,a) == 0 and sign(compare(a,b)) == -sign(compare(b,a))
      if (properties.includes("comparator") && args.length === 2) {
        const selfCmp = fn([args[0], args[0]]);
        if (_cmpSign(selfCmp) !== 0) {
          throw new Error(`Comparator self-compare should be zero: ${JSON.stringify(selfCmp)}`);
        }
        const resultRev = fn([args[1], args[0]]);
        if (_cmpSign(result) !== -_cmpSign(resultRev)) {
          throw new Error(`Comparator antisymmetry violated: ${JSON.stringify(result)} vs ${JSON.stringify(resultRev)}`);
        }
      }
    } catch (e: unknown) {
      if (_isCrash(e)) {
        crash++;
        _fuzzResults.push({function: name, input: _shortJson(args),
          error_type: e instanceof Error ? e.constructor.name : "unknown",
          message: _clipText(e instanceof Error ? e.message : String(e)),
          severity: (e instanceof TypeError || e instanceof RangeError || e instanceof ReferenceError || e instanceof URIError) ? "crash" : "property_violation"});
        if (crash === 1) firstCrash = `  CRASH ${name}(${_shortJson(args)}): ${_clipText(e)}`;
      } else {
        reject++;
      }
    }
  }
  const total = pass + reject + crash;
  if (crash > 0) {
    console.log(`FUZZ ${name}: ${pass} passed, ${reject} rejected, ${crash} CRASHED (of ${total})`);
    console.log(firstCrash);
    _fuzzTotalFailures++;
  } else if (pass === 0) {
    console.log(`FUZZ ${name}: all ${total} inputs rejected (nothing tested)`);
    _fuzzTotalFailures++;
  } else {
    console.log(`FUZZ ${name}: ${pass} passed, ${reject} rejected (of ${total})`);
  }
}
"#;

const TYPESCRIPT_FUZZ_EPILOGUE: &str = r#"
if (_fuzzResults.length > 0) {
  console.log("__COURT_JESTER_FUZZ_JSON__");
  console.log(JSON.stringify(_fuzzResults));
}
if (_fuzzTotalFailures > 0) {
  console.error(`Fuzz testing failed: ${_fuzzTotalFailures} function(s) had failures`);
  process.exit(1);
} else {
  console.log("All fuzz tests passed");
}
"#;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

fn extract_generic_arg(t: &str) -> String {
    let start = match t.find('[').or_else(|| t.find('<')) {
        Some(i) => i,
        None => return t.to_string(),
    };
    let open = t.as_bytes()[start];
    let close: u8 = if open == b'[' { b']' } else { b'>' };
    if let Some(end) = t.rfind(close as char) {
        t[start + 1..end].trim().to_string()
    } else {
        t.to_string()
    }
}

fn extract_two_generic_args(t: &str) -> (String, String) {
    let inner = extract_generic_arg(t);
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '[' | '<' | '(' => depth += 1,
            ']' | '>' | ')' => depth -= 1,
            ',' if depth == 0 => {
                return (
                    inner[..i].trim().to_string(),
                    inner[i + 1..].trim().to_string(),
                );
            }
            _ => {}
        }
    }
    (inner, String::new())
}

fn split_ts_top_level<'a>(text: &'a str, separator: char) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' | '[' | '<' | '(' => depth += 1,
            '}' | ']' | '>' | ')' => depth -= 1,
            _ if ch == separator && depth == 0 => {
                let part = text[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }

    parts
}

fn python_edge_type_name(type_ann: Option<&str>) -> &'static str {
    match type_ann.map(|t| t.trim()) {
        Some("int") => "int",
        Some("float") => "float",
        Some("str") => "str",
        Some("bytes") => "bytes",
        Some("dict") | Some("Dict") => "dict",
        Some(t) if t.contains("dict") || t.contains("Dict") => "dict",
        _ => "",
    }
}

fn ts_edge_type_name(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> &'static str {
    let t = match type_ann {
        Some(t) => t.trim(),
        None => return "",
    };
    if let Some(resolved) = ts_resolve_alias_text(t, type_defs) {
        return ts_edge_type_name(Some(&resolved), type_defs);
    }

    match Some(t) {
        Some("number") => "number",
        Some("string") => "string",
        Some("string[]") | Some("Array<string>") => "string_array",
        Some(t) if is_string_array_like_type(t) => "string_array",
        Some(t) if t.starts_with("Record<") => "object",
        Some(t) if looks_like_ts_object_type(t) => "object",
        Some(t) if ts_class_def(t, type_defs).is_some() => "object",
        _ => "",
    }
}

fn is_string_array_like_type(type_ann: &str) -> bool {
    let trimmed = type_ann.trim();
    if trimmed.ends_with("[]") {
        let inner = trimmed.trim_end_matches("[]").trim();
        return is_string_like_union(inner);
    }
    if trimmed.starts_with("Array<") {
        let inner = extract_generic_arg(trimmed);
        return is_string_like_union(inner.trim());
    }
    false
}

fn is_string_like_union(type_ann: &str) -> bool {
    let branches = split_ts_top_level(type_ann, '|');
    if branches.is_empty() {
        return false;
    }
    branches
        .iter()
        .all(|branch| matches!(branch.trim(), "string" | "null" | "undefined"))
}

// ── Involution pair detection ───────────────────────────────────────────────

const INVOLUTION_PAIRS: &[(&str, &str)] = &[
    ("encode", "decode"),
    ("encrypt", "decrypt"),
    ("serialize", "deserialize"),
    ("pack", "unpack"),
    ("compress", "decompress"),
    ("marshal", "unmarshal"),
];

fn find_involution_pairs<'a>(
    analysis: &'a AnalysisResult,
) -> Vec<(&'a FunctionInfo, &'a FunctionInfo)> {
    let func_map: HashMap<String, &FunctionInfo> = analysis
        .functions
        .iter()
        .filter(|f| !f.name.starts_with('_') && !f.is_method && !f.is_nested)
        .map(|f| (f.name.to_lowercase(), f))
        .collect();

    let mut result = vec![];
    let mut seen: Vec<String> = vec![];

    for func in &analysis.functions {
        if func.name.starts_with('_') || func.is_method || func.is_nested {
            continue;
        }
        let params: Vec<_> = func
            .params
            .iter()
            .filter(|p| !p.name.starts_with('*'))
            .collect();
        if params.len() != 1 {
            continue;
        }

        let lower = func.name.to_lowercase();

        for (enc, dec) in INVOLUTION_PAIRS {
            if !lower.contains(enc) {
                continue;
            }
            let partner_lower = lower.replace(enc, dec);
            if let Some(partner) = func_map.get(&partner_lower) {
                let partner_params: Vec<_> = partner
                    .params
                    .iter()
                    .filter(|p| !p.name.starts_with('*'))
                    .collect();
                if partner_params.len() != 1 {
                    continue;
                }

                let key = if func.name < partner.name {
                    format!("{}+{}", func.name, partner.name)
                } else {
                    format!("{}+{}", partner.name, func.name)
                };
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);

                result.push((func, *partner));
            }
        }
    }

    result
}

fn synthesize_python_involution_checks(
    analysis: &AnalysisResult,
    type_defs: &HashMap<&str, &ClassInfo>,
) -> String {
    let pairs = find_involution_pairs(analysis);
    let mut code = String::new();

    for (enc, dec) in &pairs {
        let param = enc
            .params
            .iter()
            .find(|p| !p.name.starts_with('*'))
            .unwrap();
        let gen = python_generator(param.type_annotation.as_deref(), type_defs);

        code.push_str(&format!(
            r#"
# Involution roundtrip: {enc_name} <-> {dec_name}
for _i in range(30):
    _inv_input = {gen}
    try:
        _inv_encoded = {enc_name}(_inv_input)
        _inv_decoded = {dec_name}(_inv_encoded)
        assert _nan_eq(_inv_input, _inv_decoded), f"Roundtrip failed: {{repr(_inv_input)}} -> {{repr(_inv_encoded)}} -> {{repr(_inv_decoded)}}"
    except AssertionError:
        print(f"  ROUNDTRIP FAIL {enc_name} <-> {dec_name}: {{_short_repr(_inv_input)}} -> {{_short_repr(_inv_encoded)}} -> {{_short_repr(_inv_decoded)}}")
        _fuzz_failures += 1
        break
    except Exception as _e:
        if _is_crash(_e):
            print(f"  ROUNDTRIP CRASH {enc_name} <-> {dec_name}: {{type(_e).__name__}}: {{_clip_text(str(_e))}}")
            _fuzz_failures += 1
            break
"#,
            enc_name = enc.name,
            dec_name = dec.name,
        ));
    }

    code
}

fn synthesize_typescript_involution_checks(
    analysis: &AnalysisResult,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    let pairs = find_involution_pairs(analysis);
    let mut code = String::new();

    for (enc, dec) in &pairs {
        let param = enc
            .params
            .iter()
            .find(|p| !p.name.starts_with('*'))
            .unwrap();
        if !ts_type_is_fuzzable(param.type_annotation.as_deref(), type_defs) {
            continue;
        }
        let gen = ts_generator(param.type_annotation.as_deref(), type_defs);

        code.push_str("\n// Involution roundtrip: ");
        code.push_str(&enc.name);
        code.push_str(" <-> ");
        code.push_str(&dec.name);
        code.push_str("\n{\n  let _invFail = false;\n  for (let i = 0; i < 30; i++) {\n");
        code.push_str("    const input = ");
        code.push_str(&gen);
        code.push_str(";\n    try {\n");
        code.push_str("      const encoded = (");
        code.push_str(&enc.name);
        code.push_str(" as Function)(input);\n");
        code.push_str("      const decoded = (");
        code.push_str(&dec.name);
        code.push_str(" as Function)(encoded);\n");
        code.push_str("      if (!_nanSafeEq(input, decoded)) {\n");
        code.push_str("        console.log(`  ROUNDTRIP FAIL ");
        code.push_str(&enc.name);
        code.push_str(" <-> ");
        code.push_str(&dec.name);
        code.push_str(
            ": ${_shortJson(input)} -> ${_shortJson(encoded)} -> ${_shortJson(decoded)}`);\n",
        );
        code.push_str(
            "        _fuzzTotalFailures++;\n        _invFail = true;\n        break;\n      }\n",
        );
        code.push_str("    } catch (e: unknown) {\n");
        code.push_str("      if (_isCrash(e)) {\n");
        code.push_str("        console.log(`  ROUNDTRIP CRASH ");
        code.push_str(&enc.name);
        code.push_str(" <-> ");
        code.push_str(&dec.name);
        code.push_str(": ${_clipText(e)}`);\n");
        code.push_str("        _fuzzTotalFailures++;\n        _invFail = true;\n        break;\n      }\n    }\n  }\n");
        code.push_str("  if (!_invFail) console.log(\"FUZZ ");
        code.push_str(&enc.name);
        code.push_str(" <-> ");
        code.push_str(&dec.name);
        code.push_str(" roundtrip: passed\");\n}\n");
    }

    code
}
