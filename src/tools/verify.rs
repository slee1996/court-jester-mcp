use std::collections::{HashMap, HashSet};
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
        } else if func.is_method {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedMethod,
                Some("methods are not fuzzed directly".into()),
            )
        } else if func.is_nested {
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
) -> (Vec<ComplexityViolation>, Vec<ComplexityViolation>) {
    let mut active = Vec::new();
    let mut suppressed = Vec::new();

    for violation in violations {
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

    (active, suppressed)
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

fn parse_literal_arg(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<ObservedArg> {
    let code = node_text(node, source);
    let literal_value = match language {
        Language::TypeScript => match node.kind() {
            "number" => code
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number),
            "true" => Some(serde_json::Value::Bool(true)),
            "false" => Some(serde_json::Value::Bool(false)),
            "null" => Some(serde_json::Value::Null),
            "undefined" | "string" => None,
            _ => return None,
        },
        Language::Python => match node.kind() {
            "integer" => code.parse::<i64>().ok().map(serde_json::Value::from),
            "float" => code
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number),
            "true" => Some(serde_json::Value::Bool(true)),
            "false" => Some(serde_json::Value::Bool(false)),
            "none" | "string" => None,
            _ => return None,
        },
    };
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
            if callee.kind() == "identifier" {
                let name = node_text(&callee, source);
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

fn collect_seed_observations(
    code: &str,
    language: &Language,
    functions: &[FunctionInfo],
    source_file: Option<&str>,
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
        let (violations, suppressed_violations) = split_complexity_violations(
            analyze::check_complexity_threshold_for_functions_with_metric(
                &functions_checked,
                threshold,
                opts.complexity_metric,
            ),
            &suppressions,
            opts.source_file,
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

    let lint_runner_failed = lint_result.error.is_some() && !lint_result.unavailable;
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
        let functions_to_fuzz: Vec<FunctionInfo> = if let Some(diff_str) = opts.diff {
            let changed_ranges = opts
                .source_file
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            analyze::filter_changed_functions(&analysis, &changed_ranges)
        } else {
            analysis.functions.clone()
        };

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
            })),
            "coverage" => Some(serde_json::json!({
                "counts": detail.get("counts").cloned().unwrap_or(serde_json::json!({})),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "seed_input_count": detail.get("seed_input_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seeded_functions": detail.get("seeded_functions").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seed_sources": detail.get("seed_sources").cloned().unwrap_or_else(|| serde_json::json!([])),
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
                                Some("fuzzed") => summary.functions_fuzzed += 1,
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
