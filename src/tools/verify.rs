use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

use crate::tools::{analyze, diff, lint, sandbox, synthesize};
use crate::types::*;

/// Options for the verify pipeline to avoid parameter sprawl.
pub struct VerifyOptions<'a> {
    pub test_code: Option<&'a str>,
    pub test_source_file: Option<&'a str>,
    pub test_runner: TestRunner,
    pub tests_only: bool,
    pub complexity_threshold: Option<usize>,
    pub complexity_metric: ComplexityMetric,
    pub project_dir: Option<&'a str>,
    pub lint_config_path: Option<&'a str>,
    pub lint_virtual_file_path: Option<&'a str>,
    pub diff: Option<&'a str>,
    pub suppressions: Option<&'a str>,
    pub suppression_source: Option<&'a str>,
    pub auto_seed: bool,
    /// Original source file path — when set, fuzz code is written as a sibling
    /// so relative imports resolve correctly.
    pub source_file: Option<&'a str>,
    pub output_dir: Option<&'a str>,
    pub report_level: ReportLevel,
    pub execute_gate: ExecuteGate,
}

/// Default execute-stage timeout for synthesized Python fuzz harnesses (seconds).
/// Overridable via `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS`.
const DEFAULT_PYTHON_EXEC_TIMEOUT: f64 = 10.0;

/// Default execute-stage timeout for synthesized TypeScript fuzz harnesses (seconds).
/// TypeScript is slower to boot (Node transform/loader startup plus transpile),
/// so it gets a longer default. Overridable via
/// `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS`.
const DEFAULT_TYPESCRIPT_EXEC_TIMEOUT: f64 = 25.0;

/// Default test-stage timeout (seconds). Overridable via
/// `COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS`.
const DEFAULT_TEST_TIMEOUT: f64 = 30.0;

fn env_timeout(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(default)
}

fn execute_timeout_for(language: &Language) -> f64 {
    match language {
        Language::Python => env_timeout(
            "COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS",
            DEFAULT_PYTHON_EXEC_TIMEOUT,
        ),
        Language::TypeScript => env_timeout(
            "COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS",
            DEFAULT_TYPESCRIPT_EXEC_TIMEOUT,
        ),
    }
}

fn test_timeout() -> f64 {
    env_timeout(
        "COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS",
        DEFAULT_TEST_TIMEOUT,
    )
}

fn test_code_has_imports(code: &str, language: &Language) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim_start();
        match language {
            Language::Python => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("from ")
                    || trimmed.contains("importlib.import_module(")
            }
            Language::TypeScript => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("export ")
                    || trimmed.contains("require(")
            }
        }
    })
}

fn err_execution_result(message: &str) -> ExecutionResult {
    ExecutionResult {
        stdout: String::new(),
        stderr: message.to_string(),
        exit_code: None,
        duration_ms: 0,
        timed_out: false,
        memory_error: false,
    }
}

fn function_key(func: &FunctionInfo) -> (String, usize) {
    (func.name.clone(), func.line)
}

fn coverage_counts(entries: &[FuzzFunctionCoverage]) -> serde_json::Value {
    let mut counts = serde_json::Map::new();
    for status in [
        FuzzFunctionStatus::Fuzzed,
        FuzzFunctionStatus::FuzzedViaFactory,
        FuzzFunctionStatus::SkippedNoFuzzableSurface,
        FuzzFunctionStatus::SkippedUnsupportedType,
        FuzzFunctionStatus::SkippedInternalHelper,
        FuzzFunctionStatus::SkippedMethod,
        FuzzFunctionStatus::SkippedNested,
        FuzzFunctionStatus::SkippedPrivateName,
        FuzzFunctionStatus::SkippedDiffFiltered,
        FuzzFunctionStatus::BlockedModuleLoad,
    ] {
        let key = serde_json::to_value(&status)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "unknown".into());
        let count = entries
            .iter()
            .filter(|entry| entry.status == status)
            .count();
        counts.insert(key, serde_json::Value::from(count));
    }
    serde_json::Value::Object(counts)
}

fn finalize_fuzz_coverage(
    analysis_functions: &[FunctionInfo],
    allowed_functions: &[FunctionInfo],
    planned_coverage: &[FuzzFunctionCoverage],
    module_load_blocked: bool,
) -> Vec<FuzzFunctionCoverage> {
    let allowed: HashSet<(String, usize)> = allowed_functions.iter().map(function_key).collect();
    let mut planned: HashMap<(String, usize), FuzzFunctionCoverage> = planned_coverage
        .iter()
        .cloned()
        .map(|entry| ((entry.function.clone(), entry.line), entry))
        .collect();

    let mut coverage = Vec::with_capacity(analysis_functions.len());
    for func in analysis_functions {
        let key = function_key(func);
        let entry = if !allowed.contains(&key) {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedDiffFiltered,
                Some("excluded by diff scoping".into()),
            )
        } else if func.is_method && func.invocation_target.is_none() {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedMethod,
                Some("methods are not fuzzed directly".into()),
            )
        } else if func.is_nested && func.invocation_target.is_none() {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedNested,
                Some(
                    "nested functions are exercised via their parent factory when possible".into(),
                ),
            )
        } else if func.name.starts_with('_') {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedPrivateName,
                Some("underscore-prefixed helpers are skipped".into()),
            )
        } else if let Some(mut planned_entry) = planned.remove(&key) {
            if module_load_blocked && planned_entry.status == FuzzFunctionStatus::Fuzzed {
                planned_entry.status = FuzzFunctionStatus::BlockedModuleLoad;
                planned_entry.reason =
                    Some("module load failed before the fuzz harness ran".into());
            }
            planned_entry
        } else {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some("function was not selected for fuzzing".into()),
            )
        };
        coverage.push(entry);
    }

    let mut synthetic_entries: Vec<_> = planned.into_values().collect();
    synthetic_entries.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.function.cmp(&right.function))
    });
    coverage.extend(synthetic_entries);

    coverage
}

fn coverage_entry_for_verify(
    func: &FunctionInfo,
    status: FuzzFunctionStatus,
    reason: Option<String>,
) -> FuzzFunctionCoverage {
    FuzzFunctionCoverage {
        function: func.name.clone(),
        line: func.line,
        end_line: func.end_line,
        status,
        is_exported: func.is_exported,
        reason,
    }
}

fn is_typescript_portability_error(stderr: &str) -> bool {
    stderr.contains("ERR_MODULE_NOT_FOUND")
        || stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
        || stderr.contains("Cannot find module 'bun'")
        || stderr.contains("Cannot find package 'bun'")
        || stderr.contains("Bun is not defined")
        || stderr.contains("needs an import attribute of \"type: json\"")
}

fn is_typescript_module_load_error(stderr: &str) -> bool {
    is_typescript_portability_error(stderr)
        || stderr.contains("Cannot find module")
        || stderr.contains("Cannot find package")
        || stderr.contains("The requested module")
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SuppressionsFile {
    #[serde(default)]
    rules: Vec<SuppressionRule>,
}

#[derive(Debug, Clone, Deserialize)]
struct SuppressionRule {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    function: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct SuppressionContext<'a> {
    source_file: Option<&'a str>,
    stage: &'a str,
    function: Option<&'a str>,
    severity: Option<&'a str>,
    error_type: Option<&'a str>,
    reason: Option<&'a str>,
}

fn parse_suppressions(raw: Option<&str>) -> SuppressionsFile {
    raw.and_then(|value| serde_json::from_str::<SuppressionsFile>(value).ok())
        .unwrap_or_default()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn suppression_matches(rule: &SuppressionRule, ctx: SuppressionContext<'_>) -> bool {
    if let Some(rule_path) = rule.path.as_deref() {
        let Some(source_file) = ctx.source_file else {
            return false;
        };
        let source = normalize_path(source_file);
        let candidate = normalize_path(rule_path);
        if !source.ends_with(candidate.as_str()) {
            return false;
        }
    }

    if let Some(stage) = rule.stage.as_deref() {
        if stage != ctx.stage {
            return false;
        }
    }

    if let Some(function) = rule.function.as_deref() {
        if Some(function) != ctx.function {
            return false;
        }
    }

    if let Some(severity) = rule.severity.as_deref() {
        if Some(severity) != ctx.severity {
            return false;
        }
    }

    if let Some(error_type) = rule.error_type.as_deref() {
        if Some(error_type) != ctx.error_type {
            return false;
        }
    }

    if let Some(reason) = rule.reason.as_deref() {
        if Some(reason) != ctx.reason {
            return false;
        }
    }

    true
}

fn split_fuzz_failures(
    failures: Vec<FuzzFailure>,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
) -> (Vec<FuzzFailure>, Vec<FuzzFailure>) {
    let mut active = Vec::new();
    let mut suppressed = Vec::new();

    for failure in failures {
        let ctx = SuppressionContext {
            source_file,
            stage: "execute",
            function: Some(failure.function.as_str()),
            severity: Some(failure.severity.as_str()),
            error_type: Some(failure.error_type.as_str()),
            reason: None,
        };
        if suppressions
            .rules
            .iter()
            .any(|rule| suppression_matches(rule, ctx))
        {
            suppressed.push(failure);
        } else {
            active.push(failure);
        }
    }

    (active, suppressed)
}

fn split_complexity_violations(
    violations: Vec<ComplexityViolation>,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
    code: &str,
    language: &Language,
) -> (
    Vec<ComplexityViolation>,
    Vec<ComplexityViolation>,
    Vec<String>,
) {
    let mut active = Vec::new();
    let mut suppressed = Vec::new();
    let mut source_directive_functions = Vec::new();

    for violation in violations {
        if analyze::source_directive_suppresses_complexity(code, language, violation.line) {
            source_directive_functions.push(violation.function.clone());
            suppressed.push(violation);
            continue;
        }

        let ctx = SuppressionContext {
            source_file,
            stage: "complexity",
            function: Some(violation.function.as_str()),
            severity: None,
            error_type: None,
            reason: None,
        };
        if suppressions
            .rules
            .iter()
            .any(|rule| suppression_matches(rule, ctx))
        {
            suppressed.push(violation);
        } else {
            active.push(violation);
        }
    }

    source_directive_functions.sort();
    source_directive_functions.dedup();

    (active, suppressed, source_directive_functions)
}

fn portability_reason(stderr: &str) -> &'static str {
    if stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
        || stderr.contains("needs an import attribute of \"type: json\"")
    {
        "err_import_attribute_missing"
    } else if stderr.contains("Cannot find module 'bun'")
        || stderr.contains("Cannot find package 'bun'")
        || stderr.contains("Bun is not defined")
    {
        "bun_runtime_dependency"
    } else if stderr.contains("ERR_MODULE_NOT_FOUND") {
        "err_module_not_found"
    } else {
        "unknown_portability_error"
    }
}

fn collect_portability_imports(stderr: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for prefix in ["Cannot find module '", "Cannot find package '"] {
        for section in stderr.split(prefix).skip(1) {
            if let Some((candidate, _)) = section.split_once('\'') {
                let candidate = candidate.trim();
                if !candidate.is_empty() && !imports.iter().any(|item| item == candidate) {
                    imports.push(candidate.to_string());
                }
            }
        }
    }
    imports
}

fn portability_fix_hint(reason: &str) -> &'static str {
    match reason {
        "err_module_not_found" => {
            "Add explicit Node ESM file extensions for relative imports, or rely on the repo-native runtime when the repo is intentionally Bun-specific."
        }
        "err_import_attribute_missing" => {
            "Add the required import attribute for JSON modules, for example `with { type: \"json\" }` in Node ESM."
        }
        "bun_runtime_dependency" => {
            "The file depends on Bun-only globals or packages. Keep portability advisory-only, or run the repo-native runtime for behavior checks."
        }
        _ => "Review the Node stderr to see which import or runtime assumption blocks strict Node execution.",
    }
}

