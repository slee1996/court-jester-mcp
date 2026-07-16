use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA_VERSION: u32 = 3;

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    TypeScript,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeProfile {
    #[default]
    LocalTrusted,
    Isolated,
}

impl RuntimeProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local-trusted" => Some(Self::LocalTrusted),
            "isolated" => Some(Self::Isolated),
            _ => None,
        }
    }
}

/// Fully validated execution request. Paths are borrowed from the caller so
/// dispatch cannot accidentally outlive the verification request.
#[derive(Debug, Clone, Copy)]
pub struct SandboxOptions<'a> {
    pub timeout_seconds: f64,
    pub memory_mb: u64,
    pub runtime_profile: RuntimeProfile,
    pub docker_image: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub source_file: Option<&'a str>,
}

impl SandboxOptions<'_> {
    pub fn validate(&self) -> Result<(), String> {
        if !self.timeout_seconds.is_finite() || self.timeout_seconds <= 0.0 {
            return Err("timeout must be finite and greater than zero".into());
        }
        if self.memory_mb == 0 {
            return Err("memory must be greater than zero".into());
        }
        if let Some(image) = self.docker_image {
            if image.trim().is_empty() || image.starts_with('-') {
                return Err("docker image must be non-empty and must not begin with '-'".into());
            }
            if self.runtime_profile != RuntimeProfile::Isolated {
                return Err("docker image overrides require the isolated runtime profile".into());
            }
        }
        if self.runtime_profile == RuntimeProfile::Isolated && self.docker_image.is_none() {
            return Err("isolated runtime profile requires a docker image".into());
        }
        Ok(())
    }
}
pub const DEFAULT_PYTHON_DOCKER_IMAGE: &str = "python:3.12-slim";
pub const DEFAULT_TYPESCRIPT_DOCKER_IMAGE: &str = "node:24-bookworm-slim";

