use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::tools::domain;
use crate::types::*;

pub fn bind_argument_slots(
    signature: &[ParameterDomain],
    slots: PlannedArgumentSlots,
) -> Result<PlannedArguments, BindingError> {
    domain::bind_argument_slots(signature, slots)
}
/// Number of random inputs to generate per function.
const FUZZ_ITERATIONS: usize = 30;
const TS_TYPE_RECURSION_LIMIT: usize = 16;

/// Generate a property-based fuzz harness that tests each function with
/// many random inputs and checks:
/// 1. No crashes on any valid input
/// 2. Return type matches annotation (where checkable)
/// 3. Idempotency where applicable (string→string, etc.)
/// 4. Consistency for statically effect-free callables (same input → same output)
pub fn synthesize_calls(analysis: &AnalysisResult, language: &Language) -> String {
    synthesize_plan(analysis, language).code
}

pub fn synthesize_plan(analysis: &AnalysisResult, language: &Language) -> FuzzPlan {
    synthesize_plan_for(
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
    QueryStringSerializer,
    QueryStringParser,
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
    synthesize_plan_for(functions, classes, aliases, language).code
}

#[derive(Clone)]
struct PlannedSeedInput {
    arguments: PlannedArguments,
    contract_valid: bool,
    supports_type_fallback: bool,
}

type PlannedSeedInputs = HashMap<String, Vec<PlannedSeedInput>>;

fn finite_declared_literals(
    domain: &DomainNode,
    language: &Language,
) -> Option<Vec<DomainLiteral>> {
    match domain {
        DomainNode::Literal(values) if !values.is_empty() => Some(values.clone()),
        DomainNode::Boolean => Some(
            [false, true]
                .into_iter()
                .map(|value| {
                    crate::tools::domain::literal_from_json_value(
                        serde_json::json!(value),
                        language,
                    )
                })
                .collect(),
        ),
        DomainNode::Nullable(inner) => {
            let mut values = finite_declared_literals(inner, language)?;
            values.push(crate::tools::domain::literal_from_json_value(
                serde_json::Value::Null,
                language,
            ));
            Some(values)
        }
        DomainNode::Union(items) => Some(
            items
                .iter()
                .map(|item| finite_declared_literals(item, language))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect(),
        ),
        _ => None,
    }
}

pub fn synthesize_plan_for(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
) -> FuzzPlan {
    let plan =
        domain::build_verification_plan(functions, classes, aliases, language, &[], &[], &[]);
    synthesize_plan_for_verification(functions, classes, aliases, language, &plan)
}

/// Render one repository-derived verification plan.  All public synthesis
pub fn synthesize_plan_for_verification(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
    plan: &VerificationPlan,
) -> FuzzPlan {
    // Invocation selection must not erase declaration context. Nested functions
    // are not direct execution units, but their signatures belong to the factory
    // that returns them. The renderers exclude nested declarations from direct
    // calls and resolve them within their owning factory's source range.
    let synthesis_functions = functions
        .iter()
        .filter(|function| {
            function.is_nested
                || plan.surfaces.iter().any(|surface| {
                    surface.invocable
                        && surface.symbol == function.name
                        && surface.line == function.line
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut seed_inputs: PlannedSeedInputs = HashMap::new();
    let mut safe_dependency_surfaces = HashSet::new();
    for input in &plan.inputs {
        if input.classification == InputClassification::Invalid {
            continue;
        }
        if input
            .sources
            .iter()
            .any(|source| source.kind == DomainSourceKind::SafeDependencySubstitute)
        {
            safe_dependency_surfaces.insert(input.surface_id.clone());
        }
        let domains = plan
            .parameter_domains
            .iter()
            .filter(|domain| domain.surface_id == input.surface_id)
            .collect::<Vec<_>>();
        if matches!(language, Language::Python)
            && domain::classify_input(
                &input.arguments,
                &domains
                    .iter()
                    .map(|domain| (*domain).clone())
                    .collect::<Vec<_>>(),
            ) == InputClassification::Invalid
        {
            continue;
        }
        let closed_domain = !domains.is_empty() && domains.iter().all(|domain| domain.closed);
        // Predicate-derived examples explore branches, including rejection
        // branches. Their provenance is not an input-admission contract.
        let contract_valid = input.classification == InputClassification::Valid && closed_domain;
        seed_inputs
            .entry(
                input
                    .surface_id
                    .split(':')
                    .next()
                    .unwrap_or(&input.surface_id)
                    .to_string(),
            )
            .or_default()
            .push(PlannedSeedInput {
                arguments: input.arguments.clone(),
                contract_valid,
                supports_type_fallback: input.sources.iter().any(|source| {
                    matches!(
                        source.kind,
                        DomainSourceKind::ObservedCall
                            | DomainSourceKind::JsonFixture
                            | DomainSourceKind::ValidationGuard
                            | DomainSourceKind::SafeDependencySubstitute
                            | DomainSourceKind::CoverageCorpus
                            | DomainSourceKind::TypescriptEnum
                            | DomainSourceKind::TypescriptConstTuple
                    )
                }),
            });
    }
    let safe_dependency_surfaces = safe_dependency_surfaces.into_iter().collect::<Vec<_>>();
    synthesize_plan_legacy(
        &synthesis_functions,
        classes,
        aliases,
        language,
        &seed_inputs,
        &safe_dependency_surfaces,
    )
}

/// Compatibility constructor for callers that already extracted literal
/// seeds. It still builds the shared plan before rendering.
pub fn synthesize_plan_for_with_seeds(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
    seed_inputs: &HashMap<String, Vec<Vec<String>>>,
) -> FuzzPlan {
    let mut plan =
        domain::build_verification_plan(functions, classes, aliases, language, &[], &[], &[]);
    plan.inputs.clear();
    for (name, rows) in seed_inputs {
        if let Some(surface) = plan.surfaces.iter().find(|surface| surface.symbol == *name) {
            for row in rows {
                plan.inputs.push(PlannedInput {
                    surface_id: surface.id.clone(),
                    arguments: PlannedArguments {
                        positional: row
                            .iter()
                            .map(|expression| DomainLiteral {
                                expression: expression.clone(),
                                json_value: None,
                            })
                            .collect(),
                        named: std::collections::BTreeMap::new(),
                    },
                    classification: InputClassification::Valid,
                    sources: vec![],
                });
            }
        }
    }
    synthesize_plan_for_verification(functions, classes, aliases, language, &plan)
}

fn synthesize_plan_legacy(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
    seed_inputs: &PlannedSeedInputs,
    safe_dependency_surfaces: &[String],
) -> FuzzPlan {
    let class_defs: HashMap<&str, &ClassInfo> =
        classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let pseudo_analysis = AnalysisResult {
        functions: functions.to_vec(),
        classes: classes.to_vec(),
        aliases: aliases.to_vec(),
        imports: vec![],
        complexity: 1,
        cognitive_complexity: 0,
        max_nesting_depth: 0,
        complexity_breakdown: std::collections::BTreeMap::new(),
        parse_error: false,
        source_mode: SourceMode::for_language(language),
        parse_diagnostics: vec![],
    };
    match language {
        Language::Python => synthesize_python(
            &pseudo_analysis,
            &class_defs,
            seed_inputs,
            safe_dependency_surfaces,
        ),
        Language::TypeScript => synthesize_typescript(
            &pseudo_analysis,
            &build_ts_named_types(classes, aliases),
            seed_inputs,
            safe_dependency_surfaces,
        ),
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

fn is_synth_top_level_candidate(func: &FunctionInfo) -> bool {
    !func.name.starts_with('_')
        && !func.is_nested
        && (!func.is_method || func.invocation_target.is_some())
}

fn has_exported_surface(functions: &[FunctionInfo]) -> bool {
    functions
        .iter()
        .any(|func| is_synth_top_level_candidate(func) && func.is_exported)
}

fn callable_param_count(func: &FunctionInfo) -> usize {
    func.params
        .iter()
        .filter(|param| !param.is_variadic())
        .count()
}

fn likely_intentionally_nondeterministic(func: &FunctionInfo) -> bool {
    if callable_param_count(func) != 0 {
        return false;
    }

    let lower = func
        .name
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .collect::<String>()
        .to_lowercase();

    lower == "now"
        || lower.ends_with("now")
        || [
            "random",
            "uuid",
            "guid",
            "nonce",
            "token",
            "timestamp",
            "correlationid",
            "requestid",
            "traceid",
            "spanid",
            "sessionid",
            "messageid",
            "clock",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn supports_implicit_consistency(func: &FunctionInfo) -> bool {
    func.effects.is_empty()
        && func.returned_callables.is_empty()
        && !likely_intentionally_nondeterministic(func)
}

const SIMPLE_HELPER_NAME_CUES: &[&str] = &[
    "parse",
    "read",
    "decode",
    "normalize",
    "trim",
    "split",
    "token",
    "header",
    "query",
    "param",
    "path",
    "slug",
];

fn likely_simple_helper(name: &str) -> bool {
    let lower = name.to_lowercase();
    SIMPLE_HELPER_NAME_CUES
        .iter()
        .any(|cue| lower.contains(cue))
}

fn synth_candidate_functions(functions: &[FunctionInfo]) -> Vec<&FunctionInfo> {
    functions
        .iter()
        .filter(|func| is_synth_top_level_candidate(func))
        .collect()
}

fn coverage_entry(
    func: &FunctionInfo,
    status: FuzzFunctionStatus,
    reason: Option<String>,
) -> FuzzFunctionCoverage {
    FuzzFunctionCoverage {
        function: func.name.clone(),
        line: func.line,
        end_line: func.end_line,
        status,
        required: func.is_exported,
        invocation_path: InvocationPath::Direct,
        is_exported: func.is_exported,
        reason,
    }
}

fn factory_callable_declaration<'a>(
    analysis: &'a AnalysisResult,
    factory: &FunctionInfo,
    callable: &str,
) -> Option<&'a FunctionInfo> {
    analysis
        .functions
        .iter()
        .filter(|candidate| {
            candidate.is_nested
                && candidate.name == callable
                && candidate.line >= factory.line
                && candidate.end_line <= factory.end_line
        })
        .max_by_key(|candidate| candidate.line)
}

fn known_factory_callable<'a>(
    analysis: &'a AnalysisResult,
    factory: &FunctionInfo,
    callable: &str,
    type_defs: &TsNamedTypes<'_>,
) -> Option<&'a FunctionInfo> {
    factory_callable_declaration(analysis, factory, callable).filter(|candidate| {
        let params = candidate
            .params
            .iter()
            .filter(|param| !param.is_variadic())
            .collect::<Vec<_>>();
        params.iter().all(|param| param.type_annotation.is_some())
            && ts_params_are_fuzzable(candidate, &params, type_defs)
    })
}

fn factory_callable_coverage(
    analysis: &AnalysisResult,
    selected_functions: &[&FunctionInfo],
    type_defs: &TsNamedTypes<'_>,
) -> Vec<FuzzFunctionCoverage> {
    let mut coverage = Vec::new();
    for func in selected_functions {
        if func.returned_callables.is_empty() {
            continue;
        }
        for callable in &func.returned_callables {
            // Match by returned property name inside the factory source range.
            // Prefer the declaration nearest the returned object so shadowed
            // shorthand names and object methods retain a stable source line.
            let nested = factory_callable_declaration(analysis, func, callable);
            let known = known_factory_callable(analysis, func, callable, type_defs).is_some();
            let status = if known {
                FuzzFunctionStatus::CheckedViaFactory
            } else {
                FuzzFunctionStatus::ReachedViaFactory
            };
            let reason = if known {
                Some(format!(
                    "planned typed invocation through factory return surface of {}; runtime target proof is required",
                    func.name
                ))
            } else {
                Some(format!(
                    "factory returned callable {} but its signature/domain is unknown",
                    callable
                ))
            };
            coverage.push(FuzzFunctionCoverage {
                function: format!("{}().{}", func.name, callable),
                line: nested.map(|entry| entry.line).unwrap_or(func.line),
                end_line: nested.map(|entry| entry.end_line).unwrap_or(func.end_line),
                status,
                required: func.is_exported,
                invocation_path: InvocationPath::Factory {
                    factory: func.name.clone(),
                    callable: callable.clone(),
                },
                is_exported: func.is_exported,
                reason,
            });
        }
    }
    coverage
}

fn rejection_domains(
    func: &FunctionInfo,
    analysis: &AnalysisResult,
    language: &Language,
) -> String {
    let domains = func
        .params
        .iter()
        .filter(|param| !param.is_variadic())
        .enumerate()
        .filter_map(|(index, param)| {
            let annotation = param
                .type_annotation
                .as_deref()
                .map(|value| func.resolved_type_annotation(value));
            let domain = domain::domain_for_annotation(
                annotation.as_deref(),
                &analysis.aliases,
                &analysis.classes,
                language,
            );
            let mut values = finite_declared_literals(&domain, language)?;
            if matches!(language, Language::TypeScript) && param.optional {
                values.push(DomainLiteral {
                    expression: "undefined".into(),
                    json_value: None,
                });
            }
            let values = values
                .iter()
                .map(|value| value.expression.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Some(match language {
                Language::Python => format!("({index}, [{values}])"),
                Language::TypeScript => format!("[{index}, [{values}]]"),
            })
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{domains}]")
}

fn python_seed_rows_expr(
    func: &FunctionInfo,
    seed_inputs: &PlannedSeedInputs,
    contract_only: bool,
) -> String {
    let Some(rows) = seed_inputs.get(&func.name) else {
        return "[]".to_string();
    };
    let fixed_params = func
        .params
        .iter()
        .filter(|param| !param.is_variadic())
        .collect::<Vec<_>>();
    let fixed_names = fixed_params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<HashSet<_>>();
    let keyword_variadic = func.params.iter().any(ParamInfo::is_keyword_variadic);
    format!(
        "[{}]",
        rows.iter()
            .filter(|row| !contract_only || row.contract_valid)
            .map(|row| {
                let row = &row.arguments;
                let mut values = Vec::new();
                let mut positional_index = 0usize;
                for param in &fixed_params {
                    if param.keyword_only {
                        if let Some(value) = row.named.get(&param.name) {
                            values.push(value.expression.clone());
                        } else {
                            values.push("None".into());
                        }
                    } else if let Some(value) = row.positional.get(positional_index) {
                        values.push(value.expression.clone());
                        positional_index += 1;
                    } else {
                        values.push("None".into());
                        positional_index += 1;
                    }
                }
                if func.params.iter().any(ParamInfo::is_positional_variadic) {
                    values.extend(
                        row.positional[positional_index..]
                            .iter()
                            .map(|value| value.expression.clone()),
                    );
                }
                if keyword_variadic {
                    let kwargs = row
                        .named
                        .iter()
                        .filter(|(name, _)| !fixed_names.contains(name.as_str()))
                        .map(|(name, value)| {
                            format!(
                                "{}: {}",
                                serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into()),
                                value.expression
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    values.push(format!("{{{kwargs}}}"));
                }
                format!("[{}]", values.join(", "))
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn ts_seed_rows(func: &FunctionInfo, seed_inputs: &PlannedSeedInputs) -> String {
    let Some(rows) = seed_inputs.get(&func.name) else {
        return String::new();
    };
    rows.iter()
        .map(|row| {
            format!(
                "{{ args: [{}], contractValid: {} }}",
                row.arguments
                    .positional
                    .iter()
                    .map(|item| item.expression.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                row.contract_valid,
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ts_default_omission_rows(params: &[&ParamInfo], generated_parts: &[String]) -> String {
    params
        .iter()
        .enumerate()
        .filter(|(index, param)| {
            param.default_value.is_some() && params[index + 1..].iter().all(|later| later.optional)
        })
        .map(|(index, _)| format!("[{}]", generated_parts[..index].join(", ")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn has_noncheckable_python_zero_arg_return_contract(func: &FunctionInfo) -> bool {
    let return_type = func.return_type.as_deref().unwrap_or("").trim();
    !return_type.is_empty() && !matches!(return_type, "str" | "int" | "float" | "bool" | "bytes")
}

fn has_noncheckable_ts_zero_arg_return_contract(func: &FunctionInfo) -> bool {
    let return_type = func.return_type.as_deref().unwrap_or("").trim();
    !return_type.is_empty() && !matches!(return_type, "string" | "number" | "boolean")
}

fn has_nested_children(func: &FunctionInfo, all_functions: &[FunctionInfo]) -> bool {
    all_functions.iter().any(|candidate| {
        candidate.is_nested && candidate.line >= func.line && candidate.end_line <= func.end_line
    })
}

fn python_type_is_simple_helper_input(
    type_ann: Option<&str>,
    type_defs: &HashMap<&str, &ClassInfo>,
) -> bool {
    let t = match type_ann {
        Some(t) => t.trim(),
        None => return true,
    };

    match t {
        "int" | "float" | "str" | "bool" | "bytes" | "Any" | "datetime" | "date" => true,
        _ if python_literal_choice_exprs(t).is_some() => true,
        _ if is_python_mapping_type(t) => true,
        _ if starts_with_any(t, &["Optional["]) => {
            let inner = extract_generic_arg(t);
            python_type_is_simple_helper_input(Some(&inner), type_defs)
        }
        _ if t.contains(" | ") => t
            .split('|')
            .map(str::trim)
            .filter(|branch| *branch != "None")
            .all(|branch| python_type_is_simple_helper_input(Some(branch), type_defs)),
        _ if is_python_sequence_type(t)
            || starts_with_any(t, &["tuple[", "Tuple[", "set[", "Set["]) =>
        {
            let inner = extract_generic_arg(t);
            split_top_level_args(&inner, ',')
                .into_iter()
                .filter(|branch| *branch != "...")
                .all(|branch| python_type_is_simple_helper_input(Some(branch), type_defs))
        }
        _ if type_defs.contains_key(t) => false,
        _ => false,
    }
}

fn should_fuzz_python_helper(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    type_defs: &HashMap<&str, &ClassInfo>,
    has_exported: bool,
) -> bool {
    if func.is_exported || !has_exported {
        return true;
    }

    if has_http_static_file_middleware_contract(func) {
        return true;
    }

    params.len() == 1
        && likely_simple_helper(&func.name)
        && python_type_is_simple_helper_input(params[0].type_annotation.as_deref(), type_defs)
}

fn ts_type_is_simple_helper_input(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> bool {
    let t = match type_ann {
        Some(t) => ts_effective_type(t.trim(), type_defs),
        None => return true,
    };
    let trimmed = t.trim();

    let union_branches = split_ts_top_level(trimmed, '|');
    if union_branches.len() > 1 {
        return union_branches
            .iter()
            .map(|branch| branch.trim())
            .filter(|branch| !matches!(*branch, "null" | "undefined"))
            .all(|branch| ts_type_is_simple_helper_input(Some(branch), type_defs));
    }

    match trimmed {
        "number" | "string" | "boolean" | "URL" | "URLSearchParams" | "Headers" | "Request"
        | "Response" => true,
        _ if trimmed.ends_with("[]") => {
            let inner = trimmed.trim_end_matches("[]").trim();
            matches!(inner, "string" | "number" | "boolean")
        }
        _ if trimmed.starts_with("Array<") => {
            let inner = extract_generic_arg(trimmed);
            matches!(inner.trim(), "string" | "number" | "boolean")
        }
        _ if trimmed.starts_with("ReadonlyArray<") => {
            let inner = extract_generic_arg(trimmed);
            matches!(inner.trim(), "string" | "number" | "boolean")
        }
        _ if trimmed.starts_with("Set<") || trimmed.starts_with("ReadonlySet<") => {
            let inner = extract_generic_arg(trimmed);
            matches!(inner.trim(), "string" | "number" | "boolean")
        }
        _ if trimmed.starts_with("Map<") || trimmed.starts_with("ReadonlyMap<") => {
            let (key, value) = extract_two_generic_args(trimmed);
            matches!(key.trim(), "string" | "number" | "boolean")
                && matches!(value.trim(), "string" | "number" | "boolean")
        }
        _ if trimmed.starts_with("Record<") => true,
        _ if looks_like_ts_object_type(trimmed) => true,
        _ if ts_class_def(trimmed, type_defs).is_some() => true,
        _ => false,
    }
}

fn should_fuzz_ts_helper(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    type_defs: &TsNamedTypes<'_>,
    has_exported: bool,
) -> bool {
    if func.is_exported || !has_exported {
        return true;
    }

    if has_http_static_file_middleware_contract(func) {
        return true;
    }

    params.len() == 1
        && likely_simple_helper(&func.name)
        && ts_type_is_simple_helper_input(params[0].type_annotation.as_deref(), type_defs)
}

// ── Python fuzz harness ─────────────────────────────────────────────────────

fn unsafe_dependency_reason(
    params: &[&ParamInfo],
    language: &Language,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
) -> Option<String> {
    params.iter().find_map(|param| {
        if !param.optional && param.default_value.is_none() {
            return None;
        }
        if !domain::is_dependency_shaped(param, classes, aliases) {
            return None;
        }
        let reason = domain::safe_dependency_substitute(param, language, classes, aliases).err()?;
        Some(unsafe_default_dependency_reason(&param.name, reason))
    })
}

fn synthesize_python(
    analysis: &AnalysisResult,
    type_defs: &HashMap<&str, &ClassInfo>,
    seed_inputs: &PlannedSeedInputs,
    safe_dependency_surfaces: &[String],
) -> FuzzPlan {
    let mut code = String::new();
    let mut coverage = Vec::new();
    let has_exported = has_exported_surface(&analysis.functions);

    // Embed a tiny random generator (no imports needed)
    code.push_str(PYTHON_FUZZ_PRELUDE);
    let safe_surfaces =
        serde_json::to_string(safe_dependency_surfaces).unwrap_or_else(|_| "[]".into());
    code.push_str(&format!(
        "_CJ_SAFE_DEPENDENCY_SURFACES = set({safe_surfaces})\n"
    ));

    let mut any_synthesized = false;
    let mut selected_functions = Vec::new();

    for func in synth_candidate_functions(&analysis.functions) {
        let callable_params: Vec<&ParamInfo> =
            func.params.iter().filter(|p| !p.is_variadic()).collect();
        let positional_variadic = func.params.iter().find(|p| p.is_positional_variadic());
        let keyword_variadic = func.params.iter().find(|p| p.is_keyword_variadic());
        let has_nested = has_nested_children(func, &analysis.functions);
        let has_seed_rows = seed_inputs
            .get(&func.name)
            .is_some_and(|rows| !rows.is_empty());
        if callable_params.is_empty()
            && positional_variadic.is_none()
            && keyword_variadic.is_none()
            && !has_nested
            && has_noncheckable_python_zero_arg_return_contract(func)
        {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedNoFuzzableSurface,
                Some(
                    "zero-argument function has no meaningful parameter surface or stable return contract to fuzz".into(),
                ),
            ));
            continue;
        }
        if let Some(reason) = unsafe_dependency_reason(
            &callable_params,
            &Language::Python,
            &analysis.classes,
            &analysis.aliases,
        ) {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(reason),
            ));
            continue;
        }

        if callable_params
            .iter()
            .any(|param| param.type_annotation.is_none())
            && !has_seed_rows
        {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(
                    "one or more Python parameters are untyped and no seed/domain examples were found"
                        .into(),
                ),
            ));
            continue;
        }

        // Check if we can generate values for all params
        let generators: Vec<String> = callable_params
            .iter()
            .map(|p| python_generator(p.type_annotation.as_deref(), type_defs))
            .collect();
        if generators.iter().any(|gen| gen == "_fuzz_none()") {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some("one or more parameters use unsupported Python types".into()),
            ));
            continue;
        }

        if !should_fuzz_python_helper(func, &callable_params, type_defs, has_exported) {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedInternalHelper,
                Some("non-exported helper is deferred to the exported API surface".into()),
            ));
            continue;
        }
        coverage.push(coverage_entry(
            func,
            FuzzFunctionStatus::CheckedDirect,
            None,
        ));
        selected_functions.push(func);
        let mut generated_parts = generators.clone();
        let rest_start = callable_params.len();
        if let Some(rest) = positional_variadic {
            let item_generator = python_generator(rest.type_annotation.as_deref(), type_defs);
            generated_parts.push(format!(
                "*[{} for _ in range(_fuzz_int_range(0, 2))]",
                item_generator
            ));
        }
        if keyword_variadic.is_some() {
            generated_parts.push("{\"__court_jester_kw\": _fuzz_str()}".into());
        }
        let gen_list = generated_parts.join(", ");
        let mut call_args: Vec<String> = callable_params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if p.keyword_only {
                    format!("{}=_call_args[{}]", p.name, i)
                } else {
                    format!("_call_args[{}]", i)
                }
            })
            .collect();
        if positional_variadic.is_some() {
            if keyword_variadic.is_some() {
                call_args.push(format!("*_call_args[{}:-1]", rest_start));
            } else {
                call_args.push(format!("*_call_args[{}:]", rest_start));
            }
        }
        if keyword_variadic.is_some() {
            call_args.push("**_call_args[-1]".into());
        }
        let call = call_args.join(", ");
        let declared_properties = func
            .declared_properties
            .iter()
            .map(|property| format!("\"{}\"", property.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(", ");
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

        let evaluator_source = python_evaluator_source(func, &callable_params, type_defs, &call);
        code.push_str(&format!(
            r#"
{evaluator_source_code}
_cj_evaluator_source = {evaluator_source}
_all_inputs = []
_seed_rows = {seed_rows}
_contract_rows = {contract_rows}
_rejection_domains = {rejection_domains}
_corpus = []
_behavior_signatures = set()
{edge_case_setup}
_all_inputs.extend(_seed_rows)
for _ in range({FUZZ_ITERATIONS}):
    if _seed_rows:
        _all_inputs.append(_fuzz_seed_row(_seed_rows))
    else:
        _all_inputs.append([{gen_list}])
_max_campaign_inputs = len(_all_inputs) + {FUZZ_ITERATIONS}
_pass = 0
_reject = 0
_crash = 0
_unknown = 0
for _iteration, _args in enumerate(_all_inputs):
    _contract_target_exception = False
    _checking_properties = False
    try:
        _target_entered("{name}:{line}", _iteration)
        try:
            _result = _cj_invoke(_args)
        except Exception:
            _contract_target_exception = any(_same_input(_args, _row) for _row in _contract_rows)
            raise
        _checking_properties = True
        _cj_evaluate(_args, _result)
        _pass += 1
        if _retain_corpus_input(_corpus, _behavior_signatures, _behavior_signature("passed", _result), _args) and len(_all_inputs) < _max_campaign_inputs:
            _all_inputs.append(_mutate_corpus_row(_args))
        _cj_unit_completed("{name}:{line}", _iteration, "passed")
    except Exception as _e:
        _outside_contract = _outside_closed_domain(_args, _rejection_domains)
        _target_exception = not _outside_contract and (_contract_target_exception or _is_crash(_e))
        if _retain_corpus_input(_corpus, _behavior_signatures, _behavior_signature("crash" if _target_exception else "rejected", _e), _args) and len(_all_inputs) < _max_campaign_inputs:
            _all_inputs.append(_mutate_corpus_row(_args))
        if _target_exception:
            _crash += 1
            _cj_unit_completed("{name}:{line}", _iteration, "target_exception")
            _emit_error("{name}", _args, _e, [{declared_properties}], lambda _candidate: (not _contract_target_exception or any(_same_input(_candidate, _row) for _row in _contract_rows)) and _reproduces_python(_candidate, _e, lambda: _cj_run(_candidate) if _checking_properties else _cj_invoke(_candidate)), invocation_path="direct", target_exception=_contract_target_exception, replay_source=_cj_evaluator_source, evaluate=_checking_properties)
            if _crash == 1:
                print(f"  CRASH {name}({{_short_repr(_args)}}): {{type(_e).__name__}}: {{_clip_text(str(_e))}}")
        elif _outside_contract:
            _reject += 1
            _cj_unit_completed("{name}:{line}", _iteration, "rejected")
        else:
            _unknown += 1
            _cj_unit_completed("{name}:{line}", _iteration, "unclassified_exception")
            _emit_uncertain_exception("{name}", _args, _e, replay_source=_cj_evaluator_source, evaluate=_checking_properties)
_CJ_CORPORA["{name}:{line}"] = _corpus[:64]
{query_string_semantic_check}
{pep440_version_ordering_check}
{pep440_specifier_membership_check}
{pep440_filter_prerelease_check}
{cookie_value_quote_check}
{cookie_header_quote_check}
_total = _pass + _reject + _crash + _unknown
if _crash > 0:
    print(f"FUZZ {name}: {{_pass}} passed, {{_reject}} rejected, {{_crash}} CRASHED (of {{_total}})")
    _fuzz_failures += 1
elif _unknown > 0:
    print(f"FUZZ {name}: {{_pass}} passed, {{_reject}} rejected, 0 CRASHED, {{_unknown}} unclassified (of {{_total}})")
elif _pass == 0:
    print(f"FUZZ {name}: all {{_total}} inputs rejected (nothing tested)")
    _fuzz_failures += 1
else:
    print(f"FUZZ {name}: {{_pass}} passed, {{_reject}} rejected (of {{_total}})")
"#,
            name = func.name,
            evaluator_source = serde_json::to_string(&evaluator_source).expect("Python evaluator source is serializable"),
            evaluator_source_code = evaluator_source,
            declared_properties = declared_properties,
            edge_case_setup = edge_case_setup,
            seed_rows = python_seed_rows_expr(func, seed_inputs, false),
            contract_rows = python_seed_rows_expr(func, seed_inputs, true),
            rejection_domains = rejection_domains(func, analysis, &Language::Python),
            line = func.line,
            query_string_semantic_check =
                python_query_string_semantic_check(func, &callable_params),
            pep440_version_ordering_check =
                python_pep440_version_ordering_check(func, &callable_params),
            pep440_specifier_membership_check =
                python_pep440_specifier_membership_check(func, &callable_params),
            pep440_filter_prerelease_check =
                python_pep440_filter_prerelease_check(func, &callable_params),
            cookie_value_quote_check = python_cookie_value_quote_check(func, &callable_params),
            cookie_header_quote_check = python_cookie_header_quote_check(func, &callable_params),
        ));

        any_synthesized = true;
    }

    if !any_synthesized {
        return FuzzPlan {
            code: String::new(),
            coverage,
        };
    }

    // Stateful factory campaign: create one instance, then execute a generated
    // sequence that covers every known action and repeats random actions.
    for func in selected_functions {
        if func.returned_callables.is_empty() {
            continue;
        }
        let callable_params: Vec<&ParamInfo> =
            func.params.iter().filter(|p| !p.is_variadic()).collect();
        let factory_positional = callable_params
            .iter()
            .filter(|param| !param.keyword_only)
            .map(|param| python_generator(param.type_annotation.as_deref(), type_defs))
            .collect::<Vec<_>>()
            .join(", ");
        let factory_keyword = callable_params
            .iter()
            .filter(|param| param.keyword_only)
            .map(|param| {
                format!(
                    "{:?}: {}",
                    param.name,
                    python_generator(param.type_annotation.as_deref(), type_defs)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let known_specs = func
            .returned_callables
            .iter()
            .filter_map(|callable| {
                let declaration = factory_callable_declaration(analysis, func, callable)?;
                if declaration.params.iter().any(|param| {
                    param.is_variadic()
                        || (param.type_annotation.is_none() && param.default_value.is_none())
                }) {
                    return None;
                }
                let mut positional = Vec::new();
                let mut keyword = Vec::new();
                for param in declaration.params.iter().filter(|param| !param.is_variadic()) {
                    let generated =
                        python_generator(param.type_annotation.as_deref(), type_defs);
                    if generated == "_fuzz_none()"
                        && param.type_annotation.as_deref() != Some("None")
                        && param.default_value.is_none()
                    {
                        return None;
                    }
                    if param.keyword_only {
                        let key = serde_json::to_string(&param.name).ok()?;
                        keyword.push(format!("{key}: {generated}"));
                    } else {
                        positional.push(generated);
                    }
                }
                let key = serde_json::to_string(callable).ok()?;
                let surface =
                    serde_json::to_string(&format!("{}().{}", func.name, callable)).ok()?;
                Some(format!(
                    "{key}: {{\"surface\": {surface}, \"line\": {line}, \"args\": lambda: [{args}], \"kwargs\": lambda: {{{kwargs}}}}}",
                    line = declaration.line,
                    args = positional.join(", "),
                    kwargs = keyword.join(", "),
                ))
            })
            .collect::<Vec<_>>();
        if known_specs.is_empty() {
            continue;
        }
        let known_specs_expr = format!("{{{}}}", known_specs.join(", "));
        let nested_names = func.returned_callables.join(", ");
        code.push_str(&format!(
            r#"
# Stateful factory action-sequence campaign: {name}
_factory_pass = 0
_factory_crash = 0
_factory_unknown = 0
_known_factory_callables = {known_specs_expr}
_action_keys = list(_known_factory_callables)
for _fi in range({iters}):
    _active_factory_callable = "unknown"
    _active_factory_surface = "{name} (factory)"
    _active_factory_line = {func_line}
    _active_factory_args = []
    _active_factory_kwargs = {{}}
    _action_trace = []
    _active_factory_unit = None
    _factory_setup = {{"args": [], "kwargs": {{}}}}
    _factory_phase = "arguments"
    try:
        _factory_setup = {{"args": [{factory_positional}], "kwargs": {{{factory_keyword}}}}}
        _factory_phase = "factory"
        _factory_result = {name}(*_copy.deepcopy(_factory_setup["args"]), **_copy.deepcopy(_factory_setup["kwargs"]))
        _action_plan = list(_action_keys)
        for _ in range(_fuzz_int_range(2, 5)):
            _action_plan.append(_rng.choice(_action_keys))
        for _step_index, _action in enumerate(_action_plan):
            _spec = _known_factory_callables[_action]
            _active_factory_callable = _action
            _active_factory_surface = _spec["surface"]
            _active_factory_line = _spec["line"]
            _factory_phase = "resolve:" + str(_step_index)
            _action_trace.append({{"action": _action, "args": [], "kwargs": {{}}}})
            _candidate = _resolve_factory_action(_factory_result, _action, len(_action_keys) == 1)
            _action_trace[-1]["callable"] = callable(_candidate)
            if not callable(_candidate):
                continue
            _factory_phase = "arguments:" + str(_step_index)
            _active_factory_args = _spec["args"]()
            _active_factory_kwargs = _spec["kwargs"]()
            _action_trace[-1].update({{"args": _copy.deepcopy(_active_factory_args), "kwargs": _copy.deepcopy(_active_factory_kwargs)}})
            _active_factory_unit = _fi * (len(_action_keys) + 5) + _step_index
            _target_entered(_active_factory_surface, _active_factory_unit)
            _factory_phase = "action:" + str(_step_index)
            _candidate(*_copy.deepcopy(_active_factory_args), **_copy.deepcopy(_active_factory_kwargs))
            _cj_unit_completed(_active_factory_surface, _active_factory_unit, "passed")
            _active_factory_unit = None
        _factory_pass += 1
    except Exception as _e:
        _factory_snippet = _factory_replay_snippet("{name}", _factory_setup, _action_trace, len(_action_keys) == 1, _factory_phase, _e)
        _factory_case = [{{"factory": _factory_setup, "actions": _action_trace}}]
        if _active_factory_unit is not None:
            _cj_unit_completed(_active_factory_surface, _active_factory_unit, "target_exception" if _is_crash(_e) else "unclassified_exception")
        if _is_crash(_e):
            _factory_crash += 1
            _emit_finding(_active_factory_surface, _factory_case, _e, "crash", "runtime_contract", "observed_call", "high", "exception", case_label=_clip_text(_action_trace), invocation_path={{"factory": {{"factory": "{name}", "callable": _active_factory_callable}}}}, replay_snippet=_factory_snippet, repro_kind="semantic_case")
            if _factory_crash == 1:
                print(f"  CRASH {{_active_factory_surface}} after actions {{_clip_text(_action_trace)}}: {{type(_e).__name__}}: {{_clip_text(str(_e))}}")
        else:
            _factory_unknown += 1
            _emit_uncertain_exception(_active_factory_surface, _factory_case, _e, case_label=_clip_text(_action_trace), invocation_path={{"factory": {{"factory": "{name}", "callable": _active_factory_callable}}}}, replay_snippet=_factory_snippet, repro_kind="semantic_case")
_factory_total = _factory_pass + _factory_crash + _factory_unknown
if _factory_crash > 0:
    print(f"FUZZ {name} (factory state machine): {{_factory_pass}} passed, {{_factory_crash}} CRASHED (of {{_factory_total}}) [actions: {nested_names}]")
    _fuzz_failures += 1
else:
    print(f"FUZZ {name} (factory state machine): {{_factory_pass}} passed, 0 rejected, 0 CRASHED, {{_factory_unknown}} unclassified (of {{_factory_total}}) [actions: {nested_names}]")
"#,
            func_line = func.line,
            name = func.name,
            known_specs_expr = known_specs_expr,
            factory_positional = factory_positional,
            factory_keyword = factory_keyword,
            iters = FUZZ_ITERATIONS,
            nested_names = nested_names,
        ));
    }

    // Involution roundtrip checks
    code.push_str(&synthesize_python_involution_checks(analysis, type_defs));

    code.push_str(PYTHON_FUZZ_EPILOGUE);
    FuzzPlan { code, coverage }
}
fn python_class_is_runtime_instantiable(class: &ClassInfo) -> bool {
    !class
        .bases
        .iter()
        .any(|base| base.split('[').next().unwrap_or(base).rsplit('.').next() == Some("Protocol"))
}

fn python_generator(type_ann: Option<&str>, type_defs: &HashMap<&str, &ClassInfo>) -> String {
    let t = match type_ann {
        Some(t) => t.trim(),
        // No type annotation: generate a mix of types instead of just None
        None => return "_fuzz_any()".to_string(),
    };

    if let Some(choices) = python_literal_choice_exprs(t) {
        return literal_choice_expr(&choices, "_fuzz_int_range");
    }

    match t {
        "int" => "_fuzz_int()".into(),
        "float" => "_fuzz_float()".into(),
        "str" => "_fuzz_str()".into(),
        "bool" => "_fuzz_bool()".into(),
        "bytes" => "_fuzz_bytes()".into(),
        "Any" => "_fuzz_any()".into(),
        _ if is_python_mapping_type(t) && !t.contains('[') => "_fuzz_dict()".into(),
        _ if is_python_sequence_type(t) => {
            let inner = extract_generic_arg(t);
            let gen = python_generator(Some(&inner), type_defs);
            format!("[{gen} for _ in range(_fuzz_int_range(0, 5))]")
        }
        _ if is_python_mapping_type(t) => {
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
            let item_types = split_top_level_args(&inner, ',');
            if item_types.len() == 2 && item_types[1] == "..." {
                let gen = python_generator(Some(item_types[0]), type_defs);
                format!("tuple({gen} for _ in range(_fuzz_int_range(0, 5)))")
            } else {
                let values = item_types
                    .into_iter()
                    .map(|item| python_generator(Some(item), type_defs))
                    .collect::<Vec<_>>();
                let trailing_comma = if values.len() == 1 { "," } else { "" };
                format!("({}{trailing_comma})", values.join(", "))
            }
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
            if !python_class_is_runtime_instantiable(class) {
                "_fuzz_none()".into()
            } else if class.fields.is_empty() {
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

fn python_evaluator_source(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    type_defs: &HashMap<&str, &ClassInfo>,
    call: &str,
) -> String {
    let body = [
        python_type_check(func.return_type.as_deref().unwrap_or(""), type_defs),
        python_idempotency_check(func, params, type_defs),
        python_consistency_check(func),
        python_boundedness_check(func, params),
        python_nonneg_check(func),
        python_clamped_check(func, params),
        python_sorted_check(func, params),
        python_permutation_check(func, params),
        python_palindrome_check(func),
        python_nullish_string_leak_check(func, params),
        python_comparator_check(func, params),
        python_symmetry_check(func, params),
        python_metamorphic_checks(func, params),
    ]
    .join("\n");
    let body = body
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    format!("def _cj_invoke(_args):\n    _call_args = _copy.deepcopy(_args)\n    return _materialize_if_iterator({}({call}))\ndef _cj_evaluate(_args, _result):\n    pass\n{body}\ndef _cj_run(_args):\n    _result = _cj_invoke(_args)\n    _cj_evaluate(_args, _result)\n    return _result\n", func.name)
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
    format!("        _cj_require(\"return_type\", {check}, lambda: f\"Return type mismatch: got {{type(_result).__name__}}\")")
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

fn likely_query_string_semantics(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("query")
        && [
            "canonical",
            "canonicalize",
            "normalise",
            "normalize",
            "serialize",
            "serialise",
            "encode",
            "stringify",
            "parse",
            "decode",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn feature_flag_key_from_function_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix("Enabled")?;
    let mut chars = stem.chars();
    let first = chars.next()?;
    let mut key = first.to_lowercase().collect::<String>();
    key.push_str(chars.as_str());
    Some(key)
}

fn likely_defaults_semantics(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("defaults")
}

/// Names that suggest comparator-style ordering (antisymmetric, NOT symmetric).
/// Keep this list narrow: generic words like "order" create false positives for
/// ordinary business functions such as average_order_value(total, count).
const ANTISYMMETRIC_NAME_CUES: &[&str] = &["compare", "cmp", "sort", "asc", "desc"];

fn has_declared_property(func: &FunctionInfo, property: &str) -> bool {
    func.declared_properties
        .iter()
        .any(|declared| declared == property)
}

fn has_query_nested_brackets_contract(func: &FunctionInfo) -> bool {
    has_declared_property(func, "query_nested_brackets")
}

fn has_same_value_zero_contract(func: &FunctionInfo) -> bool {
    if has_declared_property(func, "same_value_zero") {
        return true;
    }
    func.name.to_lowercase().replace(['_', '-'], "") == "samevaluezero"
}

fn has_http_request_metadata_contract(func: &FunctionInfo) -> bool {
    has_declared_property(func, "http_request_metadata")
}

fn has_http_response_helpers_contract(func: &FunctionInfo) -> bool {
    has_declared_property(func, "http_response_helpers")
}

fn has_http_static_file_middleware_contract(func: &FunctionInfo) -> bool {
    has_declared_property(func, "http_static_file_middleware")
}

fn is_api_surface(func: &FunctionInfo) -> bool {
    !func.is_nested && func.is_exported && (!func.is_method || func.invocation_target.is_some())
}

fn ts_call_with_args(func: &FunctionInfo, args: &[&str]) -> String {
    let joined = args.join(", ");
    if let Some(target) = func.invocation_target.as_deref() {
        format!("{target}({joined})")
    } else {
        format!("({} as Function)({joined})", func.name)
    }
}

fn ts_call_with_spread(func: &FunctionInfo, spread_args: &str) -> String {
    if let Some(target) = func.invocation_target.as_deref() {
        format!("{target}({spread_args})")
    } else {
        format!("({} as Function)({spread_args})", func.name)
    }
}

fn is_ts_type_param_like(type_name: &str) -> bool {
    let trimmed = type_name.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_ts_defaults_semantic_target(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    if !is_api_surface(func) || !likely_defaults_semantics(&func.name) || param_types.is_empty() {
        return false;
    }

    let target_type = param_types[0].trim();
    let ret_type = ret_type.trim();
    !target_type.is_empty()
        && ret_type == target_type
        && (is_ts_type_param_like(target_type)
            || is_ts_mapping_type(target_type, type_defs)
            || looks_like_ts_object_type(target_type)
            || ts_class_def(target_type, type_defs).is_some())
}

fn ts_has_object_like_input_type(type_name: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    let trimmed = type_name.trim();
    if trimmed.is_empty() {
        return false;
    }

    let union_branches = split_ts_top_level(trimmed, '|');
    if union_branches.len() > 1 {
        return union_branches.iter().any(|branch| {
            let branch = branch.trim();
            !matches!(branch, "null" | "undefined")
                && ts_has_object_like_input_type(branch, type_defs)
        });
    }

    let intersection_branches = split_ts_top_level(trimmed, '&');
    if intersection_branches.len() > 1 {
        return intersection_branches
            .iter()
            .any(|branch| ts_has_object_like_input_type(branch.trim(), type_defs));
    }

    let effective = ts_effective_type(trimmed, type_defs);
    let effective = effective.trim();
    is_ts_mapping_type(effective, type_defs) || ts_class_def(effective, type_defs).is_some()
}

fn should_require_ts_nonempty_string(
    func: &FunctionInfo,
    param_types: &[String],
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    if has_declared_property(func, "nonempty_string") {
        return true;
    }
    if !likely_nonempty_string(&func.name) {
        return false;
    }

    !func.is_method
        && !func.is_nested
        && param_types
            .iter()
            .any(|param_type| ts_has_object_like_input_type(param_type, type_defs))
}

fn is_python_sequence_type(type_name: &str) -> bool {
    let trimmed = type_name.trim();
    matches!(
        trimmed,
        "list"
            | "List"
            | "Sequence"
            | "Iterable"
            | "Collection"
            | "typing.Sequence"
            | "typing.Iterable"
            | "typing.Collection"
            | "collections.abc.Sequence"
            | "collections.abc.Iterable"
            | "collections.abc.Collection"
    ) || starts_with_any(
        trimmed,
        &[
            "list[",
            "List[",
            "Sequence[",
            "Iterable[",
            "Collection[",
            "typing.Sequence[",
            "typing.Iterable[",
            "typing.Collection[",
            "collections.abc.Sequence[",
            "collections.abc.Iterable[",
            "collections.abc.Collection[",
        ],
    )
}

fn is_python_mapping_type(type_name: &str) -> bool {
    let trimmed = type_name.trim();
    matches!(
        trimmed,
        "dict"
            | "Dict"
            | "Mapping"
            | "MutableMapping"
            | "typing.Mapping"
            | "typing.MutableMapping"
            | "collections.abc.Mapping"
            | "collections.abc.MutableMapping"
    ) || starts_with_any(
        trimmed,
        &[
            "dict[",
            "Dict[",
            "Mapping[",
            "MutableMapping[",
            "typing.Mapping[",
            "typing.MutableMapping[",
            "collections.abc.Mapping[",
            "collections.abc.MutableMapping[",
        ],
    )
}

fn is_ts_mapping_type(type_name: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    let effective = ts_effective_type(type_name, type_defs);
    let trimmed = effective.trim();
    trimmed.starts_with("Record<") || looks_like_ts_object_type(trimmed)
}

fn is_ts_query_parser_return_type(type_name: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    let effective = ts_effective_type(type_name, type_defs);
    let trimmed = effective.trim();
    matches!(trimmed, "unknown" | "any" | "object")
        || trimmed.starts_with("Record<")
        || looks_like_ts_object_type(trimmed)
}

fn is_boolean_like_ts_type(type_name: &str) -> bool {
    let branches = split_ts_top_level(type_name.trim(), '|');
    if branches.len() > 1 {
        return branches
            .iter()
            .all(|branch| matches!(branch.trim(), "boolean" | "null" | "undefined"));
    }
    matches!(type_name.trim(), "boolean" | "null" | "undefined")
}

fn ts_object_field_type(
    type_name: &str,
    field_name: &str,
    type_defs: &TsNamedTypes<'_>,
) -> Option<String> {
    let trimmed = type_name.trim();
    if let Some(resolved) = ts_resolve_alias_text(trimmed, type_defs) {
        return ts_object_field_type(&resolved, field_name, type_defs);
    }

    let union_branches = split_ts_top_level(trimmed, '|');
    if union_branches.len() > 1 {
        for branch in union_branches {
            let branch = branch.trim();
            if matches!(branch, "null" | "undefined") {
                continue;
            }
            if let Some(found) = ts_object_field_type(branch, field_name, type_defs) {
                return Some(found);
            }
        }
        return None;
    }

    let intersection_branches = split_ts_top_level(trimmed, '&');
    if intersection_branches.len() > 1 {
        for branch in intersection_branches {
            if let Some(found) = ts_object_field_type(branch.trim(), field_name, type_defs) {
                return Some(found);
            }
        }
        return None;
    }

    if let Some(class) = ts_class_def(trimmed, type_defs) {
        return class
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .and_then(|field| field.type_annotation.clone());
    }

    if looks_like_ts_object_type(trimmed) {
        return extract_ts_object_type_fields_from_text(trimmed)
            .into_iter()
            .find(|field| field.name == field_name)
            .and_then(|field| field.type_annotation);
    }

    None
}

fn infer_python_contract(func: &FunctionInfo, params: &[&ParamInfo]) -> Option<ContractKind> {
    let ret_type = func.return_type.as_deref().unwrap_or("").trim();
    if params.len() == 1 {
        let param_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
        if param_type == "str" && ret_type == "str" {
            return Some(ContractKind::StringTransform);
        }
        if is_python_mapping_type(param_type) && ret_type == "str" {
            if likely_query_string_semantics(&func.name) {
                return Some(ContractKind::QueryStringSerializer);
            }
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
    if !param_types.is_empty()
        && param_types[0].trim() == "string"
        && is_ts_query_parser_return_type(ret_type, type_defs)
        && likely_query_string_semantics(&func.name)
    {
        return Some(ContractKind::QueryStringParser);
    }
    if param_types.len() == 1 {
        let param_type = param_types[0].trim();
        let ret_type = ret_type.trim();
        if param_type == "string" && ret_type == "string" {
            return Some(ContractKind::StringTransform);
        }
        if is_ts_mapping_type(param_type, type_defs) && ret_type == "string" {
            if likely_query_string_semantics(&func.name) {
                return Some(ContractKind::QueryStringSerializer);
            }
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
        && has_declared_property(func, "idempotent")
    {
        "        _result2 = _cj_invoke(_replace_args(_args, {0: _result}))\n        _cj_require(\"idempotent\", _nan_eq(_result, _result2), lambda: f\"Not idempotent: {repr(_result)} -> {repr(_result2)}\")".into()
    } else {
        String::new()
    }
}

fn python_consistency_check(func: &FunctionInfo) -> String {
    if !supports_implicit_consistency(func) {
        return String::new();
    }

    // Run the same input twice, verify same output
    "        _result_b = _cj_invoke(_args)\n        _cj_require(\"consistent\", _consistency_eq(_result, _result_b), lambda: f\"Inconsistent: {repr(_result)} != {repr(_result_b)}\")".into()
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
    if types_match && has_declared_property(func, "bounded") {
        "        _cj_require(\"bounded\", len(_result) <= len(_args[0]), lambda: f\"Not bounded: len({repr(_result)}) > len({repr(_args[0])})\")".to_string()
    } else {
        String::new()
    }
}

fn python_nonneg_check(func: &FunctionInfo) -> String {
    let ret_type = func.return_type.as_deref().unwrap_or("");
    if (ret_type == "int" || ret_type == "float") && has_declared_property(func, "nonneg") {
        "        _cj_require(\"nonneg\", _result >= 0, lambda: f\"Non-negative violation: {repr(_result)} < 0\")".to_string()
    } else {
        String::new()
    }
}

fn python_clamped_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "clamped") || params.len() < 3 {
        return String::new();
    }

    "        if all(isinstance(_value, (int, float)) and not isinstance(_value, bool) for _value in (_args[0], _args[1], _args[2], _result)):\n            _lo = min(_args[1], _args[2])\n            _hi = max(_args[1], _args[2])\n            _cj_require(\"clamped\", _lo <= _result <= _hi, lambda: f\"Clamp bounds violated: {repr(_result)} not in [{repr(_lo)}, {repr(_hi)}]\")\n            if _lo <= _args[0] <= _hi:\n                _cj_require(\"clamped\", _result == _args[0], lambda: f\"Clamp passthrough violated: {repr(_result)} != {repr(_args[0])}\")".to_string()
}

fn python_sorted_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "sorted") || params.is_empty() {
        return String::new();
    }

    "        if isinstance(_result, (list, tuple)) and all(isinstance(_item, (int, float, str)) for _item in _result):\n            _cj_require(\"sorted\", list(_result) == sorted(_result), lambda: f\"Not sorted: {repr(_result)}\")".to_string()
}

fn python_permutation_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "permutation") || params.is_empty() {
        return String::new();
    }

    "        if isinstance(_args[0], (list, tuple)) and isinstance(_result, (list, tuple)):\n            _cj_require(\"permutation\", _multiset_counts(_result) == _multiset_counts(_args[0]), lambda: f\"Permutation violated: {repr(_result)} vs {repr(_args[0])}\")".to_string()
}

fn python_palindrome_check(func: &FunctionInfo) -> String {
    if !has_declared_property(func, "palindrome") {
        return String::new();
    }

    "        if isinstance(_result, (list, tuple, str)):\n            _cj_require(\"palindrome\", _is_palindrome_sequence(_result), lambda: f\"Palindrome violated: {repr(_result)}\")".to_string()
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
        && (has_declared_property(func, "no_nullish_string")
            || ((is_api_surface(func)
                || infer_python_contract(func, params) == Some(ContractKind::MappingSerializer))
                && likely_nullish_string_leak(&func.name)))
    {
        "        if _contains_nullish(_args[0]):\n            _cj_require(\"no_nullish_string\", not _string_leaks_nullish(_result), lambda: f\"Nullish string leak: {repr(_result)}\")".to_string()
    } else {
        String::new()
    }
}

fn python_query_string_semantic_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if infer_python_contract(func, params) != Some(ContractKind::QueryStringSerializer) {
        return String::new();
    }

    format!(
        r#"_query_cases = [
    ("tag/nullish", {{"tag": ["pro", None, " beta "]}}, [("tag", "pro"), ("tag", "beta")]),
    ("blank scalar", {{"q": "  ", "page": 2}}, [("page", "2")]),
    ("accent fold", {{"q": "naïve café"}}, [("q", _ascii_fold("naïve café"))]),
    ("nested non-scalars", {{"filters": [{{"label": "pro"}}, None, " beta "]}}, [("filters", "beta")]),
]
for _query_label, _query_input, _expected_pairs in _query_cases:
    _crash += _semantic_check("{name}", {name}, [_query_input], _expected_pairs, "query_pairs", "Query semantics (" + _query_label + ")")
"#,
        name = func.name,
    )
}

fn python_pep440_version_ordering_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "pep440_version_ordering") || params.len() != 2 {
        return String::new();
    }
    if !matches!(
        func.return_type.as_deref().map(str::trim),
        Some("int" | "float")
    ) {
        return String::new();
    }
    if !params
        .iter()
        .all(|param| matches!(param.type_annotation.as_deref().map(str::trim), Some("str")))
    {
        return String::new();
    }

    format!(
        r#"_pep440_cases = [
    ("dev before alpha", "1.0.dev1", "1.0a1", -1),
    ("alpha before beta", "1.0a1", "1.0b1", -1),
    ("beta before rc", "1.0b1", "1.0rc1", -1),
    ("rc before final", "1.0rc1", "1.0", -1),
    ("final before post", "1.0", "1.0.post1", -1),
    ("release segment numeric ordering", "1.2", "1.10", -1),
    ("equivalent release forms", "1.0", "1.0.0", 0),
]
for _pep440_label, _left, _right, _expected in _pep440_cases:
    _crash += _semantic_check("{name}", {name}, [_left, _right], _expected, "sign", "PEP 440 version ordering (" + _pep440_label + ")")
    _crash += _semantic_check("{name}", {name}, [_right, _left], -_expected, "sign", "PEP 440 version ordering reverse (" + _pep440_label + ")")
"#,
        name = func.name,
    )
}

fn python_pep440_specifier_membership_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "pep440_specifier_membership") || params.len() != 2 {
        return String::new();
    }
    if !matches!(func.return_type.as_deref().map(str::trim), Some("bool")) {
        return String::new();
    }
    if !params
        .iter()
        .all(|param| matches!(param.type_annotation.as_deref().map(str::trim), Some("str")))
    {
        return String::new();
    }

    format!(
        r#"_specifier_cases = [
    ("inclusive lower bound", "1.0", ">=1.0", True),
    ("exclusive upper bound", "2.0.0", "<2.0", False),
    ("compatible includes patch", "1.4.5", "~=1.4", True),
    ("compatible excludes next minor", "1.5.0", "~=1.4.5", False),
    ("prerelease excluded by default", "1.0a1", ">=1.0", False),
]
for _specifier_label, _version, _specifier, _expected in _specifier_cases:
    _crash += _semantic_check("{name}", {name}, [_version, _specifier], _expected, "bool", "PEP 440 specifier membership (" + _specifier_label + ")")
"#,
        name = func.name,
    )
}

fn python_pep440_filter_prerelease_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "pep440_filter_prerelease") || params.len() != 2 {
        return String::new();
    }
    if !matches!(
        func.return_type.as_deref().map(str::trim),
        Some("list[str]" | "List[str]")
    ) {
        return String::new();
    }
    let first_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
    let second_type = params[1].type_annotation.as_deref().unwrap_or("").trim();
    if !matches!(first_type, "list[str]" | "List[str]") || second_type != "str" {
        return String::new();
    }

    format!(
        r#"_filter_cases = [
    ("stable lower bound", ["1.2", "1.3"], ">=1.3", ["1.3"]),
    ("prerelease-only fallback", ["1.2", "1.5a1"], ">=1.5", ["1.5a1"]),
    ("empty specifier preserves prerelease-only input", ["1.0a1"], "", ["1.0a1"]),
    ("stable match suppresses prerelease fallback", ["1.5a1", "1.5"], ">=1.5", ["1.5"]),
]
for _filter_label, _candidates, _specifier, _expected in _filter_cases:
    _crash += _semantic_check("{name}", {name}, [_candidates, _specifier], _expected, "list", "PEP 440 prerelease filter (" + _filter_label + ")")
"#,
        name = func.name,
    )
}

fn python_cookie_value_quote_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "cookie_value_quote") || params.len() != 1 {
        return String::new();
    }
    let param_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
    if param_type != "str" || !matches!(func.return_type.as_deref().map(str::trim), Some("str")) {
        return String::new();
    }

    format!(
        r#"_cookie_value_cases = [
    ("already quoted value round-trips", '"two words"', '"two words"'),
    ("unquoted value is trimmed", "  dark  ", "dark"),
]
for _cookie_value_label, _cookie_value, _expected in _cookie_value_cases:
    _crash += _semantic_check("{name}", {name}, [_cookie_value], _expected, "identity", "Cookie value quoting (" + _cookie_value_label + ")")
"#,
        name = func.name,
    )
}

fn python_cookie_header_quote_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if !has_declared_property(func, "cookie_header_quote") || params.len() != 1 {
        return String::new();
    }
    let param_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
    if !is_python_mapping_type(param_type)
        || !matches!(func.return_type.as_deref().map(str::trim), Some("str"))
    {
        return String::new();
    }

    format!(
        r#"_cookie_header_cases = [
    ("quoted value round-trips", {{"session": '"two words"'}}, 'session="two words"'),
    ("separator value is quoted", {{"token": "a,b"}}, 'token="a,b"'),
    ("none values are skipped", {{"theme": "dark", "empty": None}}, "theme=dark"),
]
for _cookie_header_label, _cookies, _expected in _cookie_header_cases:
    _crash += _semantic_check("{name}", {name}, [_cookies], _expected, "identity", "Cookie header quoting (" + _cookie_header_label + ")")
"#,
        name = func.name,
    )
}

fn python_comparator_check(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if infer_python_contract(func, params) == Some(ContractKind::Comparator)
        || has_declared_property(func, "antisymmetric")
    {
        "        _self_cmp = _cj_invoke(_replace_args(_args, {1: _args[0]}))\n        _cj_require(\"comparator\", _cmp_sign(_self_cmp) == 0, lambda: f\"Comparator self-compare should be zero: {repr(_self_cmp)}\")\n        _rev_cmp = _cj_invoke(_replace_args(_args, {0: _args[1], 1: _args[0]}))\n        _cj_require(\"comparator\", _cmp_sign(_result) == -_cmp_sign(_rev_cmp), lambda: f\"Comparator antisymmetry violated: {repr(_result)} vs {repr(_rev_cmp)}\")".into()
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
    if t0 == t1 && !t0.is_empty() && has_declared_property(func, "symmetric") {
        "        _result_sym = _cj_invoke(_replace_args(_args, {0: _args[1], 1: _args[0]}))\n        _cj_require(\"symmetric\", _nan_eq(_result, _result_sym), lambda: f\"Not symmetric: {repr(_result)} != {repr(_result_sym)}\")".into()
    } else {
        String::new()
    }
}

fn python_metamorphic_checks(func: &FunctionInfo, params: &[&ParamInfo]) -> String {
    if params.len() != 1 {
        return String::new();
    }
    let param = params[0];
    let param_type = param.type_annotation.as_deref().unwrap_or("").trim();
    let return_type = func.return_type.as_deref().unwrap_or("").trim();
    let invoke = |argument: &str| format!("_cj_invoke(_replace_args(_args, {{0: {argument}}}))");
    let mut checks = Vec::new();

    if has_declared_property(func, "involution")
        && !param_type.is_empty()
        && param_type == return_type
    {
        checks.push(format!(
            "        _involution_result = _materialize_if_iterator({})\n        _cj_require(\"involution\", _nan_eq(_args[0], _involution_result), lambda: f\"Involution violated: {{repr(_args[0])}} -> {{repr(_result)}} -> {{repr(_involution_result)}}\")",
            invoke("_copy.deepcopy(_result)")
        ));
    }

    if has_declared_property(func, "monotonic")
        && matches!(param_type, "int" | "float")
        && matches!(return_type, "int" | "float")
    {
        checks.push(format!(
            "        _monotonic_input = _copy.deepcopy(_args[0]) + 1\n        _monotonic_result = _materialize_if_iterator({})\n        _cj_require(\"monotonic\", _monotonic_result >= _result, lambda: f\"Monotonicity violated: f({{repr(_args[0])}})={{repr(_result)}} > f({{repr(_monotonic_input)}})={{repr(_monotonic_result)}}\")",
            invoke("_monotonic_input")
        ));
    }

    let orderable_input = param_type.starts_with("list[")
        || param_type.starts_with("List[")
        || param_type.starts_with("tuple[")
        || param_type.starts_with("Tuple[");
    if has_declared_property(func, "order_invariant") && orderable_input {
        checks.push(format!(
            "        _order_input = type(_args[0])(reversed(_copy.deepcopy(_args[0])))\n        _order_result = _materialize_if_iterator({})\n        _cj_require(\"order_invariant\", _nan_eq(_result, _order_result), lambda: f\"Order invariance violated: {{repr(_result)}} != {{repr(_order_result)}}\")",
            invoke("_order_input")
        ));
    }

    checks.join("\n")
}

const PYTHON_FUZZ_PRELUDE: &str = include_str!("synthesize/python/prelude.py");

const PYTHON_FUZZ_EPILOGUE: &str = include_str!("synthesize/python/epilogue.py");

// ── TypeScript fuzz harness ─────────────────────────────────────────────────

fn synthesize_typescript(
    analysis: &AnalysisResult,
    type_defs: &TsNamedTypes<'_>,
    seed_inputs: &PlannedSeedInputs,
    safe_dependency_surfaces: &[String],
) -> FuzzPlan {
    let mut code = String::new();
    let mut coverage = Vec::new();
    let has_exported = has_exported_surface(&analysis.functions);

    code.push_str(TYPESCRIPT_FUZZ_PRELUDE);
    let safe_surfaces =
        serde_json::to_string(safe_dependency_surfaces).unwrap_or_else(|_| "[]".into());
    code.push_str(&format!(
        "const _CJ_SAFE_DEPENDENCY_SURFACES = new Set({safe_surfaces});\n"
    ));

    let mut any_synthesized = false;
    let mut selected_functions = Vec::new();

    for func in synth_candidate_functions(&analysis.functions) {
        let callable_params: Vec<&ParamInfo> =
            func.params.iter().filter(|p| !p.is_variadic()).collect();
        let positional_variadic = func.params.iter().find(|p| p.is_positional_variadic());
        let has_nested = has_nested_children(func, &analysis.functions);
        let has_seed_rows = seed_inputs
            .get(&func.name)
            .is_some_and(|rows| !rows.is_empty());
        // Automatic domain/omission rows do not implement an unsupported type's
        // generator. Only independently supplied seed evidence permits fallback.
        let has_fallback_seeds = seed_inputs
            .get(&func.name)
            .is_some_and(|rows| rows.iter().any(|row| row.supports_type_fallback));
        if callable_params.is_empty()
            && positional_variadic.is_none()
            && !has_nested
            && has_noncheckable_ts_zero_arg_return_contract(func)
            && !has_seed_rows
        {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedNoFuzzableSurface,
                Some(
                    "zero-argument function has no meaningful parameter surface or stable return contract to fuzz".into(),
                ),
            ));
            continue;
        }
        if let Some(reason) = unsafe_dependency_reason(
            &callable_params,
            &Language::TypeScript,
            &analysis.classes,
            &analysis.aliases,
        ) {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(reason),
            ));
            continue;
        }

        if callable_params
            .iter()
            .any(|param| param.type_annotation.is_none())
            && !has_seed_rows
        {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(
                    "one or more TypeScript parameters are untyped and no seed/domain examples were found"
                        .into(),
                ),
            ));
            continue;
        }

        if !has_fallback_seeds && !ts_params_are_fuzzable(func, &callable_params, type_defs) {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(
                    "one or more parameters use unsupported or unresolved TypeScript types".into(),
                ),
            ));
            continue;
        }

        if !should_fuzz_ts_helper(func, &callable_params, type_defs, has_exported) {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedInternalHelper,
                Some("non-exported helper is deferred to the exported API surface".into()),
            ));
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

        let Some(mut generated_parts) = callable_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                ts_generator_for_param(contract, p.type_annotation.as_deref(), type_defs, idx, func)
                    .or_else(|| has_fallback_seeds.then(|| "_fuzzAny()".to_string()))
            })
            .collect::<Option<Vec<_>>>()
        else {
            coverage.push(coverage_entry(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some(
                    "one or more parameters use unsupported or unresolved TypeScript types".into(),
                ),
            ));
            continue;
        };
        if let Some(rest) = positional_variadic {
            let rest_annotation = rest.type_annotation.as_deref().map(ts_rest_item_annotation);
            let Some(item_generator) =
                ts_generator_for_param(contract, rest_annotation.as_deref(), type_defs, 0, func)
            else {
                coverage.push(coverage_entry(
                    func,
                    FuzzFunctionStatus::SkippedUnsupportedType,
                    Some(
                        "one or more parameters use unsupported or unresolved TypeScript types"
                            .into(),
                    ),
                ));
                continue;
            };
            generated_parts.push(format!(
                "...Array.from({{ length: _fuzzIntRange(0, 2) }}, () => {})",
                item_generator
            ));
        }
        coverage.push(coverage_entry(
            func,
            FuzzFunctionStatus::CheckedDirect,
            None,
        ));
        selected_functions.push(func);
        let gen_list = generated_parts.join(", ");

        let mut properties: Vec<&str> = vec![];
        let mut push_property = |property: &'static str| {
            if !properties.contains(&property) {
                properties.push(property);
            }
        };
        if supports_implicit_consistency(func) {
            push_property("consistent");
        }

        // Idempotency: single-arg, same in/out type, name suggests it
        if callable_params.len() == 1
            && !param_types[0].is_empty()
            && param_types[0].as_str() == ret_type
            && !ret_type.contains("null")
            && !ret_type.contains("undefined")
            && is_idempotent_candidate_type(ret_type)
            && has_declared_property(func, "idempotent")
        {
            push_property("idempotent");
        }
        // Boundedness: single-arg, str→str or array→array, name suggests it
        if callable_params.len() == 1
            && ((param_types[0].as_str() == "string" && ret_type == "string")
                || (param_types[0].ends_with("[]") && ret_type.ends_with("[]")))
            && has_declared_property(func, "bounded")
        {
            push_property("bounded");
        }
        // Non-negativity: returns number, name suggests it
        if ret_type == "number" && has_declared_property(func, "nonneg") {
            push_property("nonneg");
        }
        // Non-empty strings: string-returning identifier/display helpers should
        // not silently return blank output.
        if ret_type == "string" && should_require_ts_nonempty_string(func, &param_types, type_defs)
        {
            push_property("nonempty_string");
        }
        // Serialized/canonical string helpers should not leak nullish sentinel
        // text into the output when the input contains null/undefined values.
        if callable_params.len() == 1
            && ret_type == "string"
            && is_ts_mapping_type(&param_types[0], type_defs)
            && (has_declared_property(func, "no_nullish_string")
                || ((is_api_surface(func)
                    || infer_ts_contract(func, &param_types, ret_type, type_defs)
                        == Some(ContractKind::MappingSerializer))
                    && likely_nullish_string_leak(&func.name)))
        {
            push_property("no_nullish_string");
        }
        if infer_ts_contract(func, &param_types, ret_type, type_defs)
            == Some(ContractKind::Comparator)
            || has_declared_property(func, "antisymmetric")
        {
            push_property("comparator");
        }
        // Symmetry: two params same type, name suggests it
        if callable_params.len() == 2
            && !param_types[0].is_empty()
            && param_types[0] == param_types[1]
            && has_declared_property(func, "symmetric")
        {
            push_property("symmetric");
        }
        if has_declared_property(func, "sorted") {
            push_property("sorted");
        }
        if has_declared_property(func, "permutation") {
            push_property("permutation");
        }
        if has_declared_property(func, "clamped") {
            push_property("clamped");
        }
        if callable_params.len() == 1
            && !param_types[0].is_empty()
            && param_types[0] == ret_type
            && has_declared_property(func, "involution")
        {
            push_property("involution");
        }
        if callable_params.len() == 1
            && param_types[0] == "number"
            && ret_type == "number"
            && has_declared_property(func, "monotonic")
        {
            push_property("monotonic");
        }
        if callable_params.len() == 1
            && (param_types[0].ends_with("[]")
                || param_types[0].starts_with("Array<")
                || param_types[0].starts_with("ReadonlyArray<"))
            && has_declared_property(func, "order_invariant")
        {
            push_property("order_invariant");
        }

        let properties_list: String = properties
            .iter()
            .map(|p| format!("\"{}\"", p))
            .collect::<Vec<_>>()
            .join(", ");
        let declared_properties_list = func
            .declared_properties
            .iter()
            .map(|property| format!("\"{}\"", property.replace('"', "\\\"")))
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

        let query_string_semantic_check =
            ts_query_string_semantic_check(func, &param_types, ret_type, type_defs);
        let query_string_parser_semantic_check =
            ts_query_string_parser_semantic_check(func, &param_types, ret_type, type_defs);
        let defaults_semantic_check =
            ts_defaults_semantic_check(func, &param_types, ret_type, type_defs);
        let feature_flag_override_check =
            ts_feature_flag_override_check(func, &param_types, ret_type, type_defs);
        let semver_compare_semantic_check =
            ts_semver_compare_semantic_check(func, &param_types, ret_type);
        let semver_caret_semantic_check =
            ts_semver_caret_semantic_check(func, &param_types, ret_type);
        let same_value_zero_semantic_check =
            ts_same_value_zero_semantic_check(func, &param_types, ret_type);
        let request_metadata_semantic_check =
            ts_http_request_metadata_semantic_check(func, &param_types);
        let response_helpers_semantic_check =
            ts_http_response_helpers_semantic_check(func, &param_types);
        let static_file_semantic_check = ts_http_static_file_semantic_check(func, &param_types);

        let mut seed_rows = ts_seed_rows(func, seed_inputs);
        if has_declared_property(func, "sorted")
            && param_types.first().is_some_and(|type_name| {
                type_name.ends_with("[]")
                    || type_name.starts_with("Array<")
                    || type_name.starts_with("ReadonlyArray<")
            })
        {
            let mut property_row = generated_parts.clone();
            if let Some(first) = property_row.first_mut() {
                *first = if param_types[0].contains("string") {
                    "[\"b\", \"a\"]".into()
                } else {
                    "[2, 1]".into()
                };
            }
            let property_row = format!(
                "{{ args: [{}], contractValid: false }}",
                property_row.join(", ")
            );
            seed_rows = if seed_rows.is_empty() {
                property_row
            } else {
                format!("{property_row}, {seed_rows}")
            };
        }
        let default_omission_rows = ts_default_omission_rows(&callable_params, &generated_parts);

        code.push_str(&format!(
            r#"
{{
  _fuzzOne("{name}", {iters}, () => [{gen_list}], (args: unknown[]) => {call_expr}, {typecheck}, [{param_type_list}], [{properties_list}], [{seed_rows}], [{default_omission_rows}], [{declared_properties_list}], {source_line}, {rejection_domains});
{query_string_semantic_check}
{query_string_parser_semantic_check}
{defaults_semantic_check}
{feature_flag_override_check}
{semver_compare_semantic_check}
{semver_caret_semantic_check}
{same_value_zero_semantic_check}
{request_metadata_semantic_check}
{response_helpers_semantic_check}
{static_file_semantic_check}
}}
"#,
            name = func.name,
            iters = FUZZ_ITERATIONS,
            call_expr = ts_call_with_spread(func, "...args"),
            typecheck = ts_type_check_fn(ret_type),
            seed_rows = seed_rows,
            default_omission_rows = default_omission_rows,
            query_string_semantic_check = query_string_semantic_check,
            query_string_parser_semantic_check = query_string_parser_semantic_check,
            defaults_semantic_check = defaults_semantic_check,
            feature_flag_override_check = feature_flag_override_check,
            semver_compare_semantic_check = semver_compare_semantic_check,
            semver_caret_semantic_check = semver_caret_semantic_check,
            same_value_zero_semantic_check = same_value_zero_semantic_check,
            request_metadata_semantic_check = request_metadata_semantic_check,
            response_helpers_semantic_check = response_helpers_semantic_check,
            static_file_semantic_check = static_file_semantic_check,
            declared_properties_list = declared_properties_list,
            source_line = func.line,
            rejection_domains = rejection_domains(func, analysis, &Language::TypeScript),
        ));

        any_synthesized = true;
    }

    if !any_synthesized {
        return FuzzPlan {
            code: String::new(),
            coverage,
        };
    }

    // Factory exercise: for functions that contain nested functions,
    // call the factory and fuzz the returned object's methods
    coverage.extend(factory_callable_coverage(
        analysis,
        &selected_functions,
        type_defs,
    ));
    code.push_str(&synthesize_typescript_factory_exercise(
        analysis,
        &selected_functions,
        type_defs,
    ));

    // Involution roundtrip checks
    code.push_str(&synthesize_typescript_involution_checks(
        analysis, type_defs,
    ));
    code.push_str(TYPESCRIPT_FUZZ_EPILOGUE);
    FuzzPlan { code, coverage }
}