fn build_portability_detail(
    repo_runtime: &str,
    node_result: &ExecutionResult,
    repo_result: &ExecutionResult,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
    suppression_source: Option<&str>,
) -> (serde_json::Value, bool) {
    let reason = portability_reason(&node_result.stderr).to_string();
    let failing_imports = collect_portability_imports(&node_result.stderr);
    let suppressed = suppressions.rules.iter().any(|rule| {
        suppression_matches(
            rule,
            SuppressionContext {
                source_file,
                stage: "portability",
                function: None,
                severity: None,
                error_type: None,
                reason: Some(reason.as_str()),
            },
        )
    });

    (
        serde_json::json!({
            "reason": reason,
            "failing_imports": failing_imports,
            "fix_hint": portability_fix_hint(portability_reason(&node_result.stderr)),
            "suppressed": suppressed,
            "suppression_source": suppression_source,
            "repo_runtime": repo_runtime,
            "node_result": serde_json::to_value(node_result).unwrap(),
            "repo_result": serde_json::to_value(repo_result).unwrap(),
        }),
        suppressed,
    )
}

#[derive(Debug, Clone)]
struct ObservedArg {
    code: String,
    literal_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct ObservedCall {
    function: String,
    args: Vec<ObservedArg>,
    source_label: String,
}

fn parser_for_language(language: &Language) -> Option<Parser> {
    let mut parser = Parser::new();
    match language {
        Language::Python => parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .ok()?,
        Language::TypeScript => parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .ok()?,
    }
    Some(parser)
}

fn node_text(node: &tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn primitive_literal_value(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<Option<serde_json::Value>> {
    let code = node_text(node, source);
    match language {
        Language::TypeScript => match node.kind() {
            "number" => Some(
                code.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number),
            ),
            "true" => Some(Some(serde_json::Value::Bool(true))),
            "false" => Some(Some(serde_json::Value::Bool(false))),
            "null" => Some(Some(serde_json::Value::Null)),
            "undefined" | "string" => Some(None),
            _ => None,
        },
        Language::Python => match node.kind() {
            "integer" => Some(code.parse::<i64>().ok().map(serde_json::Value::from)),
            "float" => Some(
                code.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number),
            ),
            "true" => Some(Some(serde_json::Value::Bool(true))),
            "false" => Some(Some(serde_json::Value::Bool(false))),
            "none" | "string" => Some(None),
            _ => None,
        },
    }
}

fn is_literal_like_arg(node: &tree_sitter::Node, language: &Language, source: &[u8]) -> bool {
    if primitive_literal_value(node, language, source).is_some() {
        return true;
    }

    match (language, node.kind()) {
        (Language::TypeScript, "array") | (Language::Python, "list" | "tuple" | "set") => {
            let mut cursor = node.walk();
            let all_literal = node.named_children(&mut cursor).all(|child| {
                !matches!(
                    child.kind(),
                    "spread_element" | "list_splat" | "dictionary_splat"
                ) && is_literal_like_arg(&child, language, source)
            });
            all_literal
        }
        (Language::TypeScript, "object") | (Language::Python, "dictionary") => {
            let mut cursor = node.walk();
            let all_literal = node.named_children(&mut cursor).all(|child| {
                if matches!(
                    child.kind(),
                    "spread_element" | "list_splat" | "dictionary_splat"
                ) {
                    return false;
                }
                if child.kind() == "pair" {
                    return child
                        .child_by_field_name("value")
                        .or_else(|| child.named_child(child.named_child_count().saturating_sub(1)))
                        .is_some_and(|value| is_literal_like_arg(&value, language, source));
                }
                false
            });
            all_literal
        }
        (Language::TypeScript, "unary_expression") | (Language::Python, "unary_operator") => {
            let text = node_text(node, source);
            matches!(text.trim().chars().next(), Some('-' | '+'))
                && node.named_child_count() == 1
                && node.named_child(0).is_some_and(|child| {
                    primitive_literal_value(&child, language, source).is_some()
                })
        }
        _ => false,
    }
}

fn parse_literal_arg(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<ObservedArg> {
    let code = node_text(node, source);
    let literal_value = primitive_literal_value(node, language, source)
        .or_else(|| is_literal_like_arg(node, language, source).then_some(None))?;
    Some(ObservedArg {
        code,
        literal_value,
    })
}

fn extract_literal_args(
    arguments_node: tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<Vec<ObservedArg>> {
    let mut args = Vec::new();
    let mut cursor = arguments_node.walk();
    for child in arguments_node.named_children(&mut cursor) {
        match (language, child.kind()) {
            (Language::Python, "keyword_argument")
            | (Language::Python, "list_splat")
            | (Language::TypeScript, "spread_element") => return None,
            _ => {}
        }
        args.push(parse_literal_arg(&child, language, source)?);
    }
    Some(args)
}

fn callee_function_name(
    callee: tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<String> {
    if callee.kind() == "identifier" {
        return Some(node_text(&callee, source));
    }

    match language {
        Language::TypeScript if callee.kind() == "member_expression" => callee
            .child_by_field_name("property")
            .filter(|property| matches!(property.kind(), "property_identifier" | "identifier"))
            .map(|property| node_text(&property, source)),
        Language::Python if callee.kind() == "attribute" => callee
            .child_by_field_name("attribute")
            .filter(|attribute| attribute.kind() == "identifier")
            .map(|attribute| node_text(&attribute, source)),
        _ => None,
    }
}

fn collect_observed_calls_recursive(
    node: tree_sitter::Node,
    language: &Language,
    source: &[u8],
    function_names: &HashSet<String>,
    source_label: &str,
    out: &mut Vec<ObservedCall>,
) {
    let is_call = matches!(
        (language, node.kind()),
        (Language::Python, "call") | (Language::TypeScript, "call_expression")
    );
    if is_call {
        let callee = node.child_by_field_name("function");
        let arguments = node.child_by_field_name("arguments");
        if let (Some(callee), Some(arguments)) = (callee, arguments) {
            if let Some(name) = callee_function_name(callee, language, source) {
                if function_names.contains(&name) {
                    if let Some(args) = extract_literal_args(arguments, language, source) {
                        out.push(ObservedCall {
                            function: name,
                            args,
                            source_label: source_label.to_string(),
                        });
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_observed_calls_recursive(
            child,
            language,
            source,
            function_names,
            source_label,
            out,
        );
    }
}

fn collect_observed_calls(
    code: &str,
    language: &Language,
    function_names: &HashSet<String>,
    source_label: &str,
) -> Vec<ObservedCall> {
    let Some(mut parser) = parser_for_language(language) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(code, None) else {
        return Vec::new();
    };
    let mut observed = Vec::new();
    collect_observed_calls_recursive(
        tree.root_node(),
        language,
        code.as_bytes(),
        function_names,
        source_label,
        &mut observed,
    );
    observed
}

fn discover_seed_files(source_file: &str, language: &Language) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) else {
        return Vec::new();
    };
    let Some(dir) = source_path.parent() else {
        return Vec::new();
    };
    let parent = dir.parent().unwrap_or(dir);
    let mut candidates = Vec::new();
    match language {
        Language::TypeScript => {
            for name in [format!("{stem}.test.ts"), format!("{stem}.spec.ts")] {
                candidates.push(dir.join(&name));
                candidates.push(dir.join("__tests__").join(&name));
                candidates.push(parent.join("tests").join(&name));
                candidates.push(parent.join("__tests__").join(&name));
            }
        }
        Language::Python => {
            for name in [format!("test_{stem}.py"), format!("{stem}_test.py")] {
                candidates.push(dir.join(&name));
                candidates.push(dir.join("tests").join(&name));
                candidates.push(parent.join("tests").join(&name));
            }
        }
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn json_value_to_literal(value: &serde_json::Value, language: &Language) -> String {
    match language {
        Language::TypeScript => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
        Language::Python => match value {
            serde_json::Value::Null => "None".into(),
            serde_json::Value::Bool(value) => {
                if *value {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::String(value) => {
                serde_json::to_string(value).unwrap_or_else(|_| "''".into())
            }
            serde_json::Value::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|item| json_value_to_literal(item, language))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            serde_json::Value::Object(values) => format!(
                "{{{}}}",
                values
                    .iter()
                    .map(|(key, item)| format!(
                        "{}: {}",
                        serde_json::to_string(key).unwrap_or_else(|_| "''".into()),
                        json_value_to_literal(item, language)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    }
}

fn json_value_as_observed_arg(value: &serde_json::Value, language: &Language) -> ObservedArg {
    let literal_value = match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(value.clone())
        }
        _ => None,
    };
    ObservedArg {
        code: json_value_to_literal(value, language),
        literal_value,
    }
}

fn candidate_fixture_function_name(
    source_file: &str,
    function_names: &HashSet<String>,
) -> Option<String> {
    let stem = Path::new(source_file)
        .file_stem()
        .and_then(|value| value.to_str())?;
    if function_names.contains(stem) {
        return Some(stem.to_string());
    }
    if function_names.len() == 1 {
        return function_names.iter().next().cloned();
    }
    None
}

fn fixture_json_paths(source_file: &str, project_dir: Option<&str>) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) else {
        return Vec::new();
    };
    let Some(source_dir) = source_path.parent() else {
        return Vec::new();
    };
    let parent = source_dir.parent().unwrap_or(source_dir);
    let filename = format!("{stem}.json");
    let mut candidates = vec![
        source_dir.join(&filename),
        source_dir.join("fixtures").join(&filename),
        source_dir.join("examples").join(&filename),
        source_dir.join("tests").join(&filename),
        parent.join("fixtures").join(&filename),
        parent.join("examples").join(&filename),
        parent.join("tests").join(&filename),
    ];

    if let Some(project_dir) = project_dir {
        let root = Path::new(project_dir);
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if !is_ignored_project_seed_dir(&path) {
                        stack.push(path);
                    }
                    continue;
                }
                if path.file_name().and_then(|value| value.to_str()) == Some(filename.as_str()) {
                    candidates.push(path);
                }
            }
        }
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| path.metadata().map(|meta| meta.len()).unwrap_or(0) <= 512 * 1024)
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn collect_json_fixture_observations(
    source_file: &str,
    project_dir: Option<&str>,
    language: &Language,
    function_names: &HashSet<String>,
) -> Vec<ObservedCall> {
    let Some(function) = candidate_fixture_function_name(source_file, function_names) else {
        return Vec::new();
    };
    let mut observed = Vec::new();
    for path in fixture_json_paths(source_file, project_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let args_value = match value {
                serde_json::Value::Array(mut row) if row.len() == 2 && row[0].is_array() => {
                    row.remove(0)
                }
                serde_json::Value::Array(row) => serde_json::Value::Array(row),
                _ => continue,
            };
            let serde_json::Value::Array(args) = args_value else {
                continue;
            };
            observed.push(ObservedCall {
                function: function.clone(),
                args: args
                    .iter()
                    .map(|arg| json_value_as_observed_arg(arg, language))
                    .collect(),
                source_label: path.to_string_lossy().to_string(),
            });
        }
    }
    observed
}

fn json_fixture_rows(
    source_file: &str,
    project_dir: Option<&str>,
    function_names: &HashSet<String>,
) -> Vec<(String, Vec<serde_json::Value>, serde_json::Value)> {
    let Some(function) = candidate_fixture_function_name(source_file, function_names) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for path in fixture_json_paths(source_file, project_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let serde_json::Value::Array(mut row) = value else {
                continue;
            };
            if row.len() != 2 || !row[0].is_array() {
                continue;
            }
            let expected = row.pop().unwrap_or(serde_json::Value::Null);
            let Some(serde_json::Value::Array(args)) = row.pop() else {
                continue;
            };
            rows.push((function.clone(), args, expected));
        }
    }
    rows
}

fn json_value_is_primitive_sortable(value: &serde_json::Value) -> bool {
    value.is_number() || value.is_string() || value.is_boolean()
}

fn json_array_is_sorted_primitive(values: &[serde_json::Value]) -> bool {
    if values.is_empty() {
        return true;
    }
    if values.iter().all(serde_json::Value::is_number) {
        let nums = values
            .iter()
            .filter_map(serde_json::Value::as_f64)
            .collect::<Vec<_>>();
        return nums.len() == values.len()
            && nums
                .windows(2)
                .all(|pair| pair[0].is_finite() && pair[1].is_finite() && pair[0] <= pair[1]);
    }
    if values.iter().all(serde_json::Value::is_string) {
        let strings = values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        return strings.windows(2).all(|pair| pair[0] <= pair[1]);
    }
    if values
        .iter()
        .any(|value| !json_value_is_primitive_sortable(value))
    {
        return false;
    }
    let mut rendered = values
        .iter()
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>();
    let original = rendered.clone();
    rendered.sort();
    original == rendered
}

fn json_multiset_key(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn json_arrays_same_multiset(left: &[serde_json::Value], right: &[serde_json::Value]) -> bool {
    let mut counts: HashMap<String, isize> = HashMap::new();
    for value in left {
        *counts.entry(json_multiset_key(value)).or_default() += 1;
    }
    for value in right {
        let entry = counts.entry(json_multiset_key(value)).or_default();
        *entry -= 1;
    }
    counts.values().all(|count| *count == 0)
}

fn json_sequence_is_palindrome(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => {
            values.len() > 1 && values.iter().eq(values.iter().rev())
        }
        serde_json::Value::String(value) => {
            value.len() > 1 && value.chars().eq(value.chars().rev())
        }
        _ => false,
    }
}

const MIN_FIXTURE_PROPERTY_SUPPORT: usize = 2;

fn fixture_row_key(args: &[serde_json::Value], expected: &serde_json::Value) -> String {
    let args = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
    let expected = serde_json::to_string(expected).unwrap_or_else(|_| "null".into());
    format!("{args}=>{expected}")
}

fn fixture_property_support_count<F>(
    rows: &[(Vec<serde_json::Value>, serde_json::Value)],
    predicate: F,
) -> usize
where
    F: Fn(&[serde_json::Value], &serde_json::Value) -> bool,
{
    let mut seen = HashSet::new();
    for (args, expected) in rows {
        if predicate(args, expected) {
            seen.insert(fixture_row_key(args, expected));
        }
    }
    seen.len()
}

fn fixture_property_has_support<F>(
    rows: &[(Vec<serde_json::Value>, serde_json::Value)],
    predicate: F,
) -> bool
where
    F: Fn(&[serde_json::Value], &serde_json::Value) -> bool,
{
    fixture_property_support_count(rows, predicate) >= MIN_FIXTURE_PROPERTY_SUPPORT
}

fn infer_fixture_properties(
    source_file: &str,
    project_dir: Option<&str>,
    function_names: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let rows = json_fixture_rows(source_file, project_dir, function_names);
    let mut grouped: HashMap<String, Vec<(Vec<serde_json::Value>, serde_json::Value)>> =
        HashMap::new();
    for (function, args, expected) in rows {
        grouped.entry(function).or_default().push((args, expected));
    }

    let mut inferred = HashMap::new();
    for (function, rows) in grouped {
        if rows.is_empty() {
            continue;
        }
        let mut properties = Vec::new();
        let sorted_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            expected
                .as_array()
                .is_some_and(|values| json_array_is_sorted_primitive(values))
        };
        let nontrivial_sorted_output =
            |args: &[serde_json::Value], expected: &serde_json::Value| {
                sorted_output(args, expected)
                    && expected.as_array().is_some_and(|values| values.len() >= 2)
            };
        if rows
            .iter()
            .all(|(args, expected)| sorted_output(args, expected))
            && fixture_property_has_support(&rows, nontrivial_sorted_output)
        {
            properties.push("sorted".to_string());
        }

        let permutation_output = |args: &[serde_json::Value], expected: &serde_json::Value| {
            let Some(input) = args.first().and_then(|value| value.as_array()) else {
                return false;
            };
            let Some(output) = expected.as_array() else {
                return false;
            };
            json_arrays_same_multiset(input, output)
        };
        let nontrivial_permutation_output =
            |args: &[serde_json::Value], expected: &serde_json::Value| {
                let Some(input) = args.first().and_then(|value| value.as_array()) else {
                    return false;
                };
                let Some(output) = expected.as_array() else {
                    return false;
                };
                input.len() >= 2 && output.len() >= 2 && json_arrays_same_multiset(input, output)
            };
        if rows
            .iter()
            .all(|(args, expected)| permutation_output(args, expected))
            && fixture_property_has_support(&rows, nontrivial_permutation_output)
        {
            properties.push("permutation".to_string());
        }

        let nonnegative_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            expected
                .as_f64()
                .is_some_and(|value| value >= 0.0 && value.is_finite())
        };
        if rows
            .iter()
            .all(|(args, expected)| nonnegative_output(args, expected))
            && fixture_property_has_support(&rows, nonnegative_output)
            && rows.iter().any(|(_, expected)| {
                expected
                    .as_f64()
                    .is_some_and(|value| value > 0.0 && value.is_finite())
            })
        {
            properties.push("nonneg".to_string());
        }

        let palindrome_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            json_sequence_is_palindrome(expected)
        };
        if function.to_lowercase().contains("palindrome")
            && rows
                .iter()
                .all(|(args, expected)| palindrome_output(args, expected))
            && fixture_property_has_support(&rows, palindrome_output)
        {
            properties.push("palindrome".to_string());
        }
        if !properties.is_empty() {
            inferred.insert(function, properties);
        }
    }
    inferred
}

fn apply_inferred_properties(
    functions: &mut [FunctionInfo],
    inferred: &HashMap<String, Vec<String>>,
) {
    for function in functions {
        let Some(properties) = inferred.get(&function.name) else {
            continue;
        };
        for property in properties {
            if !function
                .declared_properties
                .iter()
                .any(|existing| existing == property)
            {
                function.declared_properties.push(property.clone());
            }
        }
    }
}

const QUERY_NESTED_BRACKETS_PROPERTY: &str = "query_nested_brackets";
const SAME_VALUE_ZERO_PROPERTY: &str = "same_value_zero";
const PEP440_VERSION_ORDERING_PROPERTY: &str = "pep440_version_ordering";
const PEP440_SPECIFIER_MEMBERSHIP_PROPERTY: &str = "pep440_specifier_membership";
const PEP440_FILTER_PRERELEASE_PROPERTY: &str = "pep440_filter_prerelease";
const COOKIE_VALUE_QUOTE_PROPERTY: &str = "cookie_value_quote";
const COOKIE_HEADER_QUOTE_PROPERTY: &str = "cookie_header_quote";
const HTTP_REQUEST_METADATA_PROPERTY: &str = "http_request_metadata";
const HTTP_RESPONSE_HELPERS_PROPERTY: &str = "http_response_helpers";
const HTTP_STATIC_FILE_MIDDLEWARE_PROPERTY: &str = "http_static_file_middleware";

fn push_inferred_property(
    inferred: &mut HashMap<String, Vec<String>>,
    function: &str,
    property: &str,
) {
    let properties = inferred.entry(function.to_string()).or_default();
    if !properties.iter().any(|existing| existing == property) {
        properties.push(property.to_string());
    }
}

fn ts_annotation_is_string_like(type_annotation: Option<&str>) -> bool {
    type_annotation
        .map(str::trim)
        .is_some_and(|value| value == "string" || value == "str")
}

fn ts_annotation_is_structured_or_unknown(type_annotation: Option<&str>) -> bool {
    let Some(value) = type_annotation.map(str::trim) else {
        return true;
    };
    matches!(value, "unknown" | "any" | "object")
        || value.starts_with("Record<")
        || value.starts_with("dict[")
        || value.starts_with("Dict[")
        || value.starts_with('{')
}

fn ts_annotation_is_mapping_like(type_annotation: Option<&str>) -> bool {
    let Some(value) = type_annotation.map(str::trim) else {
        return false;
    };
    value.starts_with("Record<")
        || value.starts_with("dict[")
        || value.starts_with("Dict[")
        || value.starts_with('{')
}

fn function_can_accept_query_nested_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    let query_name_context = lower.contains("query") || lower.contains("urlencoded");
    if !query_name_context {
        return false;
    }

    let first_param_type = func
        .params
        .iter()
        .find(|param| !param.name.starts_with('*'))
        .and_then(|param| param.type_annotation.as_deref());
    let return_type = func.return_type.as_deref();

    let parse_like = (lower.contains("parse") || lower.contains("decode"))
        && ts_annotation_is_string_like(first_param_type)
        && ts_annotation_is_structured_or_unknown(return_type);
    let stringify_like = [
        "stringify",
        "serialize",
        "serialise",
        "canonical",
        "canonicalize",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
        && ts_annotation_is_mapping_like(first_param_type)
        && ts_annotation_is_string_like(return_type);

    parse_like || stringify_like
}

fn text_suggests_query_nested_brackets_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let query_context = lower.contains("query")
        || lower.contains("query-string")
        || lower.contains("query string")
        || lower.contains("qs.")
        || lower.contains("urlencoded")
        || lower.contains("url-encoded")
        || lower.contains("urlsearchparams")
        || lower.contains("bracket notation")
        || lower.contains("form parsing")
        || lower.contains("form parser")
        || lower.contains("form body")
        || lower.contains("body parsing")
        || lower.contains("body parser");
    let nested_context = [
        "bracket notation",
        "extended parsing",
        "extended parser",
        "extended urlencoded",
        "nested query",
        "nested queries",
        "nested object",
        "nested objects",
        "nested array",
        "nested arrays",
        "nested form",
        "nested forms",
        "nested collection",
        "nested collections",
        "object-plus-array",
        "larger arrays",
        "arrays of objects",
        "duplicate nested",
        "[] suffix",
        "[tags][]",
        "filter[",
        "%5b",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    query_context && nested_context
}

fn function_can_accept_http_request_metadata_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("request") && (lower.contains("decorate") || lower.contains("metadata"))) {
        return false;
    }
    func.params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| {
            let lower = annotation.to_lowercase();
            lower.contains("request") || lower.contains("req")
        })
        .unwrap_or(false)
}

