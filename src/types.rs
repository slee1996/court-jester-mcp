use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    TypeScript,
}

impl Language {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "python" | "py" => Some(Language::Python),
            "typescript" | "ts" => Some(Language::TypeScript),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// True for Python keyword-only parameters (after `*` separator).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keyword_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<ParamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    pub line: usize,
    pub end_line: usize,
    pub complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cognitive_complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_nesting_depth: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub complexity_breakdown: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_method: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_nested: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_exported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    pub name: String,
    pub bases: Vec<String>,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeAliasInfo {
    pub name: String,
    pub type_annotation: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedTypeInfo {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<ClassInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<TypeAliasInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub statement: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<TypeAliasInfo>,
    pub imports: Vec<ImportInfo>,
    pub complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cognitive_complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_nesting_depth: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub complexity_breakdown: BTreeMap<String, usize>,
    pub parse_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub memory_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintDiagnostic {
    pub rule: String,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    pub diagnostics: Vec<LintDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzFailure {
    pub function: String,
    pub input: String,
    pub error_type: String,
    pub message: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuzzFunctionStatus {
    Fuzzed,
    SkippedUnsupportedType,
    SkippedInternalHelper,
    SkippedMethod,
    SkippedNested,
    SkippedPrivateName,
    SkippedDiffFiltered,
    BlockedModuleLoad,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzFunctionCoverage {
    pub function: String,
    pub line: usize,
    pub end_line: usize,
    pub status: FuzzFunctionStatus,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzPlan {
    pub code: String,
    pub coverage: Vec<FuzzFunctionCoverage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityViolation {
    pub function: String,
    pub complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cognitive_complexity: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_nesting_depth: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub complexity_breakdown: BTreeMap<String, usize>,
    pub threshold: usize,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationStage {
    pub name: String,
    pub ok: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub stages: Vec<VerificationStage>,
    pub overall_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    pub language: String,
    pub timestamp: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    pub functions_analyzed: usize,
    pub functions_fuzzed: usize,
    pub functions_skipped: usize,
    pub functions_blocked_module_load: usize,
    pub fuzz_pass: usize,
    pub fuzz_crash: usize,
    pub lint_issues: usize,
    pub complexity_violations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedReport {
    pub meta: ReportMeta,
    pub stages: Vec<VerificationStage>,
    pub overall_ok: bool,
    pub summary: ReportSummary,
}