/// For factory functions (functions containing nested function definitions),
/// call the factory with fuzzed args, then exercise any callable properties
/// on the returned object.
fn synthesize_typescript_factory_exercise(
    analysis: &AnalysisResult,
    selected_functions: &[&FunctionInfo],
    type_defs: &TsNamedTypes<'_>,
) -> String {
    let mut code = String::new();

    for func in selected_functions {
        if func.returned_callables.is_empty() {
            continue;
        }

        let callable_params: Vec<&ParamInfo> = func
            .params
            .iter()
            .filter(|param| !param.is_variadic())
            .collect();
        if !ts_params_are_fuzzable(func, &callable_params, type_defs) {
            continue;
        }
        let Some(factory_args) = callable_params
            .iter()
            .map(|param| ts_generator(param.type_annotation.as_deref(), type_defs))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let factory_arg_slots = (0..factory_args.len())
            .map(|index| format!("_args[{index}]"))
            .collect::<Vec<_>>();
        let factory_arg_refs = factory_arg_slots
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let factory_call = ts_call_with_args(func, &factory_arg_refs);

        let known_specs = func
            .returned_callables
            .iter()
            .filter_map(|callable| {
                let declaration = known_factory_callable(analysis, func, callable, type_defs)?;
                let args = declaration
                    .params
                    .iter()
                    .filter(|param| !param.is_variadic())
                    .map(|param| ts_generator(param.type_annotation.as_deref(), type_defs))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ");
                let key = serde_json::to_string(callable).ok()?;
                let surface =
                    serde_json::to_string(&format!("{}().{}", func.name, callable)).ok()?;
                Some(format!(
                    "{key}: {{ surface: {surface}, line: {line}, args: () => [{args}] }}",
                    line = declaration.line,
                ))
            })
            .collect::<Vec<_>>();
        if known_specs.is_empty() {
            continue;
        }
        let known_specs_expr = format!("{{{}}}", known_specs.join(", "));
        let returned_names = func.returned_callables.join(", ");

        code.push_str(&format!(
            r#"
// Stateful factory action-sequence campaign: {name}
{{
  let _factoryPass = 0, _factoryCrash = 0, _factoryUnknown = 0;
  const _factoryInvoke = (_args: unknown[]) => {factory_call};
  const _knownFactoryCallables: Record<string, {{surface: string; line: number; args: () => unknown[]}}> = {known_specs_expr};
  const _actionKeys = Object.keys(_knownFactoryCallables);
  for (let _fi = 0; _fi < {iters}; _fi++) {{
    let _activeFactoryCallable = "unknown";
    let _activeFactorySurface = "{name} (factory)";
    let _activeFactoryLine = {factory_line};
    let _activeFactoryArgs: unknown[] = [];
    const _actionTrace: Array<{{action: string; expression: string | null; callable?: boolean}}> = [];
    let _setupExpression: string | null = null;
    let _factoryPhase = "arguments";
    try {{
      const _setupArgs = [{factory_args}];
      _setupExpression = _factoryArgumentExpression(_setupArgs);
      _factoryPhase = "factory";
      const _factory = _factoryInvoke(_setupArgs);
      const _actionPlan = [..._actionKeys];
      for (let _step = 0; _step < _fuzzIntRange(2, 5); _step++) {{
        _actionPlan.push(_actionKeys[_fuzzIntRange(0, _actionKeys.length - 1)]);
      }}
      for (const [_index, _action] of _actionPlan.entries()) {{
        const _spec = _knownFactoryCallables[_action];
        _activeFactoryCallable = _action;
        _activeFactorySurface = _spec.surface;
        _activeFactoryLine = _spec.line;
        _factoryPhase = "resolve:" + _index;
        const _entry: {{action: string; expression: string | null; callable?: boolean}} = {{ action: _action, expression: "[]" }};
        _actionTrace.push(_entry);
        const _candidate = _resolveFactoryAction(_factory, _action, _actionKeys.length === 1);
        _entry.callable = typeof _candidate === "function";
        if (typeof _candidate !== "function") continue;
        _factoryPhase = "arguments:" + _index;
        _activeFactoryArgs = _spec.args();
        _entry.expression = _factoryArgumentExpression(_activeFactoryArgs);
        _targetEntered(_activeFactorySurface);
        _factoryPhase = "action:" + _index;
        (_candidate as Function).apply(_factory, _activeFactoryArgs);
      }}
      _factoryPass++;
    }} catch (_e: unknown) {{
      const _caseSource = _setupExpression === null || _actionTrace.some(entry => entry.expression === null)
        ? null : "{{factory:" + _setupExpression + ",actions:[" + _actionTrace.map(entry =>
          "{{action:" + JSON.stringify(entry.action) + ",args:" + entry.expression + ",callable:" + String(entry.callable) + "}}").join(",") + "]}}";
      const _snippet = _factoryReplaySnippet(_factoryInvoke, _caseSource, _actionKeys.length === 1, _factoryPhase, _e);
      const _originalCase = {{ arguments: [{{ expression: _caseSource ?? "undefined" }}], input_text: _clipText(_shortJson(_actionTrace)) }};
      const _crash = _isCrash(_e);
      _emitFinding(_activeFactorySurface, [], _e, "crash", "runtime_contract", "observed_call", _crash ? "high" : "low", "exception", null, {{factory: {{factory: "{name}", callable: _activeFactoryCallable}}}}, _clipText(_shortJson(_actionTrace)), _activeFactoryLine, _snippet, _crash ? "valid" : "unknown", null, "semantic_case", _originalCase);
      if (_crash) {{
        _factoryCrash++;
        if (_factoryCrash === 1) console.log(`  CRASH ${{_activeFactorySurface}} after actions ${{_clipText(_shortJson(_actionTrace))}}: ${{_clipText(_e)}}`);
      }} else {{ _factoryUnknown++; }}
    }}
  }}
  const _ftotal = _factoryPass + _factoryCrash + _factoryUnknown;
  if (_factoryCrash > 0) {{
    console.log(`FUZZ {name} (factory state machine): ${{_factoryPass}} passed, ${{_factoryCrash}} CRASHED (of ${{_ftotal}}) [actions: {returned_names}]`);
    _fuzzTotalFailures++;
  }} else {{
    console.log(`FUZZ {name} (factory state machine): ${{_factoryPass}} passed, 0 rejected, 0 CRASHED, ${{_factoryUnknown}} unclassified (of ${{_ftotal}}) [actions: {returned_names}]`);
  }}
}}
"#,
            name = func.name,
            factory_line = func.line,
            factory_call = factory_call,
            factory_args = factory_args.join(", "),
            known_specs_expr = known_specs_expr,
            iters = FUZZ_ITERATIONS,
            returned_names = returned_names,
        ));
    }

    code
}