fn text_suggests_http_request_metadata_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let request_context = lower.contains("request metadata")
        || lower.contains("request introspection")
        || lower.contains("request decoration")
        || lower.contains("request helpers")
        || lower.contains("req.get")
        || lower.contains("req.header")
        || lower.contains("req.xhr");
    let behavior_context = [
        "header lookup",
        "xhr detection",
        "trust proxy",
        "forwarded-proto",
        "x-forwarded-proto",
        "protocol",
        "secure",
        "query-parser request decoration",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    request_context && behavior_context
}

fn function_can_accept_http_response_helpers_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("response") && (lower.contains("decorate") || lower.contains("helper"))) {
        return false;
    }
    func.params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| annotation.to_lowercase().contains("response"))
        .unwrap_or(false)
}

fn text_suggests_http_response_helpers_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let response_context = lower.contains("response header")
        || lower.contains("response helper")
        || lower.contains("response metadata")
        || lower.contains("status helpers")
        || lower.contains("sendstatus")
        || lower.contains("res.location")
        || lower.contains("res.vary");
    let behavior_context = [
        "location",
        "link header",
        "vary",
        "sendstatus",
        "status helper",
        "empty response body",
        "header composition",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    response_context && behavior_context
}

fn function_can_accept_http_static_file_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("static") && (lower.contains("middleware") || lower.contains("serve"))) {
        return false;
    }
    let first_param_is_root = func
        .params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| annotation.trim() == "string" || annotation.trim() == "str")
        .unwrap_or(false);
    let returns_handler = func
        .return_type
        .as_deref()
        .map(|annotation| {
            let lower = annotation.to_lowercase();
            lower.contains("handler") || lower.contains("middleware") || lower.contains("function")
        })
        .unwrap_or(false);
    first_param_is_root && returns_handler
}