impl Language {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "python" | "py" => Some(Language::Python),
            "typescript" | "ts" => Some(Language::TypeScript),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReportLevel {
    #[default]
    Full,
    Minimal,
}

impl ReportLevel {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "full" => Some(Self::Full),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SummaryFormat {
    #[default]
    Json,
    Human,
    RepairJson,
}

impl SummaryFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "json" => Some(Self::Json),
            "human" => Some(Self::Human),
            "repair-json" | "repair_json" => Some(Self::RepairJson),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteGate {
    #[default]
    All,
    Crash,
    None,
}

impl ExecuteGate {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "all" => Some(Self::All),
            "crash" => Some(Self::Crash),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestRunner {
    #[default]
    Auto,
    Node,
    Bun,
    RepoNative,
}

impl TestRunner {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "auto" => Some(Self::Auto),
            "node" => Some(Self::Node),
            "bun" => Some(Self::Bun),
            "repo-native" | "repo_native" => Some(Self::RepoNative),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InferredOracleGate {
    #[default]
    Advisory,
    Fail,
}

impl InferredOracleGate {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "advisory" => Some(Self::Advisory),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityMetric {
    #[default]
    Cyclomatic,
    Cognitive,
}

impl ComplexityMetric {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "cyclomatic" => Some(Self::Cyclomatic),
            "cognitive" => Some(Self::Cognitive),
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_properties: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returned_callables: Vec<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallResolution {
    Local,
    Imported { module: String, symbol: String },
    Alias { symbol: String },
    Dynamic,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallEdge {
    pub caller_surface_id: String,
    pub callee_surface_id: String,
    pub source_file: String,
    pub line: usize,
    pub resolution: CallResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorSnapshot {
    pub returned: Option<serde_json::Value>,
    pub exception_type: Option<String>,
    pub exception_message: Option<String>,
    pub stdout: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedLocalSource {
    pub relative_path: String,
    pub content: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentationMode {
    PythonSitecustomize,
    NodeModuleRegister,
    BunPreload,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentationOverlay {
    pub mode: InstrumentationMode,
    pub source_file: String,
    pub surfaces: Vec<String>,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runner_diagnostics: Vec<LintDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub runner_failed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Crash,
    PropertyViolation,
    BehavioralRegression,
    Infrastructure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingConfidence {
    Authoritative,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Exception,
    Property,
    Test,
    Differential,
    Infrastructure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputClassification {
    Valid,
    Invalid,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OracleKind {
    AuthoritativeTest,
    RuntimeContract,
    TypeContract,
    DeclaredProperty,
    SeedRegression,
    Differential,
    GenericProperty,
    InferredSemantic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OracleProvenance {
    TestFile,
    LanguageRuntime,
    TypeAnnotation,
    SourceDirective,
    ObservedCall,
    JsonFixture,
    ContextFile,
    NameHeuristic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingLocation {
    pub source_file: String,
    pub function: String,
    pub line: usize,
    pub invocation_path: InvocationPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OracleInfo {
    pub id: String,
    pub kind: OracleKind,
    pub provenance: OracleProvenance,
    pub confidence: FindingConfidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FindingsSummary {
    pub total: usize,
    pub gating: usize,
    pub advisory: usize,
    pub suppressed: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_severity: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_oracle_kind: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReproKind {
    FunctionCall,
    FactoryCall,
    CallerCall,
    SemanticCase,
    TestCommand,
    Differential,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReproValue {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReproCase {
    pub arguments: Vec<ReproValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayExpectation {
    pub severity: FindingSeverity,
    pub oracle_kind: OracleKind,
    pub category: FindingCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddedSource {
    pub relative_path: String,
    pub content: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyContract {
    pub language: Language,
    pub runtime_identity: String,
    pub lockfiles: Vec<EmbeddedSource>,
    pub third_party_modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DifferentialRepro {
    pub relative_entry: String,
    pub base_files: Vec<EmbeddedSource>,
    pub candidate_files: Vec<EmbeddedSource>,
    pub base_tree_sha256: String,
    pub candidate_tree_sha256: String,
    pub dependency_contract: DependencyContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredRepro {
    pub kind: ReproKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    pub arguments: Vec<ReproValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_label: Option<String>,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub expectation: ReplayExpectation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub differential: Option<DifferentialRepro>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MinimizationStatus {
    NotNeeded,
    Preserved,
    Failed,
    BudgetExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MinimizationInfo {
    pub status: MinimizationStatus,
    pub attempts: usize,
    pub original: ReproCase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimized: Option<ReproCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationFinding {
    pub id: String,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub category: FindingCategory,
    pub location: FindingLocation,
    pub oracle: OracleInfo,
    pub input_classification: InputClassification,
    pub repro: StructuredRepro,
    pub minimization: MinimizationInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStrength {
    None,
    ParseOnly,
    StaticChecked,
    RuntimeSmoke,
    PropertyChecked,
    AuthoritativeTests,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Passed,
    Failed,
    Inconclusive,
    Advisory,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverageGate {
    #[default]
    ChangedExports,
    None,
}

impl CoverageGate {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "changed-exports" | "changed_exports" => Some(Self::ChangedExports),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationPath {
    Direct,
    Factory {
        factory: String,
        callable: String,
    },
    Caller {
        source_file: String,
        symbol: String,
        line: usize,
    },
    AuthoritativeTest {
        source_file: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuzzFunctionStatus {
    CheckedDirect,
    ReachedViaFactory,
    CheckedViaFactory,
    CheckedViaCaller,
    CheckedViaAuthoritativeTest,
    SkippedNoFuzzableSurface,
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
    #[serde(default)]
    pub required: bool,
    pub invocation_path: InvocationPath,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_exported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoverageSummary {
    pub required: usize,
    pub behaviorally_checked: usize,
    pub reached_only: usize,
    pub no_inputs_reached: usize,
    pub skipped: usize,
    pub blocked: usize,
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
    pub status: StageStatus,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub parsed: bool,
    pub static_checks_completed: bool,
    pub valid_invocations: usize,
    pub evaluated_oracles: usize,
    pub authoritative_test_completed: bool,
    pub authoritative_test_covered_surfaces: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: u32,
    pub stages: Vec<VerificationStage>,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub summary: ReportSummary,
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
    pub fuzz_no_inputs_reached: usize,
    pub findings: FindingsSummary,
    pub suppressed_complexity_violations: usize,
    pub suppressed_portability_warnings: usize,
    pub lint_issues: usize,
    pub lint_runner_failures: usize,
    pub complexity_violations: usize,
    pub coverage: CoverageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedReport {
    pub schema_version: u32,
    pub meta: ReportMeta,
    pub stages: Vec<VerificationStage>,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub summary: ReportSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    pub status: StageStatus,
    pub detail: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub verdict: VerificationVerdict,
    pub runtime_profile: RuntimeProfile,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairSummary {
    pub schema_version: u32,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub recommended_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_finding: Option<VerificationFinding>,
    pub findings: Vec<VerificationFinding>,
    pub coverage: CoverageSummary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOutcome {
    Reproduced,
    NotReproduced,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayReport {
    pub schema_version: u32,
    pub finding_id: String,
    pub outcome: ReplayOutcome,
    pub execution: ExecutionResult,
}
// Repository-derived domain and verification-plan IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DomainNode {
    Any,
    Boolean,
    Integer,
    Float,
    String,
    Bytes,
    Literal(Vec<DomainLiteral>),
    Nullable(Box<DomainNode>),
    Union(Vec<DomainNode>),
    Array(Box<DomainNode>),
    Tuple(Vec<DomainNode>),
    Set(Box<DomainNode>),
    Map(Box<DomainNode>, Box<DomainNode>),
    Object(Vec<DomainField>),
    Opaque(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainLiteral {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainField {
    pub name: String,
    pub domain: DomainNode,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainSourceKind {
    TypeAnnotation,
    TypescriptEnum,
    TypescriptConstTuple,
    ImportedType,
    ObservedCall,
    JsonFixture,
    DefaultValue,
    ValidationGuard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainSource {
    pub kind: DomainSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceSpec {
    pub id: String,
    pub symbol: String,
    pub source_file: String,
    pub line: usize,
    pub exported: bool,
    pub invocable: bool,
    pub parameter_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterDomain {
    pub surface_id: String,
    pub parameter: String,
    pub index: usize,
    pub domain: DomainNode,
    pub closed: bool,
    pub sources: Vec<DomainSource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallerEvidence {
    StaticSyntax,
    RuntimeConfirmed,
    AuthoritativeFixture,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedArguments {
    pub positional: Vec<DomainLiteral>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub named: BTreeMap<String, DomainLiteral>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallerExample {
    pub caller: String,
    pub target_surface_id: String,
    pub source_file: String,
    pub line: usize,
    pub arguments: PlannedArguments,
    pub evidence: CallerEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FixtureExample {
    pub target_surface_id: String,
    pub source_file: String,
    pub line: usize,
    pub arguments: PlannedArguments,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<DomainLiteral>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferredProperty {
    pub target_surface_id: String,
    pub contract_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub evidence: CallerEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedInput {
    pub surface_id: String,
    pub arguments: PlannedArguments,
    pub classification: InputClassification,
    pub sources: Vec<DomainSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractSpec {
    pub id: String,
    pub target_surface_id: String,
    pub oracle_kind: OracleKind,
    pub provenance: OracleProvenance,
    pub confidence: FindingConfidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationTarget {
    Direct,
    ExportedCaller {
        caller_surface_id: String,
        source_file: String,
        line: usize,
    },
    FactoryCallable {
        factory_surface_id: String,
        callable_surface_id: String,
    },
    AuthoritativeTest {
        source_file: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionUnit {
    pub surface_id: String,
    pub invocation: InvocationPath,
    pub target: InvocationTarget,
    pub source_file: String,
    pub inputs: Vec<PlannedInput>,
    pub contracts: Vec<ContractSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationPlan {
    pub surfaces: Vec<SurfaceSpec>,
    pub parameter_domains: Vec<ParameterDomain>,
    pub contracts: Vec<ContractSpec>,
    pub inputs: Vec<PlannedInput>,
    pub execution_units: Vec<ExecutionUnit>,
}