fn strip_balanced_ts_outer_parentheses(mut text: &str) -> &str {
    loop {
        let bytes = text.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'(' || bytes[bytes.len() - 1] != b')' {
            return text;
        }
        let mut depth = 0i32;
        let mut quote = None;
        let mut escaped = false;
        let mut encloses = true;
        for (index, character) in text.char_indices() {
            if let Some(active_quote) = quote {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == active_quote {
                    quote = None;
                }
                continue;
            }
            if matches!(character, '\'' | '"' | '`') {
                quote = Some(character);
                continue;
            }
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && index + character.len_utf8() < text.len() {
                        encloses = false;
                        break;
                    }
                    if depth < 0 {
                        encloses = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !encloses || depth != 0 || quote.is_some() {
            return text;
        }
        text = text[1..text.len() - 1].trim();
    }
}

fn ts_type_seen(stack: &[String], type_name: &str) -> bool {
    stack.iter().any(|item| item == type_name)
}

fn ts_top_level_arrow_return(type_ann: &str) -> Option<&str> {
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in type_ann.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            continue;
        }
        if ch == '=' && depth == 0 && type_ann.as_bytes().get(index + 1).copied() == Some(b'>') {
            return Some(type_ann[index + 2..].trim());
        }
        match ch {
            '{' | '[' | '<' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            '>' if type_ann.as_bytes().get(index.wrapping_sub(1)).copied() != Some(b'=') => {
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn ts_generator(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> Option<String> {
    if !ts_type_is_fuzzable(type_ann, type_defs) {
        return None;
    }
    Some(ts_generator_with_stack(
        type_ann,
        type_defs,
        &mut Vec::new(),
        0,
    ))
}

fn ts_generator_with_stack(
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    let t = match type_ann {
        Some(t) => strip_balanced_ts_outer_parentheses(t.trim()),
        // No type annotation: generate a mix of types instead of just undefined
        None => return "_fuzzAny()".to_string(),
    };
    if depth >= TS_TYPE_RECURSION_LIMIT {
        return "({})".into();
    }
    if let Some(TsNamedTypeRef::Alias(alias)) = type_defs.get(t) {
        if ts_type_seen(stack, t) {
            return "({})".into();
        }
        stack.push(t.to_string());
        let generated = ts_generator_with_stack(
            Some(alias.type_annotation.as_str()),
            type_defs,
            stack,
            depth + 1,
        );
        stack.pop();
        return generated;
    }
    if let Some(return_type) = ts_top_level_arrow_return(t) {
        let generated = ts_generator_with_stack(Some(return_type), type_defs, stack, depth + 1);
        return format!(
            "(() => {{ const _callbackValue = {generated}; return () => _callbackValue; }})()"
        );
    }
    match t {
        "number" => "_fuzzNum()".into(),
        "string" => "_fuzzStr()".into(),
        "boolean" => "_fuzzBool()".into(),
        "any" | "unknown" => "_fuzzAny()".into(),
        "typeof fetch" => {
            "(async () => ({ ok: false, status: 503, text: async () => \"\", json: async () => ({}) }))"
                .into()
        }
        _ if t.starts_with("keyof ") => "_fuzzAny()".into(),
        "object" => "_fuzzObject()".into(),
        _ if t.ends_with("[]") => {
            let inner = t[..t.len() - 2].trim();
            if inner == "never" {
                "[]".into()
            } else {
                let gen = ts_generator_with_stack(Some(inner), type_defs, stack, depth + 1);
                format!("Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen})")
            }
        }
        _ if t.starts_with("Array<") => {
            let inner = extract_generic_arg(t);
            if inner.trim() == "never" {
                "[]".into()
            } else {
                let gen = ts_generator_with_stack(Some(&inner), type_defs, stack, depth + 1);
                format!("Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen})")
            }
        }
        _ if t.starts_with("ReadonlyArray<") => {
            let inner = extract_generic_arg(t);
            if inner.trim() == "never" {
                "[]".into()
            } else {
                let gen = ts_generator_with_stack(Some(&inner), type_defs, stack, depth + 1);
                format!("Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen})")
            }
        }
        _ if t.starts_with("Set<") || t.starts_with("ReadonlySet<") => {
            let inner = extract_generic_arg(t);
            let gen = ts_generator_with_stack(Some(&inner), type_defs, stack, depth + 1);
            format!("new Set(Array.from({{length: _fuzzIntRange(0,5)}}, () => {gen}))")
        }
        _ if t.starts_with("Map<") || t.starts_with("ReadonlyMap<") => {
            let (key, value) = extract_two_generic_args(t);
            let key_gen = ts_generator_with_stack(Some(&key), type_defs, stack, depth + 1);
            let value_gen = ts_generator_with_stack(Some(&value), type_defs, stack, depth + 1);
            format!(
                "new Map(Array.from({{length: _fuzzIntRange(0,3)}}, () => [{key_gen}, {value_gen}]))"
            )
        }
        _ if t.starts_with("Record<") => {
            let (_k, v) = extract_two_generic_args(t);
            let vg = ts_generator_with_stack(Some(&v), type_defs, stack, depth + 1);
            format!("Object.fromEntries(Array.from({{length: _fuzzIntRange(0,3)}}, (_, i) => [`k${{i}}`, {vg}]))")
        }
        _ if looks_like_ts_object_type(t) => {
            ts_inline_object_generator_with_stack(t, type_defs, stack, depth + 1)
        }
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
                        .map(|b| ts_generator_with_stack(Some(b), type_defs, stack, depth + 1))
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
            } else if let Some(literal) = ts_literal_expr(t) {
                literal
            } else if t.contains(" & ") {
                let intersection = split_ts_top_level(t, '&');
                let first = intersection.first().map(|s| s.trim()).unwrap_or(t);
                ts_generator_with_stack(Some(first), type_defs, stack, depth + 1)
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
            } else if t == "URLSearchParams" {
                "_fuzzUrlSearchParams()".into()
            } else if t == "Request" {
                "_fuzzRequest()".into()
            } else if t == "Response" {
                "_fuzzResponse()".into()
            } else if t == "Headers" {
                "_fuzzHeaders()".into()
            } else if t == "FormData" {
                "new FormData()".into()
            } else if t == "AbortController" {
                "new AbortController()".into()
            } else if t.starts_with("Promise<") {
                let inner = extract_generic_arg(t);
                let gen = ts_generator_with_stack(Some(&inner), type_defs, stack, depth + 1);
                format!("Promise.resolve({gen})")
            } else if t == "Promise" {
                "Promise.resolve(_fuzzAny())".into()
            } else if let Some(class) = ts_class_def(t, type_defs) {
                if ts_type_seen(stack, t) {
                    return "({})".into();
                }
                if class.fields.is_empty() {
                    "({})".into()
                } else {
                    stack.push(t.to_string());
                    let props: Vec<String> = class
                        .fields
                        .iter()
                        .map(|f| {
                            let field_gen = ts_field_generator_with_stack(
                                f.name.as_str(),
                                f.type_annotation.as_deref(),
                                type_defs,
                                stack,
                                depth + 1,
                            );
                            let val = if f.optional {
                                format!("_fuzzBool() ? null : {}", field_gen)
                            } else {
                                field_gen
                            };
                            format!("{}: {}", f.name, val)
                        })
                        .collect();
                    stack.pop();
                    format!("({{ {} }})", props.join(", "))
                }
            } else if t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                "({})".into()
            } else {
                "undefined".into()
            }
        }
    }
}

fn ts_params_are_fuzzable(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    if params.iter().all(|param| {
        let resolved = param
            .type_annotation
            .as_deref()
            .map(|annotation| func.resolved_type_annotation(annotation));
        ts_type_is_fuzzable(resolved.as_deref(), type_defs)
    }) {
        return true;
    }

    if ts_query_parser_params_are_fuzzable(func, params, type_defs) {
        return true;
    }

    if !is_api_surface(func) || !likely_defaults_semantics(&func.name) || params.is_empty() {
        return false;
    }

    let target_type = params[0].type_annotation.as_deref().unwrap_or("").trim();
    let ret_type = func.return_type.as_deref().unwrap_or("").trim();
    !target_type.is_empty()
        && ret_type == target_type
        && func
            .type_parameters
            .iter()
            .any(|parameter| parameter == target_type)
        && params[1..]
            .iter()
            .all(|param| ts_type_is_fuzzable(param.type_annotation.as_deref(), type_defs))
}

fn ts_query_parser_params_are_fuzzable(
    func: &FunctionInfo,
    params: &[&ParamInfo],
    type_defs: &TsNamedTypes<'_>,
) -> bool {
    if params.is_empty()
        || !likely_query_string_semantics(&func.name)
        || !is_ts_query_parser_return_type(func.return_type.as_deref().unwrap_or(""), type_defs)
    {
        return false;
    }
    let first = params[0].type_annotation.as_deref().unwrap_or("").trim();
    first == "string"
        && params[1..].iter().all(|param| {
            ts_type_is_fuzzable(param.type_annotation.as_deref(), type_defs)
                || is_ts_parser_setting_type(param.type_annotation.as_deref())
        })
}

fn is_ts_parser_setting_type(type_ann: Option<&str>) -> bool {
    let Some(type_ann) = type_ann else {
        return false;
    };
    let lower = type_ann.trim().to_lowercase();
    lower.contains("setting")
        || lower.contains("mode")
        || lower.contains("option")
        || lower.contains("boolean")
}

fn ts_type_is_fuzzable(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> bool {
    ts_type_is_fuzzable_with_stack(type_ann, type_defs, &mut Vec::new(), 0)
}

fn ts_type_is_fuzzable_with_stack(
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
    stack: &mut Vec<String>,
    depth: usize,
) -> bool {
    let t = match type_ann {
        Some(t) => strip_balanced_ts_outer_parentheses(t.trim()),
        None => return true,
    };
    if depth >= TS_TYPE_RECURSION_LIMIT {
        return false;
    }
    if let Some(TsNamedTypeRef::Alias(alias)) = type_defs.get(t) {
        if ts_type_seen(stack, t) {
            return false;
        }
        stack.push(t.to_string());
        let fuzzable = ts_type_is_fuzzable_with_stack(
            Some(alias.type_annotation.as_str()),
            type_defs,
            stack,
            depth + 1,
        );
        stack.pop();
        return fuzzable;
    }
    if let Some(return_type) = ts_top_level_arrow_return(t) {
        return return_type == "void"
            || ts_type_is_fuzzable_with_stack(Some(return_type), type_defs, stack, depth + 1);
    }

    match t {
        "number" | "string" | "boolean" | "object" | "any" | "unknown" | "null" | "undefined"
        | "Date" | "RegExp" | "Map" | "Set" | "Error" | "Buffer" | "Uint8Array" | "ArrayBuffer"
        | "URL" | "URLSearchParams" | "Request" | "Response" | "Headers" | "FormData"
        | "AbortController" | "Promise" => true,
        _ if t == "typeof fetch" || t.starts_with("keyof ") => true,
        "never[]" => true,
        _ if t.ends_with("[]") => {
            let inner = &t[..t.len() - 2];
            ts_type_is_fuzzable_with_stack(Some(inner), type_defs, stack, depth + 1)
        }
        _ if t.starts_with("Array<") => {
            let inner = extract_generic_arg(t);
            ts_type_is_fuzzable_with_stack(Some(&inner), type_defs, stack, depth + 1)
        }
        _ if t.starts_with("ReadonlyArray<") => {
            let inner = extract_generic_arg(t);
            ts_type_is_fuzzable_with_stack(Some(&inner), type_defs, stack, depth + 1)
        }
        _ if t.starts_with("Set<") || t.starts_with("ReadonlySet<") => {
            let inner = extract_generic_arg(t);
            ts_type_is_fuzzable_with_stack(Some(&inner), type_defs, stack, depth + 1)
        }
        _ if t.starts_with("Map<") || t.starts_with("ReadonlyMap<") => {
            let (key, value) = extract_two_generic_args(t);
            ts_type_is_fuzzable_with_stack(Some(&key), type_defs, stack, depth + 1)
                && ts_type_is_fuzzable_with_stack(Some(&value), type_defs, stack, depth + 1)
        }
        _ if t.starts_with("Record<") => {
            let (k, v) = extract_two_generic_args(t);
            ts_type_is_fuzzable_with_stack(Some(&k), type_defs, stack, depth + 1)
                && ts_type_is_fuzzable_with_stack(Some(&v), type_defs, stack, depth + 1)
        }
        _ if looks_like_ts_object_type(t) => {
            extract_ts_object_type_fields_from_text(t)
                .iter()
                .all(|field| {
                    ts_type_is_fuzzable_with_stack(
                        field.type_annotation.as_deref(),
                        type_defs,
                        stack,
                        depth + 1,
                    )
                })
        }
        _ => {
            let union_branches = split_ts_top_level(t, '|');
            if union_branches.len() > 1 {
                union_branches.iter().all(|branch| {
                    matches!(branch.trim(), "null" | "undefined")
                        || ts_type_is_fuzzable_with_stack(
                            Some(branch.trim()),
                            type_defs,
                            stack,
                            depth + 1,
                        )
                })
            } else if t.contains(" & ") {
                split_ts_top_level(t, '&').iter().all(|branch| {
                    ts_type_is_fuzzable_with_stack(Some(branch.trim()), type_defs, stack, depth + 1)
                })
            } else if t.contains("=>") {
                true
            } else if t.starts_with("Promise<") {
                let inner = extract_generic_arg(t);
                ts_type_is_fuzzable_with_stack(Some(&inner), type_defs, stack, depth + 1)
            } else if let Some(class) = ts_class_def(t, type_defs) {
                if ts_type_seen(stack, t) {
                    return false;
                }
                stack.push(t.to_string());
                let fuzzable = class.fields.iter().all(|field| {
                    ts_type_is_fuzzable_with_stack(
                        field.type_annotation.as_deref(),
                        type_defs,
                        stack,
                        depth + 1,
                    )
                });
                stack.pop();
                fuzzable
            } else {
                ts_literal_expr(t).is_some()
            }
        }
    }
}

fn ts_rest_item_annotation(type_ann: &str) -> String {
    let trimmed = type_ann.trim();
    if let Some(inner) = trimmed.strip_suffix("[]") {
        return inner.trim().to_string();
    }
    if trimmed.starts_with("Array<") && trimmed.ends_with('>') {
        return trimmed[6..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

fn ts_generator_for_param(
    contract: Option<ContractKind>,
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
    index: usize,
    func: &FunctionInfo,
) -> Option<String> {
    if contract == Some(ContractKind::QueryStringParser) && index > 0 {
        return Some("\"extended\"".to_string());
    }
    if contract == Some(ContractKind::Comparator)
        && is_semver_like_version_type(type_ann, type_defs)
    {
        return Some("_fuzzSemverVersion()".to_string());
    }
    if index == 0
        && likely_defaults_semantics(&func.name)
        && type_ann.is_some_and(|type_ann| {
            let type_ann = type_ann.trim();
            func.type_parameters
                .iter()
                .any(|parameter| parameter == type_ann)
        })
    {
        return Some("_fuzzObject()".to_string());
    }
    let resolved = type_ann.map(|annotation| func.resolved_type_annotation(annotation));
    ts_generator(resolved.as_deref(), type_defs)
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

fn ts_query_string_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    if infer_ts_contract(func, param_types, ret_type, type_defs)
        != Some(ContractKind::QueryStringSerializer)
    {
        return String::new();
    }
    let query_args: Vec<&str> = if param_types.len() > 1 {
        vec!["_args[0]", "_args[1]"]
    } else {
        vec!["_args[0]"]
    };
    let call = ts_call_with_args(func, &query_args);
    let args = if param_types.len() > 1 {
        "[_queryInput, \"extended\"]"
    } else {
        "[_queryInput]"
    };
    let query_cases = if has_query_nested_brackets_contract(func) {
        r#"      ["top-level repeated array", { page: 2, tag: ["pro", "beta"] }, [["page", "2"], ["tag", "pro"], ["tag", "beta"]]],
      ["deep object array", { filter: { city: "Paris", tags: ["pro", "beta"] } }, [["filter[city]", "Paris"], ["filter[tags][]", "pro"], ["filter[tags][]", "beta"]]],
      ["empty string and nullish", { filter: { city: "", zip: null } }, [["filter[city]", ""]]],"#
    } else {
        r#"      ["tag/nullish", { tag: ["pro", null, " beta "] }, [["tag", "pro"], ["tag", "beta"]]],
      ["blank scalar", { q: "  ", page: 2 }, [["page", "2"]]],
      ["accent fold", { q: "naïve café" }, [["q", _asciiFold("naïve café")]]],"#
    };
    format!(
        r#"  {{
    const _cases: Array<[string, Record<string, unknown>, Array<[string, string]>]> = [
{query_cases}
    ];
    for (const [_label, _queryInput, _expectedPairs] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {call}, {args}, _expectedPairs, "query_pairs", "Query semantics (" + _label + ")");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_query_string_parser_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    if infer_ts_contract(func, param_types, ret_type, type_defs)
        != Some(ContractKind::QueryStringParser)
        || !has_query_nested_brackets_contract(func)
    {
        return String::new();
    }
    let query_args: Vec<&str> = if param_types.len() > 1 {
        vec!["_args[0]", "_args[1]"]
    } else {
        vec!["_args[0]"]
    };
    let call = ts_call_with_args(func, &query_args);
    let args = if param_types.len() > 1 {
        "[_queryInput, \"extended\"]"
    } else {
        "[_queryInput]"
    };
    format!(
        r#"  {{
    const _cases: Array<[string, string, Record<string, unknown>]> = [
      ["repeated scalar", "tag=pro&tag=beta", {{ tag: ["pro", "beta"] }}],
      ["deep object array", "filter[city]=Paris&filter[tags][]=pro&filter[tags][]=beta", {{ filter: {{ city: "Paris", tags: ["pro", "beta"] }} }}],
    ];
    for (const [_label, _queryInput, _expectedObject] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {call}, {args}, _expectedObject, "identity", "Query parse semantics (" + _label + ")");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_defaults_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    if !is_ts_defaults_semantic_target(func, param_types, ret_type, type_defs) {
        return String::new();
    }
    let call = ts_call_with_args(func, &["_args[0]", "_args[1]"]);
    format!(
        r#"  {{
    _semanticCase("{name}", (_args: unknown[]) => {call},
      () => [{{ a: null }}, {{ a: 1 }}], null, {{ property: "a" }}, "Defaults semantics (null target preserves value)");
    _semanticCase("{name}", (_args: unknown[]) => {call},
      () => [{{ a: undefined }}, {{ a: 1 }}], 1, {{ property: "a" }}, "Defaults semantics (undefined target accepts source)");
    _semanticCase("{name}", (_args: unknown[]) => {call},
      () => [{{}}, globalThis.Object.create({{ inherited: 7 }})], 7, {{ property: "inherited" }}, "Defaults semantics (inherited source keys)");
  }}
"#,
        name = func.name,
    )
}

fn ts_feature_flag_override_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
    type_defs: &TsNamedTypes<'_>,
) -> String {
    if param_types.len() != 1 || ret_type.trim() != "boolean" || !is_api_surface(func) {
        return String::new();
    }
    let flag_key = match feature_flag_key_from_function_name(&func.name) {
        Some(flag_key) => flag_key,
        None => return String::new(),
    };
    let flags_type = match ts_object_field_type(param_types[0].trim(), "flags", type_defs) {
        Some(flags_type) => flags_type,
        None => return String::new(),
    };
    let flag_value_type = match ts_object_field_type(&flags_type, &flag_key, type_defs) {
        Some(flag_value_type) => flag_value_type,
        None => return String::new(),
    };
    if !is_boolean_like_ts_type(&flag_value_type) {
        return String::new();
    }
    let call = ts_call_with_args(func, &["_args[0]"]);
    format!(
        r#"  {{
    const _flagKey = "{flag_key}";
    _semanticCase("{name}", (_args: unknown[]) => {call},
      [[{{}}], [{{ flags: null }}]], true, "boolean_equal", "Feature flag semantics (flags null)", true);
    _semanticCase("{name}", (_args: unknown[]) => {call},
      [[{{}}], [{{ flags: {{ [_flagKey]: null }} }}]], true, "boolean_equal", "Feature flag semantics (flag null)", true);
    _semanticCase("{name}", (_args: unknown[]) => {call},
      [{{ flags: {{ [_flagKey]: false }} }}], false, "bool", "Feature flag semantics (explicit false)");
  }}
"#,
        name = func.name,
    )
}

fn ts_semver_compare_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
) -> String {
    let lower = func.name.to_lowercase();
    if param_types.len() != 2
        || param_types[0].trim() != "string"
        || param_types[1].trim() != "string"
        || ret_type.trim() != "number"
        || !is_api_surface(func)
        || !ANTISYMMETRIC_NAME_CUES
            .iter()
            .any(|cue| lower.contains(cue))
        || !(lower.contains("version") || lower.contains("semver"))
    {
        return String::new();
    }
    let call = ts_call_with_args(func, &["_args[0]", "_args[1]"]);
    format!(
        r#"  {{
    const _cases: Array<[string, string, number]> = [
      ["1.0.0-beta.1", "1.0.0", -1],
      ["1.0.0-alpha", "1.0.0-alpha.1", -1],
      ["1.0.0-beta.11", "1.0.0-beta.2", 1],
      ["1.0.0+build.1", "1.0.0+build.9", 0],
    ];
    for (const [_left, _right, _expected] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {call}, [_left, _right], _expected, "sign", "Semver compare semantics");
      _semanticCase("{name}", (_args: unknown[]) => {call}, [_right, _left], -_expected, "sign", "Semver compare antisymmetry");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_semver_caret_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
) -> String {
    let lower = func.name.to_lowercase();
    if param_types.len() != 2
        || param_types[0].trim() != "string"
        || param_types[1].trim() != "string"
        || ret_type.trim() != "boolean"
        || !is_api_surface(func)
        || !lower.contains("caret")
    {
        return String::new();
    }
    let call = ts_call_with_args(func, &["_args[0]", "_args[1]"]);
    format!(
        r#"  {{
    const _cases: Array<[string, string, boolean]> = [
      ["1.3.0-beta.1", "^1.2.3", false],
      ["1.0.2-beta.3", "^1.0.2", false],
      ["0.3.0", "^0.2.3", false],
      ["0.2.9", "^0.2.3", true],
      ["0.0.4", "^0.0.3", false],
    ];
    for (const [_version, _range, _expected] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {call}, [_version, _range], _expected, "bool", "Semver caret semantics");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_same_value_zero_semantic_check(
    func: &FunctionInfo,
    param_types: &[String],
    ret_type: &str,
) -> String {
    if !has_same_value_zero_contract(func) || param_types.len() != 2 || ret_type.trim() != "boolean"
    {
        return String::new();
    }
    let call = ts_call_with_args(func, &["_args[0]", "_args[1]"]);
    format!(
        r#"  {{
    const _cases: Array<[string, unknown, unknown, boolean]> = [
      ["NaN equals NaN", NaN, NaN, true],
      ["zero sign ignored", 0, -0, true],
      ["same scalar", "a", "a", true],
      ["different scalar", "a", "b", false],
    ];
    for (const [_label, _left, _right, _expected] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {call}, [_left, _right], _expected, "bool", "SameValueZero semantics (" + _label + ")");
      _semanticCase("{name}", (_args: unknown[]) => {call}, [_right, _left], _expected, "bool", "SameValueZero symmetry (" + _label + ")");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_http_request_metadata_semantic_check(func: &FunctionInfo, param_types: &[String]) -> String {
    if !has_http_request_metadata_contract(func) || param_types.is_empty() {
        return String::new();
    }
    let first = param_types[0].to_lowercase();
    if !first.contains("request") && !first.contains("req") {
        return String::new();
    }
    let request_call = ts_call_with_args(func, &["_args[0]"]);
    format!(
        r#"  {{
    const _requestArgs = () => [{{
      method: "GET",
      url: "/?user[name]=tj&user[roles][0]=admin",
      headers: {{
        Host: "example.test",
        "X-Requested-With": "XMLHttpRequest",
        "X-Forwarded-Proto": "https, http",
      }},
      app: {{ __settings: new globalThis.Map<string, unknown>([["query parser", "extended"], ["trust proxy", true]]) }},
    }}];
    const _cases: Array<[string, unknown, (_request: any) => unknown]> = [
      ["header lookup", "example.test", (_request: any) => _request.get("host")],
      ["header alias and xhr", ["XMLHttpRequest", true], (_request: any) => [_request.header("x-requested-with"), _request.xhr]],
      ["trusted forwarded protocol", ["https", true], (_request: any) => [_request.protocol, _request.secure]],
      ["extended query decoration", {{ user: {{ name: "tj", roles: ["admin"] }} }}, (_request: any) => _request.query],
    ];
    for (const [_label, _expected, _project] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {{ {request_call}; return _args[0]; }},
        _requestArgs, _expected, _project, "HTTP request metadata (" + _label + ")");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_http_response_helpers_semantic_check(func: &FunctionInfo, param_types: &[String]) -> String {
    if !has_http_response_helpers_contract(func) || param_types.is_empty() {
        return String::new();
    }
    let first = param_types[0].to_lowercase();
    if !first.contains("response") && !first.contains("res") {
        return String::new();
    }
    let args: Vec<&str> = if param_types.len() > 1 {
        vec!["_args[0]", "_args[1]"]
    } else {
        vec!["_args[0]"]
    };
    let response_call = ts_call_with_args(func, &args);
    let recipe = if param_types.len() > 1 {
        r#"() => [{}, { method: "GET", headers: { referer: "/from" } }]"#
    } else {
        "() => [{}]"
    };
    format!(
        r#"  {{
    const _responseArgs = {recipe};
    const _cases: Array<[string, unknown, (_response: any, _step: (label: string) => void) => unknown]> = [
      ["location encodes spaces", "/a%20path/with%20spaces", (_response: any, _step: (label: string) => void) => {{
        _step("location"); _response.location("/a path/with spaces");
        _step("getHeader"); return _response.getHeader("Location");
      }}],
      ["vary merges case-insensitively", "Accept-Encoding, Accept", (_response: any, _step: (label: string) => void) => {{
        _step("vary:0"); _response.vary("Accept-Encoding");
        _step("vary:1"); _response.vary("accept-encoding, Accept");
        _step("getHeader"); return _response.getHeader("Vary");
      }}],
      ["sendStatus 204 empty body", [204, ""], (_response: any, _step: (label: string) => void) => {{
        _step("sendStatus"); _response.sendStatus(204);
        _step("read_status_body"); return [_response.statusCode, String(_response.__body ?? "")];
      }}],
    ];
    for (const [_label, _expected, _project] of _cases) {{
      _semanticCase("{name}", (_args: unknown[]) => {{ {response_call}; return _args[0]; }},
        _responseArgs, _expected, _project, "HTTP response helpers (" + _label + ")");
    }}
  }}
"#,
        name = func.name,
    )
}

fn ts_http_static_file_semantic_check(func: &FunctionInfo, param_types: &[String]) -> String {
    if !has_http_static_file_middleware_contract(func)
        || param_types.is_empty()
        || param_types[0].trim() != "string"
    {
        return String::new();
    }
    let factory_call = ts_call_with_args(func, &["_args[0]"]);

    format!(
        r#"  {{
    const _staticArgs = () => [process.cwd() + "/static"];
    _semanticCase("{name}", (_args: unknown[]) => {factory_call}, _staticArgs,
      [true, false, "hello world\n"], (_handler: any, _step: (label: string) => void) => {{
      _step("handler_shape");
      if (typeof _handler !== "function") {{
        throw new Error("HTTP static file middleware: factory did not return a handler");
      }}
      let _nextCalled = false;
      const _request: any = {{ method: "GET", url: "/hello.txt" }};
      const _response: any = {{
        statusCode: 200,
        headersSent: false,
        __headers: new globalThis.Map<string, string>(),
        setHeader(name: string, value: string) {{ this.__headers.set(name.toLowerCase(), value); }},
        getHeader(name: string) {{ return this.__headers.get(name.toLowerCase()); }},
        end(body?: unknown) {{ this.headersSent = true; this.__body = body ?? ""; }},
        send(body?: unknown) {{ this.end(body ?? ""); return this; }},
      }};
      _step("handler");
      _handler(_request, _response, () => {{ _nextCalled = true; }});
      _step("body");
      const _body = String(_response.__body ?? "");
      _step("headersSent");
      return [Boolean(_response.headersSent), _nextCalled, _body];
    }}, "HTTP static file middleware (serve known file)");
  }}
"#,
        name = func.name,
        factory_call = factory_call,
    )
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
                    .is_some_and(|ann| ann.trim() == "number");
            }
            "minor" => {
                has_minor = field
                    .type_annotation
                    .as_deref()
                    .is_some_and(|ann| ann.trim() == "number");
            }
            "patch" => {
                has_patch = field
                    .type_annotation
                    .as_deref()
                    .is_some_and(|ann| ann.trim() == "number");
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

fn ts_field_generator_with_stack(
    field_name: &str,
    type_ann: Option<&str>,
    type_defs: &TsNamedTypes<'_>,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    if depth >= TS_TYPE_RECURSION_LIMIT {
        return "({})".into();
    }
    if is_semver_part_field(field_name, type_ann) {
        return "_fuzzSemverPart()".into();
    }
    if is_semver_prerelease_field(field_name, type_ann) {
        return "[null, [], [_fuzzSemverIdentifier()], [_fuzzSemverIdentifier(), _fuzzSemverIdentifier()]][_fuzzIntRange(0, 3)]".into();
    }
    let base = ts_generator_with_stack(type_ann, type_defs, stack, depth + 1);
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

fn ts_inline_object_generator_with_stack(
    type_ann: &str,
    type_defs: &TsNamedTypes<'_>,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    if depth >= TS_TYPE_RECURSION_LIMIT {
        return "({})".into();
    }
    let fields = extract_ts_object_type_fields_from_text(type_ann);
    if fields.is_empty() {
        return "({})".into();
    }

    let props: Vec<String> = fields
        .iter()
        .map(|field| {
            let field_gen = ts_field_generator_with_stack(
                field.name.as_str(),
                field.type_annotation.as_deref(),
                type_defs,
                stack,
                depth + 1,
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

const TYPESCRIPT_FUZZ_PRELUDE: &str = include_str!("synthesize/typescript/prelude.ts");
const TYPESCRIPT_FUZZ_EPILOGUE: &str = include_str!("synthesize/typescript/epilogue.ts");

// ── Helpers ─────────────────────────────────────────────────────────────────

fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

fn literal_choice_expr(choices: &[String], picker: &str) -> String {
    if choices.is_empty() {
        return "undefined".into();
    }
    if choices.len() == 1 {
        return choices[0].clone();
    }
    format!(
        "[{}][{}(0, {})]",
        choices.join(", "),
        picker,
        choices.len() - 1
    )
}

fn strip_quoted_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let mut chars = trimmed.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    if !trimmed.ends_with(quote) || trimmed.len() < 2 {
        return None;
    }
    let inner = &trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()];
    serde_json::to_string(inner).ok()
}

fn split_top_level_args(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in text.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
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

fn numeric_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|ch| !(ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | '_')))
    {
        return None;
    }
    let normalized = trimmed.replace('_', "");
    normalized.parse::<f64>().ok()?;
    Some(normalized)
}

fn ts_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(quoted) = strip_quoted_literal(trimmed) {
        return Some(quoted);
    }
    match trimmed {
        "true" | "false" | "null" | "undefined" => Some(trimmed.to_string()),
        _ => numeric_literal_expr(trimmed),
    }
}

fn python_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(quoted) = strip_quoted_literal(trimmed) {
        return Some(quoted);
    }
    match trimmed {
        "True" => Some("True".into()),
        "False" => Some("False".into()),
        "None" => Some("None".into()),
        _ => numeric_literal_expr(trimmed),
    }
}

fn python_literal_choice_exprs(type_ann: &str) -> Option<Vec<String>> {
    let trimmed = type_ann.trim();
    if !starts_with_any(
        trimmed,
        &["Literal[", "typing.Literal[", "typing_extensions.Literal["],
    ) {
        return None;
    }

    let inner = extract_generic_arg(trimmed);
    let choices: Vec<String> = split_top_level_args(&inner, ',')
        .into_iter()
        .filter_map(python_literal_expr)
        .collect();
    if choices.is_empty() {
        None
    } else {
        Some(choices)
    }
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

fn split_ts_top_level(text: &str, separator: char) -> Vec<&str> {
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

fn ts_type_contains_unknown_leaf(type_ann: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    fn inner(
        type_ann: &str,
        type_defs: &TsNamedTypes<'_>,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> bool {
        if depth >= TS_TYPE_RECURSION_LIMIT {
            return false;
        }
        let trimmed = type_ann.trim();
        if matches!(trimmed, "unknown" | "any") {
            return true;
        }
        if let Some(resolved) = ts_resolve_alias_text(trimmed, type_defs) {
            if resolved != trimmed {
                return inner(&resolved, type_defs, stack, depth + 1);
            }
        }
        let union_branches = split_ts_top_level(trimmed, '|');
        if union_branches.len() > 1 {
            return union_branches
                .iter()
                .any(|branch| inner(branch, type_defs, stack, depth + 1));
        }
        if trimmed.ends_with("[]") {
            return inner(
                trimmed.trim_end_matches("[]").trim(),
                type_defs,
                stack,
                depth + 1,
            );
        }
        if trimmed.starts_with("Array<") || trimmed.starts_with("ReadonlyArray<") {
            return inner(&extract_generic_arg(trimmed), type_defs, stack, depth + 1);
        }
        if looks_like_ts_object_type(trimmed) {
            return extract_ts_object_type_fields_from_text(trimmed)
                .iter()
                .any(|field| {
                    field
                        .type_annotation
                        .as_deref()
                        .is_some_and(|field_type| inner(field_type, type_defs, stack, depth + 1))
                });
        }
        if let Some(class) = ts_class_def(trimmed, type_defs) {
            if ts_type_seen(stack, trimmed) {
                return false;
            }
            stack.push(trimmed.to_string());
            let contains = class.fields.iter().any(|field| {
                field
                    .type_annotation
                    .as_deref()
                    .is_some_and(|field_type| inner(field_type, type_defs, stack, depth + 1))
            });
            stack.pop();
            return contains;
        }
        false
    }

    inner(type_ann, type_defs, &mut Vec::new(), 0)
}

fn ts_edge_type_name(type_ann: Option<&str>, type_defs: &TsNamedTypes<'_>) -> &'static str {
    let t = match type_ann {
        Some(t) => t.trim(),
        None => return "",
    };
    if let Some(resolved) = ts_resolve_alias_text(t, type_defs) {
        return ts_edge_type_name(Some(&resolved), type_defs);
    }
    if matches!(t, "unknown" | "any") {
        return "unknown";
    }
    if (t.ends_with("[]") || t.starts_with("Array<") || t.starts_with("ReadonlyArray<"))
        && ts_type_contains_unknown_leaf(t, type_defs)
    {
        return "unknown_array";
    }
    if (looks_like_ts_object_type(t) || ts_class_def(t, type_defs).is_some())
        && ts_type_contains_unknown_leaf(t, type_defs)
    {
        // The typed generator already varies unknown-valued fields while preserving every
        // required sibling. A shape-blind object edge would also replace required literals
        // and can turn an invalid object into a purportedly valid invocation.
        return "";
    }
    if ts_type_contains_literal_domain(t, type_defs) {
        return "";
    }

    match Some(t) {
        Some("number") => "number",
        Some("string") => "string",
        Some("string[]") | Some("Array<string>") => "string_array",
        Some(t) if is_string_array_like_type(t) => "string_array",
        Some(t)
            if t.ends_with("[]") || t.starts_with("Array<") || t.starts_with("ReadonlyArray<") =>
        {
            ""
        }
        Some(t) if t.starts_with("Record<") => "object",
        Some(t) if looks_like_ts_object_type(t) || ts_class_def(t, type_defs).is_some() => "",
        _ => "",
    }
}

fn ts_type_contains_literal_domain(type_ann: &str, type_defs: &TsNamedTypes<'_>) -> bool {
    fn inner(
        type_ann: &str,
        type_defs: &TsNamedTypes<'_>,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> bool {
        if depth >= TS_TYPE_RECURSION_LIMIT {
            return false;
        }
        let trimmed = type_ann.trim();
        if let Some(TsNamedTypeRef::Alias(alias)) = type_defs.get(trimmed) {
            if ts_type_seen(stack, trimmed) {
                return false;
            }
            stack.push(trimmed.to_string());
            let contains = inner(&alias.type_annotation, type_defs, stack, depth + 1);
            stack.pop();
            return contains;
        }

        let union_branches = split_ts_top_level(trimmed, '|');
        if union_branches.len() > 1 {
            return union_branches.iter().any(|branch| {
                let branch = branch.trim();
                !matches!(branch, "null" | "undefined")
                    && (ts_literal_expr(branch).is_some()
                        || inner(branch, type_defs, stack, depth + 1))
            });
        }

        if ts_literal_expr(trimmed).is_some() && !matches!(trimmed, "null" | "undefined") {
            return true;
        }
        if trimmed.ends_with("[]") {
            let inner_type = trimmed.trim_end_matches("[]").trim();
            return inner(inner_type, type_defs, stack, depth + 1);
        }
        if trimmed.starts_with("Array<") || trimmed.starts_with("ReadonlyArray<") {
            let inner_type = extract_generic_arg(trimmed);
            return inner(&inner_type, type_defs, stack, depth + 1);
        }
        if looks_like_ts_object_type(trimmed) {
            return extract_ts_object_type_fields_from_text(trimmed)
                .iter()
                .any(|field| {
                    field
                        .type_annotation
                        .as_deref()
                        .is_some_and(|field_type| inner(field_type, type_defs, stack, depth + 1))
                });
        }
        if let Some(class) = ts_class_def(trimmed, type_defs) {
            if ts_type_seen(stack, trimmed) {
                return false;
            }
            stack.push(trimmed.to_string());
            let contains = class.fields.iter().any(|field| {
                field
                    .type_annotation
                    .as_deref()
                    .is_some_and(|field_type| inner(field_type, type_defs, stack, depth + 1))
            });
            stack.pop();
            return contains;
        }
        false
    }

    inner(type_ann, type_defs, &mut vec![], 0)
}

fn is_string_array_like_type(type_ann: &str) -> bool {
    let trimmed = type_ann.trim();
    if trimmed.ends_with("[]") {
        let inner = trimmed.trim_end_matches("[]").trim();
        return is_string_like_union(inner);
    }
    if trimmed.starts_with("Array<") || trimmed.starts_with("ReadonlyArray<") {
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

fn find_involution_pairs(analysis: &AnalysisResult) -> Vec<(&FunctionInfo, &FunctionInfo)> {
    let candidates = synth_candidate_functions(&analysis.functions);
    let func_map: HashMap<String, &FunctionInfo> = analysis
        .functions
        .iter()
        .map(|f| (f.name.to_lowercase(), f))
        .collect();

    let mut result = vec![];
    let mut seen: Vec<String> = vec![];

    for func in candidates {
        let params: Vec<_> = func.params.iter().filter(|p| !p.is_variadic()).collect();
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
                let partner_params: Vec<_> =
                    partner.params.iter().filter(|p| !p.is_variadic()).collect();
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
        let param = enc.params.iter().find(|p| !p.is_variadic()).unwrap();
        let gen = python_generator(param.type_annotation.as_deref(), type_defs);
        let pair_label = serde_json::to_string(&format!("{}<->{}", enc.name, dec.name))
            .unwrap_or_else(|_| "\"roundtrip\"".into());
        let corpus_key = serde_json::to_string(&format!("{}:{}", enc.name, enc.line))
            .unwrap_or_else(|_| "\"\"".into());

        code.push_str(&format!(
            r#"
# Involution roundtrip: {enc_name} <-> {dec_name}
_inv_inputs = [{gen} for _ in range(30)]
_inv_inputs.extend(_copy.deepcopy(_row[0]) for _row in _CJ_CORPORA.get({corpus_key}, []) if _row)
for _inv_input in _inv_inputs:
    try:
        _inv_encoded = {enc_name}(_copy.deepcopy(_inv_input))
        _inv_decoded = {dec_name}(_copy.deepcopy(_inv_encoded))
        if not _nan_eq(_inv_input, _inv_decoded):
            _roundtrip_error = AssertionError(f"Roundtrip failed: {{repr(_inv_input)}} -> {{repr(_inv_encoded)}} -> {{repr(_inv_decoded)}}")
            _emit_finding({pair_label}, [_inv_input], _roundtrip_error, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", case_label="roundtrip")
            print(f"  ROUNDTRIP FAIL {enc_name} <-> {dec_name}: {{_short_repr(_inv_input)}} -> {{_short_repr(_inv_encoded)}} -> {{_short_repr(_inv_decoded)}}")
            _fuzz_failures += 1
            break
    except Exception as _e:
        if _is_crash(_e):
            _emit_finding({pair_label}, [_inv_input], _e, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", case_label="roundtrip")
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
        let param = enc.params.iter().find(|p| !p.is_variadic()).unwrap();
        let Some(gen) = ts_generator(param.type_annotation.as_deref(), type_defs) else {
            continue;
        };
        let pair_label = serde_json::to_string(&format!("{}<->{}", enc.name, dec.name))
            .unwrap_or_else(|_| "\"roundtrip\"".into());
        let corpus_key = serde_json::to_string(&format!("{}:{}", enc.name, enc.line))
            .unwrap_or_else(|_| "\"\"".into());
        let encode_call = ts_call_with_args(enc, &["input"]);
        let decode_call = ts_call_with_args(dec, &["encoded"]);

        code.push_str(&format!(
            r#"
// Involution roundtrip: {enc_name} <-> {dec_name}
{{
  let _invFail = false;
  const _invInputs: unknown[] = Array.from({{ length: 30 }}, () => {gen});
  for (const row of _cjCorpora.get({corpus_key}) ?? []) {{
    if (row.length > 0) _invInputs.push(_cloneSeed(row[0]));
  }}
  for (const input of _invInputs) {{
    try {{
      const encoded = {encode_call};
      const decoded = {decode_call};
      if (!_nanSafeEq(input, decoded)) {{
        const failure = new Error(`Roundtrip failed: ${{_shortJson(input)}} -> ${{_shortJson(encoded)}} -> ${{_shortJson(decoded)}}`);
        _emitFinding({pair_label}, [input], failure, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", null, "direct", "roundtrip", {source_line});
        console.log(`  ROUNDTRIP FAIL {enc_name} <-> {dec_name}: ${{_shortJson(input)}} -> ${{_shortJson(encoded)}} -> ${{_shortJson(decoded)}}`);
        _fuzzTotalFailures++;
        _invFail = true;
        break;
      }}
    }} catch (e: unknown) {{
      if (_isCrash(e)) {{
        _emitFinding({pair_label}, [input], e, "property_violation", "inferred_semantic", "name_heuristic", "low", "property", null, "direct", "roundtrip", {source_line});
        console.log(`  ROUNDTRIP CRASH {enc_name} <-> {dec_name}: ${{_clipText(e)}}`);
        _fuzzTotalFailures++;
        _invFail = true;
        break;
      }}
    }}
  }}
  if (!_invFail) console.log("FUZZ {enc_name} <-> {dec_name} roundtrip: passed");
}}
"#,
            enc_name = enc.name,
            dec_name = dec.name,
            source_line = enc.line,
        ));
    }

    code
}

#[derive(Debug, Clone)]
pub struct NativeFuzzPlan {
    pub code: String,
    pub engine: NativeFuzzEngine,
    pub target_count: usize,
}

fn python_native_argument(type_annotation: Option<&str>) -> Option<&'static str> {
    let normalized = type_annotation
        .unwrap_or("str")
        .to_ascii_lowercase()
        .replace(' ', "");
    match normalized.as_str() {
        "bool" => Some("_cj_data.ConsumeBool()"),
        "int" => Some("_cj_data.ConsumeInt(8)"),
        "float" => Some("_cj_data.ConsumeFloat()"),
        "str" | "string" | "any" => Some("_cj_data.ConsumeUnicodeNoSurrogates(64)"),
        "bytes" => Some("_cj_data.ConsumeBytes(64)"),
        "bytearray" => Some("bytearray(_cj_data.ConsumeBytes(64))"),
        "list[int]" | "typing.list[int]" => Some("_cj_data.ConsumeIntList(8, 8)"),
        "list[str]" | "typing.list[str]" => Some(
            "[_cj_data.ConsumeUnicodeNoSurrogates(16) for _ in range(_cj_data.ConsumeIntInRange(0, 4))]",
        ),
        "list[bool]" | "typing.list[bool]" => Some(
            "[_cj_data.ConsumeBool() for _ in range(_cj_data.ConsumeIntInRange(0, 8))]",
        ),
        _ => None,
    }
}

fn typescript_native_argument(type_annotation: Option<&str>) -> Option<&'static str> {
    let normalized = type_annotation
        .unwrap_or("unknown")
        .to_ascii_lowercase()
        .replace(' ', "");
    match normalized.as_str() {
        "number" => Some("_cj_data.number()"),
        "string" | "unknown" | "any" => Some("_cj_data.string()"),
        "boolean" | "bool" => Some("_cj_data.boolean()"),
        "bigint" => Some("BigInt(_cj_data.integer())"),
        "uint8array" => Some("_cj_data.bytes()"),
        "buffer" => Some("Buffer.from(_cj_data.bytes())"),
        "number[]" | "array<number>" => Some("_cj_data.array(() => _cj_data.number())"),
        "string[]" | "array<string>" => Some("_cj_data.array(() => _cj_data.string())"),
        "boolean[]" | "array<boolean>" => Some("_cj_data.array(() => _cj_data.boolean())"),
        "date" => Some("new Date(Math.abs(_cj_data.integer()) % 4102444800000)"),
        _ => None,
    }
}

fn synthesize_python_native_fuzz(selected_functions: &[&FunctionInfo]) -> Option<NativeFuzzPlan> {
    let targets: Vec<(&FunctionInfo, Vec<&'static str>)> = selected_functions
        .iter()
        .copied()
        .filter(|func| !func.is_nested && !func.is_method)
        .filter_map(|func| {
            let arguments = func
                .params
                .iter()
                .filter(|param| !param.is_variadic())
                .map(|param| python_native_argument(param.type_annotation.as_deref()))
                .collect::<Option<Vec<_>>>()?;
            Some((func, arguments))
        })
        .collect();
    if targets.is_empty() {
        return None;
    }

    let mut code = format!(
        r#"

# -- Court Jester optional Atheris adapter ------------------------------------
import atheris as _cj_atheris
import json as _cj_native_json
import sys as _cj_native_sys

def _cj_native_value(value):
    try:
        return _cj_native_json.loads(
            _cj_native_json.dumps(value, ensure_ascii=False, allow_nan=False)
        )
    except Exception:
        return repr(value)

def _cj_native_emit(function, line, arguments, error, data):
    payload = {{
        "function": function,
        "line": line,
        "arguments": [_cj_native_value(value) for value in arguments],
        "input": bytes(data).hex(),
        "error_type": type(error).__name__,
        "message": str(error),
    }}
    print("__COURT_JESTER_NATIVE_FINDING__" + _cj_native_json.dumps(payload, ensure_ascii=False))

def TestOneInput(data):
    _cj_data = _cj_atheris.FuzzedDataProvider(data)
    _cj_target = _cj_data.ConsumeIntInRange(0, {last_target})
"#,
        last_target = targets.len() - 1
    );

    for (index, (func, arguments)) in targets.iter().enumerate() {
        let branch = if index == 0 { "if" } else { "elif" };
        let call_args = func
            .params
            .iter()
            .filter(|param| !param.is_variadic())
            .enumerate()
            .map(|(argument_index, param)| {
                if param.keyword_only {
                    format!("{}=_cj_args[{argument_index}]", param.name)
                } else {
                    format!("_cj_args[{argument_index}]")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let function_label =
            serde_json::to_string(&func.name).unwrap_or_else(|_| "\"unknown\"".into());
        let _ = writeln!(
            code,
            "    {branch} _cj_target == {index}:\n        _cj_args = [{}]\n        try:\n            {}({call_args})\n        except Exception as _cj_error:\n            _cj_native_emit({function_label}, {}, _cj_args, _cj_error, data)\n            raise",
            arguments.join(", "),
            func.name,
            func.line,
        );
    }
    code.push_str(
        "\n_cj_atheris.instrument_all()\n_cj_atheris.Setup(_cj_native_sys.argv, TestOneInput)\n_cj_atheris.Fuzz()\n",
    );

    Some(NativeFuzzPlan {
        code,
        engine: NativeFuzzEngine::Atheris,
        target_count: targets.len(),
    })
}

fn synthesize_typescript_native_fuzz(
    selected_functions: &[&FunctionInfo],
) -> Option<NativeFuzzPlan> {
    let targets: Vec<(&FunctionInfo, Vec<&'static str>)> = selected_functions
        .iter()
        .copied()
        .filter(|func| is_api_surface(func))
        .filter_map(|func| {
            let arguments = func
                .params
                .iter()
                .filter(|param| !param.is_variadic())
                .map(|param| typescript_native_argument(param.type_annotation.as_deref()))
                .collect::<Option<Vec<_>>>()?;
            Some((func, arguments))
        })
        .collect();
    if targets.is_empty() {
        return None;
    }

    let mut code = format!(
        r#"

// -- Court Jester optional Jazzer.js adapter ---------------------------------
class _CourtJesterNativeInput {{
  private offset = 1;
  constructor(private readonly data: Uint8Array) {{}}
  private take(length: number): Uint8Array {{
    const available = Math.max(0, Math.min(length, this.data.length - this.offset));
    const value = this.data.slice(this.offset, this.offset + available);
    this.offset += available;
    return value;
  }}
  integer(): number {{
    const bytes = this.take(6);
    let value = 0;
    for (const byte of bytes) value = value * 256 + byte;
    return value - 2 ** 47;
  }}
  number(): number {{
    const value = this.integer();
    return this.boolean() ? value / 1000 : value;
  }}
  boolean(): boolean {{
    return (this.take(1)[0] ?? 0) % 2 === 1;
  }}
  bytes(): Uint8Array {{
    const length = (this.take(1)[0] ?? 0) % 65;
    return this.take(length);
  }}
  string(): string {{
    return new TextDecoder("utf-8", {{ fatal: false }}).decode(this.bytes());
  }}
  array<T>(generate: () => T): T[] {{
    const length = (this.take(1)[0] ?? 0) % 9;
    return Array.from({{ length }}, generate);
  }}
}}

function _cjNativeValue(value: unknown): unknown {{
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Uint8Array) return Array.from(value);
  try {{
    return JSON.parse(JSON.stringify(value));
  }} catch {{
    return String(value);
  }}
}}

function _cjNativeEmit(functionName: string, line: number, args: unknown[], error: unknown, input: Uint8Array): void {{
  const payload = {{
    function: functionName,
    line,
    arguments: args.map(_cjNativeValue),
    input: Array.from(input, (byte) => byte.toString(16).padStart(2, "0")).join(""),
    error_type: error instanceof Error ? error.constructor.name : typeof error,
    message: error instanceof Error ? error.message : String(error),
  }};
  console.log("__COURT_JESTER_NATIVE_FINDING__" + JSON.stringify(payload));
}}

export async function fuzz(data: Uint8Array): Promise<void> {{
  const _cj_data = new _CourtJesterNativeInput(data);
  const _cj_target = (data[0] ?? 0) % {target_count};
"#,
        target_count = targets.len()
    );

    for (index, (func, arguments)) in targets.iter().enumerate() {
        let branch = if index == 0 { "if" } else { "else if" };
        let arg_names = (0..arguments.len())
            .map(|argument_index| format!("_cj_args[{argument_index}]"))
            .collect::<Vec<_>>();
        let arg_refs = arg_names.iter().map(String::as_str).collect::<Vec<_>>();
        let call = ts_call_with_args(func, &arg_refs);
        let function_label =
            serde_json::to_string(&func.name).unwrap_or_else(|_| "\"unknown\"".into());
        let _ = writeln!(
            code,
            "  {branch} (_cj_target === {index}) {{\n    const _cj_args: unknown[] = [{}];\n    try {{\n      await {call};\n    }} catch (_cj_error: unknown) {{\n      _cjNativeEmit({function_label}, {}, _cj_args, _cj_error, data);\n      throw _cj_error;\n    }}\n  }}",
            arguments.join(", "),
            func.line,
        );
    }
    code.push_str("}\n");

    Some(NativeFuzzPlan {
        code,
        engine: NativeFuzzEngine::Jazzer,
        target_count: targets.len(),
    })
}

pub fn synthesize_native_fuzz(
    language: &Language,
    selected_functions: &[&FunctionInfo],
    engine: NativeFuzzEngine,
) -> Option<NativeFuzzPlan> {
    match (language, engine) {
        (Language::Python, NativeFuzzEngine::Auto | NativeFuzzEngine::Atheris) => {
            synthesize_python_native_fuzz(selected_functions)
        }
        (Language::TypeScript, NativeFuzzEngine::Auto | NativeFuzzEngine::Jazzer) => {
            synthesize_typescript_native_fuzz(selected_functions)
        }
        _ => None,
    }
}