fn text_suggests_http_static_file_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let static_context = lower.contains("static-file")
        || lower.contains("static file")
        || lower.contains("static serving")
        || lower.contains("static-file wrapper")
        || lower.contains("static root");
    let file_context = [
        "serve known files",
        "serving a known static file",
        "serving an existing file",
        "serve an existing file",
        "serve known file",
        "static/",
        "hello.txt",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    static_context && file_context
}

fn text_suggests_same_value_zero_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("samevaluezero")
        || lower.contains("same value zero")
        || lower.contains("same_value_zero")
}

fn text_suggests_pep440_context(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("pep 440")
        || lower.contains("pep440")
        || lower.contains("pypa/packaging")
        || lower.contains("packaging.version")
        || lower.contains("packaging specifier")
}

fn text_suggests_pep440_version_ordering(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("version-ordering")
            || lower.contains("version ordering")
            || lower.contains("compare_versions")
            || lower.contains("test_version.py")
            || (lower.contains("dev releases") && lower.contains("post releases")))
}

fn text_suggests_pep440_specifier_membership(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("specifier-set")
            || lower.contains("specifier behavior")
            || lower.contains("allows(version")
            || lower.contains("compatible release")
            || lower.contains("~="))
}

fn text_suggests_pep440_filter_prerelease(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("specifier.filter")
            || lower.contains("filter_versions")
            || lower.contains("prerelease fallback")
            || lower.contains("only matching candidates"))
}

fn source_suggests_cookie_quote_context(source_file: &str, source_text: Option<&str>) -> bool {
    let path = source_file.replace('\\', "/").to_lowercase();
    if path.contains("cookie") || path.ends_with("/_quote.py") || path.ends_with("/quote.py") {
        return true;
    }
    let Some(text) = source_text else {
        return false;
    };
    let lower = text.to_lowercase();
    lower.contains("cookie") && lower.contains("quote")
}

fn push_context_dir_candidates(dir: &Path, candidates: &mut Vec<PathBuf>) {
    for name in [
        "README.md",
        "readme.md",
        "UPSTREAM_NOTES.md",
        "CONTRACT.md",
        "contract.md",
        "API.md",
        "api.md",
    ] {
        candidates.push(dir.join(name));
    }

    let docs = dir.join("docs");
    for name in ["README.md", "readme.md", "CONTRACT.md", "contract.md"] {
        candidates.push(docs.join(name));
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten().take(40) {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("md") {
                candidates.push(path);
            }
        }
    }
}

fn discover_context_contract_files(source_file: &str, project_dir: Option<&str>) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let mut dirs = Vec::new();
    if let Some(dir) = source_path.parent() {
        dirs.push(dir.to_path_buf());
        if let Some(parent) = dir.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Some(root) = project_dir {
        dirs.push(PathBuf::from(root));
    }

    let mut candidates = Vec::new();
    for dir in dirs {
        push_context_dir_candidates(&dir, &mut candidates);
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| path.metadata().map(|meta| meta.len()).unwrap_or(0) <= 256 * 1024)
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn infer_context_properties(
    source_file: &str,
    project_dir: Option<&str>,
    functions: &[FunctionInfo],
) -> HashMap<String, Vec<String>> {
    let query_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_query_nested_context(func))
        .collect();
    let request_metadata_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_request_metadata_context(func))
        .collect();
    let response_helper_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_response_helpers_context(func))
        .collect();
    let static_file_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_static_file_context(func))
        .collect();
    let source_text = std::fs::read_to_string(source_file).ok();
    let has_pep440_candidate = functions.iter().any(|func| {
        let lower = func.name.to_lowercase();
        (lower.contains("compare") && lower.contains("version"))
            || lower == "allows"
            || (lower.contains("filter") && lower.contains("version"))
    });
    if query_candidates.is_empty()
        && request_metadata_candidates.is_empty()
        && response_helper_candidates.is_empty()
        && static_file_candidates.is_empty()
        && !functions.iter().any(|func| {
            func.name
                .to_lowercase()
                .replace('_', "")
                .replace('-', "")
                .contains("samevaluezero")
        })
        && !has_pep440_candidate
        && !source_suggests_cookie_quote_context(source_file, source_text.as_deref())
    {
        return HashMap::new();
    }

    let mut inferred = HashMap::new();
    if source_suggests_cookie_quote_context(source_file, source_text.as_deref()) {
        for func in functions {
            match func.name.as_str() {
                "format_cookie_value" => {
                    push_inferred_property(&mut inferred, &func.name, COOKIE_VALUE_QUOTE_PROPERTY)
                }
                "build_cookie_header" => {
                    push_inferred_property(&mut inferred, &func.name, COOKIE_HEADER_QUOTE_PROPERTY)
                }
                _ => {}
            }
        }
    }
    for path in discover_context_contract_files(source_file, project_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = text.to_lowercase();
        if text_suggests_query_nested_brackets_contract(&text) {
            for func in &query_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || query_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        QUERY_NESTED_BRACKETS_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_request_metadata_contract(&text) {
            for func in &request_metadata_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || request_metadata_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_REQUEST_METADATA_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_response_helpers_contract(&text) {
            for func in &response_helper_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || response_helper_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_RESPONSE_HELPERS_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_static_file_contract(&text) {
            for func in &static_file_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || static_file_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_STATIC_FILE_MIDDLEWARE_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_same_value_zero_contract(&text) {
            for func in functions {
                let normalized_name = func.name.to_lowercase().replace('_', "").replace('-', "");
                if normalized_name.contains("samevaluezero") {
                    push_inferred_property(&mut inferred, &func.name, SAME_VALUE_ZERO_PROPERTY);
                }
            }
        }
        if text_suggests_pep440_version_ordering(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower.contains("compare") && lower.contains("version") {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_VERSION_ORDERING_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_pep440_specifier_membership(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower == "allows" || (lower.contains("allow") && lower.contains("specifier")) {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_SPECIFIER_MEMBERSHIP_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_pep440_filter_prerelease(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower.contains("filter") && lower.contains("version") {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_FILTER_PRERELEASE_PROPERTY,
                    );
                }
            }
        }
    }
    inferred
}

fn is_ignored_project_seed_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | ".hg"
                    | ".svn"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | "coverage"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
            )
        })
}

fn is_probably_test_seed_file(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.contains("/tests/")
        || normalized.contains("/__tests__/")
        || normalized.contains("/test/")
    {
        return true;
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.starts_with("test_")
                || name.ends_with("_test.py")
                || name.contains(".test.")
                || name.contains(".spec.")
        })
}

fn is_supported_project_seed_file(path: &Path, language: &Language) -> bool {
    match language {
        Language::TypeScript => {
            path.extension().and_then(|value| value.to_str()) == Some("ts")
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with(".d.ts"))
        }
        Language::Python => path.extension().and_then(|value| value.to_str()) == Some("py"),
    }
}

fn discover_project_seed_files(
    source_file: &str,
    project_dir: Option<&str>,
    language: &Language,
) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let root = project_dir
        .map(PathBuf::from)
        .or_else(|| source_path.parent().map(Path::to_path_buf));
    let Some(root) = root else {
        return Vec::new();
    };

    let source_canonical = source_path.canonicalize().ok();
    let mut stack = vec![root];
    let mut candidates = Vec::new();

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_ignored_project_seed_dir(&path) {
                    stack.push(path);
                }
                continue;
            }
            if candidates.len() >= 80 {
                continue;
            }
            if !is_supported_project_seed_file(&path, language) || is_probably_test_seed_file(&path)
            {
                continue;
            }
            if path.metadata().map(|meta| meta.len()).unwrap_or(0) > 256 * 1024 {
                continue;
            }
            if source_canonical
                .as_ref()
                .is_some_and(|source| path.canonicalize().ok().as_ref() == Some(source))
            {
                continue;
            }
            candidates.push(path);
        }
    }

    candidates.sort();
    candidates
}

fn collect_seed_observations(
    code: &str,
    language: &Language,
    functions: &[FunctionInfo],
    source_file: Option<&str>,
    project_dir: Option<&str>,
    explicit_test_code: Option<&str>,
    explicit_test_file: Option<&str>,
    auto_seed: bool,
) -> Vec<ObservedCall> {
    let function_names: HashSet<String> = functions.iter().map(|func| func.name.clone()).collect();
    let mut observed = Vec::new();

    let primary_label = source_file.unwrap_or("<source>");
    observed.extend(collect_observed_calls(
        code,
        language,
        &function_names,
        primary_label,
    ));

    if let Some(test_code) = explicit_test_code {
        observed.extend(collect_observed_calls(
            test_code,
            language,
            &function_names,
            explicit_test_file.unwrap_or("<explicit-test>"),
        ));
    } else if auto_seed {
        if let Some(source_file) = source_file {
            for path in discover_seed_files(source_file, language) {
                if let Ok(test_code) = std::fs::read_to_string(&path) {
                    observed.extend(collect_observed_calls(
                        &test_code,
                        language,
                        &function_names,
                        &path.to_string_lossy(),
                    ));
                }
            }
        }
    }

    if auto_seed {
        if let Some(source_file) = source_file {
            observed.extend(collect_json_fixture_observations(
                source_file,
                project_dir,
                language,
                &function_names,
            ));
            for path in discover_project_seed_files(source_file, project_dir, language) {
                if let Ok(context_code) = std::fs::read_to_string(&path) {
                    observed.extend(collect_observed_calls(
                        &context_code,
                        language,
                        &function_names,
                        &path.to_string_lossy(),
                    ));
                }
            }
        }
    }

    observed
}

fn dedupe_seed_rows(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for row in rows {
        let key = row.join("\u{1f}");
        if seen.insert(key) {
            deduped.push(row.clone());
        }
    }
    deduped
}

fn seed_inputs_from_observations(
    functions: &[FunctionInfo],
    observed_calls: &[ObservedCall],
) -> HashMap<String, Vec<Vec<String>>> {
    let expected_arity: HashMap<String, usize> = functions
        .iter()
        .map(|func| {
            (
                func.name.clone(),
                func.params
                    .iter()
                    .filter(|param| !param.name.starts_with('*'))
                    .count(),
            )
        })
        .collect();
    let mut grouped: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for observed in observed_calls {
        let Some(expected) = expected_arity.get(&observed.function) else {
            continue;
        };
        if observed.args.len() != *expected {
            continue;
        }
        grouped
            .entry(observed.function.clone())
            .or_default()
            .push(observed.args.iter().map(|arg| arg.code.clone()).collect());
    }
    grouped
        .into_iter()
        .map(|(name, rows)| (name, dedupe_seed_rows(&rows)))
        .collect()
}

fn seed_sources(observed_calls: &[ObservedCall]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for observed in observed_calls {
        if seen.insert(observed.source_label.clone()) {
            sources.push(observed.source_label.clone());
        }
    }
    sources
}

fn total_seed_rows(seed_inputs: &HashMap<String, Vec<Vec<String>>>) -> usize {
    seed_inputs.values().map(Vec::len).sum()
}

fn classify_type_signature_failures(
    failures: &mut [FuzzFailure],
    observed_calls: &[ObservedCall],
    language: &Language,
) {
    if !matches!(language, Language::TypeScript) {
        return;
    }

    let mut by_function: HashMap<&str, Vec<&ObservedCall>> = HashMap::new();
    for observed in observed_calls {
        by_function
            .entry(observed.function.as_str())
            .or_default()
            .push(observed);
    }

    for failure in failures.iter_mut() {
        if failure.severity != "crash" || failure.classification.is_some() {
            continue;
        }
        let Some(observed) = by_function.get(failure.function.as_str()) else {
            continue;
        };
        let Ok(args) = serde_json::from_str::<Vec<serde_json::Value>>(&failure.input) else {
            continue;
        };

        for (index, arg) in args.iter().enumerate() {
            let Some(failing_number) = arg.as_f64() else {
                continue;
            };
            let mut observed_numbers = Vec::new();
            let mut saw_non_numeric = false;
            for call in observed {
                match call
                    .args
                    .get(index)
                    .and_then(|item| item.literal_value.as_ref())
                {
                    Some(value) if value.is_number() => {
                        if let Some(number) = value.as_f64() {
                            observed_numbers.push(number);
                        }
                    }
                    Some(_) | None => {
                        saw_non_numeric = true;
                        break;
                    }
                }
            }
            if saw_non_numeric || observed_numbers.is_empty() || observed_numbers.len() > 8 {
                continue;
            }
            if observed_numbers
                .iter()
                .any(|value| (*value - failing_number).abs() < f64::EPSILON)
            {
                continue;
            }

            let mut preview = observed_numbers
                .iter()
                .map(|value| {
                    if value.fract() == 0.0 {
                        format!("{}", *value as i64)
                    } else {
                        value.to_string()
                    }
                })
                .collect::<Vec<_>>();
            preview.sort();
            preview.dedup();

            failure.classification = Some("type_signature_wider_than_usage".into());
            failure.suggestion = Some(format!(
                "Observed static call sites only pass literal values like {} for parameter {}. Consider narrowing the declared type before adding a runtime guard.",
                preview.join(", "),
                index + 1
            ));
            break;
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FuzzOutcomeStatus {
    Passed,
    Crashed,
    NoInputsReached,
}

#[derive(Debug, Clone, Serialize)]
struct FuzzOutcome {
    function: String,
    status: FuzzOutcomeStatus,
    pass_count: usize,
    reject_count: usize,
    crash_count: usize,
    total_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct FuzzFindingCounts {
    crash: usize,
    property_violation: usize,
}

impl FuzzFindingCounts {
    fn total(self) -> usize {
        self.crash + self.property_violation
    }
}

fn parse_leading_usize(raw: &str, suffix: &str) -> Option<usize> {
    raw.trim()
        .strip_suffix(suffix)?
        .trim()
        .parse::<usize>()
        .ok()
}

fn parse_fuzz_outcomes(stdout: &str) -> Vec<FuzzOutcome> {
    let mut outcomes = Vec::new();

    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("FUZZ ") else {
            continue;
        };
        let Some((function, result)) = rest.split_once(": ") else {
            continue;
        };

        if let Some(total) = result
            .strip_prefix("all ")
            .and_then(|value| value.strip_suffix(" inputs rejected (nothing tested)"))
            .and_then(|value| value.parse::<usize>().ok())
        {
            outcomes.push(FuzzOutcome {
                function: function.to_string(),
                status: FuzzOutcomeStatus::NoInputsReached,
                pass_count: 0,
                reject_count: total,
                crash_count: 0,
                total_count: total,
            });
            continue;
        }

        let core = result.split(" [exercises: ").next().unwrap_or(result);
        let total_count = core
            .rsplit_once("(of ")
            .and_then(|(_, tail)| tail.strip_suffix(')'))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let parts: Vec<&str> = core.split(", ").map(|part| part.trim()).collect();
        if parts.len() < 2 {
            continue;
        }

        let pass_count = parse_leading_usize(parts[0], " passed").unwrap_or(0);
        let reject_count = parts
            .get(1)
            .and_then(|part| part.split(" (of ").next())
            .and_then(|part| parse_leading_usize(part, " rejected"))
            .unwrap_or(0);
        let crash_count = parts
            .get(2)
            .and_then(|part| part.split(" (of ").next())
            .and_then(|part| parse_leading_usize(part, " CRASHED"))
            .unwrap_or(0);
        let status = if crash_count > 0 {
            FuzzOutcomeStatus::Crashed
        } else {
            FuzzOutcomeStatus::Passed
        };

        outcomes.push(FuzzOutcome {
            function: function.to_string(),
            status,
            pass_count,
            reject_count,
            crash_count,
            total_count,
        });
    }

    outcomes
}

fn count_fuzz_findings(failures: &[FuzzFailure]) -> FuzzFindingCounts {
    let mut counts = FuzzFindingCounts {
        crash: 0,
        property_violation: 0,
    };

    for failure in failures {
        match failure.severity.as_str() {
            "crash" => counts.crash += 1,
            "property_violation" => counts.property_violation += 1,
            _ => {}
        }
    }

    counts
}

fn no_inputs_reached_count(outcomes: &[FuzzOutcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.status == FuzzOutcomeStatus::NoInputsReached)
        .count()
}

fn execute_gate_failed(gate: ExecuteGate, counts: FuzzFindingCounts) -> bool {
    match gate {
        ExecuteGate::All => counts.total() > 0,
        ExecuteGate::Crash => counts.crash > 0,
        ExecuteGate::None => false,
    }
}

fn execute_stage_ok(
    result: &ExecutionResult,
    gate: ExecuteGate,
    active_findings: &[FuzzFailure],
    suppressed_findings: &[FuzzFailure],
    outcomes: &[FuzzOutcome],
    module_load_blocked: bool,
) -> bool {
    if result.timed_out || result.memory_error || result.exit_code.is_none() || module_load_blocked
    {
        return false;
    }

    let counts = count_fuzz_findings(active_findings);
    if execute_gate_failed(gate, counts) {
        return false;
    }

    if counts.total() > 0 || no_inputs_reached_count(outcomes) > 0 {
        return true;
    }

    if !suppressed_findings.is_empty() {
        return true;
    }

    result.exit_code == Some(0)
}

/// Run the full verification pipeline: parse → complexity → lint → synthesize+execute → test.
pub async fn verify(
    code: &str,
    language: &Language,
    opts: VerifyOptions<'_>,
) -> VerificationReport {
    let mut stages = vec![];
    let mut overall_ok = true;
    let suppressions = parse_suppressions(opts.suppressions);

    // Stage 1: Parse / Analyze
    let start = Instant::now();
    let analysis = analyze::analyze(code, language);
    let parse_ms = start.elapsed().as_millis() as u64;

    if analysis.parse_error {
        stages.push(VerificationStage {
            name: "parse".into(),
            ok: false,
            duration_ms: parse_ms,
            detail: Some(serde_json::to_value(&analysis).unwrap()),
            error: Some("Code contains syntax errors".into()),
        });
        return finalize_report(
            build_report(stages, false),
            opts.output_dir,
            opts.source_file,
            language,
            opts.report_level,
        );
    }

    stages.push(VerificationStage {
        name: "parse".into(),
        ok: true,
        duration_ms: parse_ms,
        detail: Some(serde_json::to_value(&analysis).unwrap()),
        error: None,
    });

    // Stage 2: Complexity threshold (optional)
    if let Some(threshold) = opts.complexity_threshold {
        let start = Instant::now();
        let (functions_checked, diff_scoped) = if let Some(diff_str) = opts.diff {
            let changed_ranges = opts
                .source_file
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            (
                analyze::filter_changed_functions(&analysis, &changed_ranges),
                true,
            )
        } else {
            (analysis.functions.clone(), false)
        };
        let (violations, suppressed_violations, source_directive_functions) =
            split_complexity_violations(
                analyze::check_complexity_threshold_for_functions_with_metric(
                    &functions_checked,
                    threshold,
                    opts.complexity_metric,
                ),
                &suppressions,
                opts.source_file,
                code,
                language,
            );
        let complexity_ms = start.elapsed().as_millis() as u64;
        let complexity_ok = violations.is_empty();
        if !complexity_ok {
            overall_ok = false;
        }
        stages.push(VerificationStage {
            name: "complexity".into(),
            ok: complexity_ok,
            duration_ms: complexity_ms,
            detail: Some(serde_json::json!({
                "violations": serde_json::to_value(&violations).unwrap(),
                "suppressed_violations": serde_json::to_value(&suppressed_violations).unwrap(),
                "threshold": threshold,
                "metric": serde_json::to_value(opts.complexity_metric).unwrap(),
                "checked_functions": functions_checked.len(),
                "diff_scoped": diff_scoped,
                "complexity_ok": complexity_ok,
                "suppression_source": opts.suppression_source,
                "source_directive_functions": serde_json::to_value(&source_directive_functions).unwrap(),
                "source_directive_suppression_count": source_directive_functions.len(),
            })),
            error: if complexity_ok {
                None
            } else {
                Some(format!(
                    "{} function(s) exceed complexity threshold {}",
                    violations.len(),
                    threshold,
                ))
            },
        });
    }

    // Stage 3: Lint — informational unless the lint runner itself errors.
    let start = Instant::now();
    let mut lint_result = lint::lint_with_options(
        code,
        language,
        lint::LintOptions {
            source_file: opts.source_file,
            project_dir: opts.project_dir,
            config_path: opts.lint_config_path,
            virtual_file_path: opts.lint_virtual_file_path,
        },
    )
    .await;
    let lint_ms = start.elapsed().as_millis() as u64;

    // Filter out false positives only when linting anonymous inline snippets.
    if opts.source_file.is_none() && opts.lint_virtual_file_path.is_none() {
        lint_result.diagnostics.retain(|d| {
            !matches!(
                d.rule.as_str(),
                "lint/correctness/noUnusedVariables" | "F401" | "F841"
            )
        });
    }

    let lint_runner_failed = lint_result.runner_failed;
    let lint_ok = !lint_runner_failed;

    stages.push(VerificationStage {
        name: "lint".into(),
        ok: lint_ok,
        duration_ms: lint_ms,
        detail: Some(serde_json::to_value(&lint_result).unwrap()),
        error: if lint_runner_failed {
            lint_result.error.clone()
        } else {
            None
        },
    });

    if opts.tests_only && opts.test_code.is_none() {
        overall_ok = false;
        stages.push(VerificationStage {
            name: "test".into(),
            ok: false,
            duration_ms: 0,
            detail: None,
            error: Some("tests_only mode requires an authoritative test".into()),
        });
        return finalize_report(
            build_report(stages, overall_ok),
            opts.output_dir,
            opts.source_file,
            language,
            opts.report_level,
        );
    }

    // Stage 4: Synthesize + Execute
    if !opts.tests_only && !analysis.functions.is_empty() {
        // Determine which functions to fuzz
        let mut functions_to_fuzz: Vec<FunctionInfo> = if let Some(diff_str) = opts.diff {
            let changed_ranges = opts
                .source_file
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            analyze::filter_changed_functions(&analysis, &changed_ranges)
        } else {
            analysis.functions.clone()
        };
        let mut inferred_fixture_properties: HashMap<String, Vec<String>> = HashMap::new();
        let mut inferred_context_properties: HashMap<String, Vec<String>> = HashMap::new();
        if opts.auto_seed {
            if let Some(source_file) = opts.source_file {
                let function_names: HashSet<String> = functions_to_fuzz
                    .iter()
                    .map(|func| func.name.clone())
                    .collect();
                inferred_fixture_properties =
                    infer_fixture_properties(source_file, opts.project_dir, &function_names);
                apply_inferred_properties(&mut functions_to_fuzz, &inferred_fixture_properties);
                inferred_context_properties =
                    infer_context_properties(source_file, opts.project_dir, &functions_to_fuzz);
                apply_inferred_properties(&mut functions_to_fuzz, &inferred_context_properties);
            }
        }

        // Resolve imported types so the fuzzer can construct proper objects
        let mut all_classes = analysis.classes.clone();
        let mut all_aliases = analysis.aliases.clone();
        if let Some(src) = opts.source_file {
            let referenced_names = analyze::referenced_type_names_for_functions(&functions_to_fuzz);
            let imported = analyze::resolve_imported_types_for_names(
                &analysis,
                src,
                language,
                &referenced_names,
            );
            all_classes.extend(imported.classes);
            all_aliases.extend(imported.aliases);
        }
        let observed_calls = collect_seed_observations(
            code,
            language,
            &functions_to_fuzz,
            opts.source_file,
            opts.project_dir,
            opts.test_code,
            opts.test_source_file,
            opts.auto_seed,
        );
        let seed_inputs = seed_inputs_from_observations(&functions_to_fuzz, &observed_calls);
        let seed_sources = seed_sources(&observed_calls);
        let seed_input_count = total_seed_rows(&seed_inputs);

        let synth_start = Instant::now();
        let fuzz_plan = synthesize::synthesize_plan_for_with_seeds(
            &functions_to_fuzz,
            &all_classes,
            &all_aliases,
            language,
            &seed_inputs,
        );
        let coverage_ms = synth_start.elapsed().as_millis() as u64;
        let mut module_load_blocked = false;

        if !fuzz_plan.code.is_empty() {
            let full_code = format!("{code}\n{}", fuzz_plan.code);
            let execute_timeout = execute_timeout_for(language);

            let start = Instant::now();
            let mut exec_runtime: Option<String> = None;
            let mut portability_detail: Option<serde_json::Value> = None;
            let exec_result = if matches!(language, Language::TypeScript) {
                if let Some(repo_runtime) =
                    sandbox::detect_repo_typescript_runner(opts.project_dir, opts.source_file)
                {
                    let node_result = sandbox::execute_typescript_node(
                        &full_code,
                        execute_timeout,
                        512,
                        opts.project_dir,
                        opts.source_file,
                    )
                    .await;
                    let node_ok = node_result.exit_code == Some(0)
                        && !node_result.timed_out
                        && !node_result.memory_error;
                    if !node_ok && is_typescript_portability_error(&node_result.stderr) {
                        if let Some(repo_result) = sandbox::execute_typescript_repo_native(
                            &full_code,
                            execute_timeout,
                            512,
                            opts.project_dir,
                            opts.source_file,
                        )
                        .await
                        {
                            exec_runtime = Some(repo_runtime.clone());
                            let (detail, suppressed) = build_portability_detail(
                                &repo_runtime,
                                &node_result,
                                &repo_result,
                                &suppressions,
                                opts.source_file,
                                opts.suppression_source,
                            );
                            portability_detail = Some(detail);
                            if suppressed {
                                exec_runtime = Some(repo_runtime.clone());
                            }
                            repo_result
                        } else {
                            exec_runtime = Some("node".into());
                            node_result
                        }
                    } else {
                        exec_runtime = Some("node".into());
                        node_result
                    }
                } else {
                    sandbox::execute(
                        &full_code,
                        language,
                        execute_timeout,
                        512,
                        opts.project_dir,
                        opts.source_file,
                    )
                    .await
                }
            } else {
                sandbox::execute(
                    &full_code,
                    language,
                    execute_timeout,
                    512,
                    opts.project_dir,
                    opts.source_file,
                )
                .await
            };
            let exec_ms = start.elapsed().as_millis() as u64;
            module_load_blocked = matches!(language, Language::TypeScript)
                && is_typescript_module_load_error(&exec_result.stderr)
                && !exec_result
                    .stdout
                    .lines()
                    .any(|line| line.starts_with("FUZZ "));

            let coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
            stages.push(VerificationStage {
                name: "coverage".into(),
                ok: true,
                duration_ms: coverage_ms,
                detail: Some(serde_json::json!({
                    "functions": serde_json::to_value(&coverage).unwrap(),
                    "counts": coverage_counts(&coverage),
                    "diff_scoped": opts.diff.is_some(),
                    "seed_input_count": seed_input_count,
                    "seeded_functions": seed_inputs.len(),
                    "seed_sources": seed_sources,
                    "inferred_fixture_properties": inferred_fixture_properties,
                    "inferred_context_properties": inferred_context_properties,
                    "auto_seed": opts.auto_seed,
                })),
                error: None,
            });

            if let Some(detail) = portability_detail {
                let suppressed = detail
                    .get("suppressed")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                stages.push(VerificationStage {
                    name: "portability".into(),
                    ok: suppressed,
                    duration_ms: 0,
                    detail: Some(detail),
                    error: if suppressed {
                        None
                    } else {
                        Some("Node compatibility failed; repo-native runtime was used for fuzz execution".into())
                    },
                });
            }

            let mut detail = serde_json::to_value(&exec_result).unwrap();
            if let Some(runtime) = exec_runtime {
                detail["runtime"] = serde_json::Value::String(runtime);
            }
            let mut failures = parse_fuzz_failures(&exec_result.stdout).unwrap_or_default();
            classify_type_signature_failures(&mut failures, &observed_calls, language);
            let (failures, suppressed_failures) =
                split_fuzz_failures(failures, &suppressions, opts.source_file);
            let fuzz_outcomes = parse_fuzz_outcomes(&exec_result.stdout);
            let finding_counts = count_fuzz_findings(&failures);
            let no_inputs_reached = no_inputs_reached_count(&fuzz_outcomes);
            let exec_ok = execute_stage_ok(
                &exec_result,
                opts.execute_gate,
                &failures,
                &suppressed_failures,
                &fuzz_outcomes,
                module_load_blocked,
            );
            if !exec_ok {
                overall_ok = false;
            }
            if !failures.is_empty() {
                detail["fuzz_failures"] = serde_json::to_value(&failures).unwrap();
            }
            if !suppressed_failures.is_empty() {
                detail["suppressed_fuzz_failures"] =
                    serde_json::to_value(&suppressed_failures).unwrap();
            }
            if !fuzz_outcomes.is_empty() {
                detail["fuzz_outcomes"] = serde_json::to_value(&fuzz_outcomes).unwrap();
            }
            detail["finding_counts"] = serde_json::json!({
                "crash": finding_counts.crash,
                "property_violation": finding_counts.property_violation,
            });
            let suppressed_counts = count_fuzz_findings(&suppressed_failures);
            detail["suppressed_finding_counts"] = serde_json::json!({
                "crash": suppressed_counts.crash,
                "property_violation": suppressed_counts.property_violation,
            });
            detail["no_inputs_reached"] = serde_json::Value::from(no_inputs_reached);
            detail["execute_gate"] = serde_json::to_value(opts.execute_gate).unwrap();
            detail["execute_gate_failed"] =
                serde_json::Value::Bool(execute_gate_failed(opts.execute_gate, finding_counts));
            detail["module_load_blocked"] = serde_json::Value::Bool(module_load_blocked);
            detail["seed_input_count"] = serde_json::Value::from(seed_input_count);
            detail["seeded_functions"] = serde_json::Value::from(seed_inputs.len());
            detail["seed_sources"] = serde_json::to_value(&seed_sources).unwrap();
            detail["inferred_fixture_properties"] =
                serde_json::to_value(&inferred_fixture_properties).unwrap();
            detail["inferred_context_properties"] =
                serde_json::to_value(&inferred_context_properties).unwrap();
            detail["auto_seed"] = serde_json::Value::Bool(opts.auto_seed);
            detail["suppression_source"] = serde_json::to_value(opts.suppression_source).unwrap();

            stages.push(VerificationStage {
                name: "execute".into(),
                ok: exec_ok,
                duration_ms: exec_ms,
                detail: Some(detail),
                error: if exec_ok {
                    None
                } else {
                    Some(exec_result.stderr.clone())
                },
            });
        } else {
            let coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
            stages.push(VerificationStage {
                name: "coverage".into(),
                ok: true,
                duration_ms: coverage_ms,
                detail: Some(serde_json::json!({
                    "functions": serde_json::to_value(&coverage).unwrap(),
                    "counts": coverage_counts(&coverage),
                    "diff_scoped": opts.diff.is_some(),
                    "seed_input_count": seed_input_count,
                    "seeded_functions": seed_inputs.len(),
                    "seed_sources": seed_sources,
                    "inferred_fixture_properties": inferred_fixture_properties,
                    "inferred_context_properties": inferred_context_properties,
                    "auto_seed": opts.auto_seed,
                })),
                error: None,
            });
        }
    }

    // Stage 5: Test (if test_code provided) — this IS authoritative
    if let Some(tests) = opts.test_code {
        let has_import_statements = test_code_has_imports(tests, language);
        let mut test_file_for_execution = opts.test_source_file.or(opts.source_file);
        if !has_import_statements {
            if let Some(source_file) = opts.source_file {
                test_file_for_execution = Some(source_file);
            }
        }

        // Test files in this benchmark can include direct symbol assertions against the
        // module under test (e.g., `displayInitials(...)`). In those cases, include the
        // candidate source so the assertions execute in a valid lexical scope.
        let test_input = if !has_import_statements {
            format!("{code}\n\n{tests}")
        } else {
            tests.to_string()
        };

        let start = Instant::now();
        let selected_test_runner = match language {
            Language::TypeScript => match opts.test_runner {
                TestRunner::Auto if sandbox::typescript_code_requires_bun_runtime(&test_input) => {
                    TestRunner::Bun
                }
                other => other,
            },
            Language::Python => TestRunner::Auto,
        };
        let test_result = match language {
            Language::Python => {
                sandbox::execute(
                    &test_input,
                    language,
                    test_timeout(),
                    512,
                    opts.project_dir,
                    test_file_for_execution,
                )
                .await
            }
            Language::TypeScript => match selected_test_runner {
                TestRunner::Auto => {
                    sandbox::execute(
                        &test_input,
                        language,
                        test_timeout(),
                        512,
                        opts.project_dir,
                        test_file_for_execution,
                    )
                    .await
                }
                TestRunner::Node => {
                    sandbox::execute_typescript_node(
                        &test_input,
                        test_timeout(),
                        512,
                        opts.project_dir,
                        test_file_for_execution,
                    )
                    .await
                }
                TestRunner::Bun => {
                    sandbox::execute_typescript_bun_test(
                        &test_input,
                        test_timeout(),
                        512,
                        opts.project_dir,
                        test_file_for_execution,
                    )
                    .await
                }
                TestRunner::RepoNative => sandbox::execute_typescript_repo_native_test(
                    &test_input,
                    test_timeout(),
                    512,
                    opts.project_dir,
                    test_file_for_execution,
                )
                .await
                .unwrap_or_else(|| {
                    err_execution_result(
                        "repo-native TypeScript test runner requested, but no repo runtime was detected",
                    )
                }),
            },
        };
        let test_ms = start.elapsed().as_millis() as u64;

        let has_assertion_failure = test_result.stderr.contains("Assertion failed")
            || test_result.stderr.contains("AssertionError");
        let test_ok =
            test_result.exit_code == Some(0) && !test_result.timed_out && !has_assertion_failure;
        if !test_ok {
            overall_ok = false;
        }

        let mut test_detail = serde_json::to_value(&test_result).unwrap();
        test_detail["test_runner_requested"] = serde_json::to_value(opts.test_runner).unwrap();
        test_detail["test_runner_selected"] = serde_json::to_value(selected_test_runner).unwrap();

        stages.push(VerificationStage {
            name: "test".into(),
            ok: test_ok,
            duration_ms: test_ms,
            detail: Some(test_detail),
            error: if test_ok {
                None
            } else {
                Some(test_result.stderr.clone())
            },
        });
    }

    finalize_report(
        build_report(stages, overall_ok),
        opts.output_dir,
        opts.source_file,
        language,
        opts.report_level,
    )
}

fn build_report(stages: Vec<VerificationStage>, overall_ok: bool) -> VerificationReport {
    let summary = compute_report_summary(&stages);
    VerificationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        stages,
        overall_ok,
        summary,
        report_path: None,
    }
}

fn finalize_report(
    mut report: VerificationReport,
    output_dir: Option<&str>,
    source_file: Option<&str>,
    language: &Language,
    report_level: ReportLevel,
) -> VerificationReport {
    if let Some(dir) = output_dir {
        report.report_path = write_report(dir, &report, source_file, language, report_level);
    }
    report
}

fn minimal_stage_view(stage: &VerificationStage) -> serde_json::Value {
    let mut value = serde_json::json!({
        "name": stage.name,
        "ok": stage.ok,
        "duration_ms": stage.duration_ms,
    });
    if let Some(error) = &stage.error {
        value["error"] = serde_json::Value::String(error.clone());
    }
    if let Some(detail) = &stage.detail {
        let trimmed = match stage.name.as_str() {
            "complexity" => Some(serde_json::json!({
                "threshold": detail.get("threshold").cloned().unwrap_or(serde_json::Value::Null),
                "metric": detail.get("metric").cloned().unwrap_or(serde_json::Value::Null),
                "checked_functions": detail.get("checked_functions").cloned().unwrap_or(serde_json::Value::Null),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "violations": detail.get("violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "suppressed_violations": detail.get("suppressed_violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_functions": detail.get("source_directive_functions").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_suppression_count": detail.get("source_directive_suppression_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
            })),
            "coverage" => Some(serde_json::json!({
                "counts": detail.get("counts").cloned().unwrap_or(serde_json::json!({})),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "seed_input_count": detail.get("seed_input_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seeded_functions": detail.get("seeded_functions").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seed_sources": detail.get("seed_sources").cloned().unwrap_or_else(|| serde_json::json!([])),
                "inferred_context_properties": detail.get("inferred_context_properties").cloned().unwrap_or_else(|| serde_json::json!({})),
            })),
            "execute" => Some(serde_json::json!({
                "runtime": detail.get("runtime").cloned().unwrap_or(serde_json::Value::Null),
                "finding_counts": detail.get("finding_counts").cloned().unwrap_or_else(|| serde_json::json!({})),
                "no_inputs_reached": detail.get("no_inputs_reached").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "execute_gate": detail.get("execute_gate").cloned().unwrap_or(serde_json::Value::Null),
                "execute_gate_failed": detail.get("execute_gate_failed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "fuzz_failures": detail.get("fuzz_failures").cloned().unwrap_or_else(|| serde_json::json!([])),
                "suppressed_finding_counts": detail.get("suppressed_finding_counts").cloned().unwrap_or_else(|| serde_json::json!({})),
                "suppressed_fuzz_failures": detail.get("suppressed_fuzz_failures").cloned().unwrap_or_else(|| serde_json::json!([])),
                "seed_input_count": detail.get("seed_input_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seeded_functions": detail.get("seeded_functions").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seed_sources": detail.get("seed_sources").cloned().unwrap_or_else(|| serde_json::json!([])),
                "inferred_context_properties": detail.get("inferred_context_properties").cloned().unwrap_or_else(|| serde_json::json!({})),
            })),
            "lint" => Some(serde_json::json!({
                "diagnostics": detail.get("diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_diagnostics": detail.get("runner_diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_failed": detail.get("runner_failed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "unavailable": detail.get("unavailable").cloned().unwrap_or(serde_json::Value::Bool(false)),
            })),
            "portability" => Some(serde_json::json!({
                "reason": detail.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                "failing_imports": detail.get("failing_imports").cloned().unwrap_or_else(|| serde_json::json!([])),
                "fix_hint": detail.get("fix_hint").cloned().unwrap_or(serde_json::Value::Null),
                "suppressed": detail.get("suppressed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "repo_runtime": detail.get("repo_runtime").cloned().unwrap_or(serde_json::Value::Null),
                "node_result": serde_json::json!({
                    "stderr": detail
                        .get("node_result")
                        .and_then(|node| node.get("stderr"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }),
            })),
            _ => None,
        };
        if let Some(trimmed) = trimmed {
            value["detail"] = trimmed;
        }
    }
    value
}

pub fn report_json_value(
    report: &VerificationReport,
    report_level: ReportLevel,
) -> serde_json::Value {
    match report_level {
        ReportLevel::Full => serde_json::to_value(report).unwrap(),
        ReportLevel::Minimal => serde_json::json!({
            "schema_version": report.schema_version,
            "overall_ok": report.overall_ok,
            "summary": report.summary,
            "report_path": report.report_path,
            "stages": report
                .stages
                .iter()
                .map(minimal_stage_view)
                .collect::<Vec<_>>(),
        }),
    }
}

fn human_status(ok: bool) -> &'static str {
    if ok {
        "OK"
    } else {
        "FAIL"
    }
}

fn clip_human(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(limit).collect();
    format!("{clipped}...")
}

fn human_number(detail: &serde_json::Value, key: &str) -> usize {
    detail
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize
}

pub fn report_human_summary(report: &VerificationReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Overall: {}",
        if report.overall_ok { "PASS" } else { "FAIL" }
    );
    if let Some(path) = &report.report_path {
        let _ = writeln!(out, "Report Path: {path}");
    }

    let summary = &report.summary;
    let _ = writeln!(
        out,
        "Coverage: {} analyzed, {} fuzzed, {} skipped, {} module-load blocked",
        summary.functions_analyzed,
        summary.functions_fuzzed,
        summary.functions_skipped,
        summary.functions_blocked_module_load
    );
    let _ = writeln!(
        out,
        "Execute: {} passed, {} crashed, {} property violations, {} no-inputs-reached",
        summary.fuzz_pass,
        summary.fuzz_crash,
        summary.fuzz_property_violation,
        summary.fuzz_no_inputs_reached
    );
    let _ = writeln!(
        out,
        "Lint: {} issues, {} runner failures",
        summary.lint_issues, summary.lint_runner_failures
    );
    let _ = writeln!(
        out,
        "Complexity: {} violations, {} suppressed",
        summary.complexity_violations, summary.suppressed_complexity_violations
    );

    let _ = writeln!(out);
    let _ = writeln!(out, "Stages:");
    for stage in &report.stages {
        let mut extra = String::new();
        if let Some(detail) = &stage.detail {
            match stage.name.as_str() {
                "execute" => {
                    let crash = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("crash"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let property = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("property_violation"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let no_inputs = human_number(detail, "no_inputs_reached");
                    extra = format!("crash={crash}, property={property}, no_inputs={no_inputs}");
                }
                "coverage" => {
                    let counts = detail.get("counts").cloned().unwrap_or_default();
                    let fuzzed = counts
                        .get("fuzzed")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let skipped: u64 = counts
                        .as_object()
                        .map(|obj| {
                            obj.iter()
                                .filter(|(key, _)| {
                                    let key = key.as_str();
                                    key != "fuzzed" && key != "fuzzed_via_factory"
                                })
                                .map(|(_, value)| value.as_u64().unwrap_or(0))
                                .sum()
                        })
                        .unwrap_or(0);
                    let factory = counts
                        .get("fuzzed_via_factory")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    extra = format!("fuzzed={fuzzed}, factory={factory}, skipped={skipped}");
                }
                "lint" => {
                    let issues = detail
                        .get("diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let runner_failures = detail
                        .get("runner_diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let unavailable = detail
                        .get("unavailable")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    extra = format!(
                        "issues={issues}, runner_failures={runner_failures}, unavailable={unavailable}"
                    );
                }
                "complexity" => {
                    let violations = detail
                        .get("violations")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let threshold = human_number(detail, "threshold");
                    extra = format!("violations={violations}, threshold={threshold}");
                }
                _ => {}
            }
        }
        let _ = if extra.is_empty() {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms",
                stage.name,
                human_status(stage.ok),
                stage.duration_ms
            )
        } else {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms  {}",
                stage.name,
                human_status(stage.ok),
                stage.duration_ms,
                extra
            )
        };
        if let Some(error) = &stage.error {
            let _ = writeln!(out, "    {}", clip_human(error, 160));
        }
    }

    if let Some(complexity_stage) = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
    {
        if let Some(violations) = complexity_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("violations"))
            .and_then(|value| value.as_array())
        {
            if !violations.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Complexity Offenders:");
                for (idx, violation) in violations.iter().take(5).enumerate() {
                    let function = violation
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let line = violation
                        .get("line")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cyclomatic = violation
                        .get("complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cognitive = violation
                        .get("cognitive_complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let _ = writeln!(
                        out,
                        "  {}. {} (line {}) cyclomatic={} cognitive={}",
                        idx + 1,
                        function,
                        line,
                        cyclomatic,
                        cognitive
                    );
                }
            }
        }
    }

    if let Some(execute_stage) = report.stages.iter().find(|stage| stage.name == "execute") {
        if let Some(failures) = execute_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("fuzz_failures"))
            .and_then(|value| value.as_array())
        {
            if !failures.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Execute Findings:");
                for (idx, failure) in failures.iter().take(5).enumerate() {
                    let function = failure
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let severity = failure
                        .get("severity")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown");
                    let message = failure
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let _ = writeln!(
                        out,
                        "  {}. {} [{}] {}",
                        idx + 1,
                        function,
                        severity,
                        clip_human(message, 140)
                    );
                }
            }
        }
    }

    out.trim_end().to_string()
}

fn compute_report_summary(stages: &[VerificationStage]) -> ReportSummary {
    let mut summary = ReportSummary {
        functions_analyzed: 0,
        functions_fuzzed: 0,
        functions_skipped: 0,
        functions_blocked_module_load: 0,
        fuzz_pass: 0,
        fuzz_crash: 0,
        fuzz_property_violation: 0,
        fuzz_no_inputs_reached: 0,
        suppressed_fuzz_findings: 0,
        suppressed_complexity_violations: 0,
        suppressed_portability_warnings: 0,
        lint_issues: 0,
        lint_runner_failures: 0,
        complexity_violations: 0,
    };

    for stage in stages {
        if let Some(detail) = &stage.detail {
            match stage.name.as_str() {
                "parse" => {
                    if let Some(arr) = detail.get("functions").and_then(|funcs| funcs.as_array()) {
                        summary.functions_analyzed = arr.len();
                    }
                }
                "coverage" => {
                    if let Some(funcs) = detail.get("functions").and_then(|value| value.as_array())
                    {
                        for func in funcs {
                            match func.get("status").and_then(|value| value.as_str()) {
                                Some("fuzzed" | "fuzzed_via_factory") => {
                                    summary.functions_fuzzed += 1
                                }
                                Some("blocked_module_load") => {
                                    summary.functions_blocked_module_load += 1;
                                }
                                Some(_) => summary.functions_skipped += 1,
                                None => {}
                            }
                        }
                    }
                }
                "execute" => {
                    if let Some(outcomes) = detail
                        .get("fuzz_outcomes")
                        .and_then(|value| value.as_array())
                    {
                        for outcome in outcomes {
                            match outcome.get("status").and_then(|value| value.as_str()) {
                                Some("passed") => summary.fuzz_pass += 1,
                                Some("crashed") => summary.fuzz_crash += 1,
                                Some("no_inputs_reached") => summary.fuzz_no_inputs_reached += 1,
                                _ => {}
                            }
                        }
                    }
                    if let Some(counts) = detail.get("finding_counts") {
                        summary.fuzz_property_violation = counts
                            .get("property_violation")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                            as usize;
                    }
                    if let Some(counts) = detail.get("suppressed_finding_counts") {
                        let suppressed = counts
                            .get("crash")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                            + counts
                                .get("property_violation")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(0);
                        summary.suppressed_fuzz_findings = suppressed as usize;
                    }
                }
                "lint" => {
                    if let Some(arr) = detail.get("diagnostics").and_then(|diags| diags.as_array())
                    {
                        summary.lint_issues = arr.len();
                    }
                    if detail
                        .get("runner_failed")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        summary.lint_runner_failures += 1;
                    }
                }
                "complexity" => {
                    if let Some(arr) = detail.get("violations").and_then(|value| value.as_array()) {
                        summary.complexity_violations = arr.len();
                    }
                    if let Some(arr) = detail
                        .get("suppressed_violations")
                        .and_then(|value| value.as_array())
                    {
                        summary.suppressed_complexity_violations = arr.len();
                    }
                }
                "portability" => {
                    if detail
                        .get("suppressed")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        summary.suppressed_portability_warnings += 1;
                    }
                }
                _ => {}
            }
        }
    }

    summary
}

fn write_report(
    output_dir: &str,
    report: &VerificationReport,
    source_file: Option<&str>,
    language: &Language,
    report_level: ReportLevel,
) -> Option<String> {
    use chrono::Utc;

    let _ = std::fs::create_dir_all(output_dir);
    let total_duration = report
        .stages
        .iter()
        .map(|stage| stage.duration_ms)
        .sum::<u64>();

    let now = Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let file_timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();

    let persisted = PersistedReport {
        schema_version: REPORT_SCHEMA_VERSION,
        meta: ReportMeta {
            source_file: source_file.map(|s| s.to_string()),
            language: format!("{:?}", language).to_lowercase(),
            timestamp,
            duration_ms: total_duration,
        },
        stages: report.stages.clone(),
        overall_ok: report.overall_ok,
        summary: report.summary.clone(),
    };
    let basename = source_file
        .map(|s| {
            std::path::Path::new(s)
                .file_stem()
                .and_then(|os| os.to_str())
                .unwrap_or("inline")
                .to_string()
        })
        .unwrap_or_else(|| "inline".to_string());

    let filename = format!("{file_timestamp}-{basename}.json");
    let path = std::path::Path::new(output_dir).join(&filename);

    let json_value = match report_level {
        ReportLevel::Full => serde_json::to_value(&persisted).ok()?,
        ReportLevel::Minimal => serde_json::json!({
            "schema_version": persisted.schema_version,
            "meta": persisted.meta,
            "overall_ok": persisted.overall_ok,
            "summary": persisted.summary,
            "stages": persisted
                .stages
                .iter()
                .map(minimal_stage_view)
                .collect::<Vec<_>>(),
        }),
    };

    match serde_json::to_string_pretty(&json_value) {
        Ok(json) => {
            if std::fs::write(&path, &json).is_ok() {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Parse structured fuzz failure JSON from stdout using the sentinel marker.
pub fn parse_fuzz_failures(stdout: &str) -> Option<Vec<FuzzFailure>> {
    let marker = "__COURT_JESTER_FUZZ_JSON__";
    let idx = stdout.rfind(marker)?;
    let json_str = stdout[idx + marker.len()..].trim();
    serde_json::from_str(json_str).ok()
}
