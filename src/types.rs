use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA_VERSION: u32 = 3;

fn is_zero(value: &usize) -> bool {
    *value == 0
}
fn one() -> usize {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    TypeScript,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    Python,
    #[default]
    TypeScript,
    Tsx,
}

impl SourceMode {
    pub fn for_language(language: &Language) -> Self {
        match language {
            Language::Python => Self::Python,
            Language::TypeScript => Self::TypeScript,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: String,
    pub message: String,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceContext {
    pub language: Language,
    pub mode: SourceMode,
    pub source_file: Option<std::path::PathBuf>,
    pub virtual_file_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionContext {
    pub invocation_dir: std::path::PathBuf,
    pub workspace_root: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization_source_root: Option<std::path::PathBuf>,
    pub target_package_root: std::path::PathBuf,
    pub test_package_root: Option<std::path::PathBuf>,
    pub dependency_roots: Vec<std::path::PathBuf>,
    pub target_source: SourceContext,
    pub test_source: Option<SourceContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationContext {
    pub candidate: ExecutionContext,
    pub base: Option<ExecutionContext>,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextRequest<'a> {
    pub invocation_dir: &'a std::path::Path,
    pub explicit_project_dir: Option<&'a std::path::Path>,
    pub target_file: Option<&'a std::path::Path>,
    pub test_file: Option<&'a std::path::Path>,
    pub language: Language,
    pub virtual_file_path: Option<&'a std::path::Path>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContextError {
    InvalidInvocationDirectory(String),
    InvalidProjectDirectory(String),
    MissingSourceFile(String),
    SourceOutsideProject { source: String, project: String },
    TestOutsideProject { test: String, project: String },
    InvalidVirtualPath(String),
    Io(String),
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInvocationDirectory(message)
            | Self::InvalidProjectDirectory(message)
            | Self::MissingSourceFile(message)
            | Self::InvalidVirtualPath(message)
            | Self::Io(message) => f.write_str(message),
            Self::SourceOutsideProject { source, project } => {
                write!(f, "source '{}' is outside project '{}'", source, project)
            }
            Self::TestOutsideProject { test, project } => {
                write!(f, "test '{}' is outside project '{}'", test, project)
            }
        }
    }
}

impl std::error::Error for ContextError {}
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    #[default]
    Deny,
    Allow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTerminationKind {
    Exited,
    Signaled,
    TimedOut,
    MemoryLimit,
    LaunchFailed,
    WaitFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessTermination {
    pub kind: ProcessTerminationKind,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub signal_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionLimits {
    pub timeout_seconds: f64,
    pub memory_mb: u64,
    pub runtime_profile: RuntimeProfile,
    #[serde(default)]
    pub network_policy: NetworkPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureDomain {
    TargetCode,
    VerifierHarness,
    Environment,
    Resource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    InvalidConfiguration,
    SyntaxError,
    ComplexityThreshold,
    TargetException,
    AssertionFailure,
    ContractViolation,
    InvalidGeneratedInput,
    AmbiguousGeneratedInput,
    HarnessProtocol,
    Instrumentation,
    ContextResolution,
    Materialization,
    ModuleLoad,
    UnsupportedSourceMode,
    NetworkDenied,
    ProcessSpawnDenied,
    ToolUnavailable,
    ToolFailure,
    LauncherFailure,
    Timeout,
    MemoryLimit,
    Signal,
    NonzeroExit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticComponent {
    Configuration,
    Target,
    FuzzHarness,
    Sandbox,
    AuthoritativeTestRunner,
    LintRunner,
    ModuleLoader,
    Instrumentation,
    DifferentialRunner,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticImpact {
    Gating,
    Blocking,
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureDiagnostic {
    pub domain: FailureDomain,
    pub kind: FailureKind,
    pub component: DiagnosticComponent,
    pub impact: DiagnosticImpact,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessTermination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<ExecutionLimits>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticsSummary {
    pub total: usize,
    pub gating: usize,
    pub blocking: usize,
    pub advisory: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_domain: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_kind: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    Standalone,
    PortabilityProbe,
    GeneratedVerifier,
    AuthoritativeTest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessRuntime {
    Python,
    NodeScript,
    TsxScript,
    BunScript,
    NodeTest,
    BunTest,
    Vitest,
    Jest,
    RepoTest,
    Jazzer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestAdapter {
    NodeTap,
    BunJunit,
    VitestJson,
    JestJson,
    Opaque,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum HarnessArg {
    Literal { literal: String },
    ProjectPath { project_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HarnessArtifact {
    Generated {
        code: String,
        relative_path: std::path::PathBuf,
    },
    Existing {
        relative_path: std::path::PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessSpec {
    pub kind: HarnessKind,
    pub runtime: HarnessRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_adapter: Option<TestAdapter>,
    pub source_mode: SourceMode,
    pub artifact: HarnessArtifact,
    #[serde(default)]
    pub args: Vec<HarnessArg>,
    #[serde(default)]
    pub network: NetworkPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchPlan {
    pub executable: std::path::PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    pub host_artifact: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_artifact: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessExecution {
    pub process: ExecutionResult,
    #[serde(default)]
    pub diagnostics: Vec<FailureDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReproLaunchContext {
    pub limits: ExecutionLimits,
    pub source_mode: SourceMode,
    pub runtime: HarnessRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_source_mode: Option<SourceMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_runtime: Option<HarnessRuntime>,
    #[serde(default)]
    pub harness_args: Vec<HarnessArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
}

/// Fully validated execution request. Paths are borrowed from the caller so
/// dispatch cannot accidentally outlive the verification request.
#[derive(Debug, Clone, Copy)]
pub struct SandboxOptions<'a> {
    pub timeout_seconds: f64,
    pub memory_mb: u64,
    pub runtime_profile: RuntimeProfile,
    pub network_policy: NetworkPolicy,
    pub harness_args: &'a [HarnessArg],
    pub docker_image: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub source_file: Option<&'a str>,
    /// Absolute original source path to intercept for in-memory instrumentation.
    pub instrumentation_target: Option<&'a str>,
    /// Instrumented source returned by the runner transform; never written into the project.
    pub instrumented_source: Option<&'a str>,
}

impl SandboxOptions<'_> {
    pub fn validate(&self) -> Result<(), String> {
        if !self.timeout_seconds.is_finite() || self.timeout_seconds <= 0.0 {
            return Err("timeout must be finite and greater than zero".into());
        }
        if self.memory_mb == 0 {
            return Err("memory must be greater than zero".into());
        }
        if self
            .memory_mb
            .checked_mul(1024)
            .and_then(|value| value.checked_mul(1024))
            .is_none()
        {
            return Err("memory limit is too large".into());
        }
        if let Some(image) = self.docker_image {
            if image.trim().is_empty() || image.starts_with('-') {
                return Err("docker image must be non-empty and must not begin with '-'".into());
            }
            if self.runtime_profile != RuntimeProfile::Isolated {
                return Err("docker image overrides require the isolated runtime profile".into());
            }
        }
        if self.runtime_profile == RuntimeProfile::Isolated
            && self.network_policy == NetworkPolicy::Allow
        {
            return Err("isolated runtime profile requires network denial".into());
        }
        if self.instrumentation_target.is_some() != self.instrumented_source.is_some() {
            return Err(
                "instrumentation target and instrumented source must be provided together".into(),
            );
        }
        if self.runtime_profile == RuntimeProfile::Isolated && self.docker_image.is_none() {
            return Err("isolated runtime profile requires a docker image".into());
        }
        Ok(())
    }
}

pub const DEFAULT_PYTHON_DOCKER_IMAGE: &str = "python:3.12-slim";
pub const DEFAULT_TYPESCRIPT_DOCKER_IMAGE: &str = "node:24-bookworm-slim";
pub const DEFAULT_BUN_DOCKER_IMAGE: &str = "oven/bun:1.3.14";

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
pub enum NativeFuzzEngine {
    #[default]
    Off,
    Auto,
    Atheris,
    Jazzer,
}

impl NativeFuzzEngine {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "off" => Some(Self::Off),
            "auto" => Some(Self::Auto),
            "atheris" => Some(Self::Atheris),
            "jazzer" | "jazzer-js" | "jazzer_js" => Some(Self::Jazzer),
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VariadicKind {
    Positional,
    Keyword,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variadic: Option<VariadicKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredicateSeed {
    pub parameter: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub property_path: Vec<String>,
    pub value: serde_json::Value,
    pub line: usize,
}

/// Statically observed behavior that makes repeat-call output equality an
/// invalid implicit oracle. Explicit source-declared properties remain active.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FunctionEffect {
    Randomness,
    Time,
    Timer,
    Io,
    MutableState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<ParamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Type parameters declared directly by this callable (for example `T` in `fn<T>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_parameters: Vec<String>,
    /// TypeScript constraints keyed by the callable-local type parameter name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_parameter_constraints: BTreeMap<String, String>,
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
    /// Boundary and literal values observed in branch predicates for this callable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicate_seeds: Vec<PredicateSeed>,
    /// Effects found in this function's own body (nested callable bodies excluded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<FunctionEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returned_callables: Vec<String>,
}

fn ts_constraint_is_atomic(constraint: &str) -> bool {
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return true;
    }

    let mut delimiters = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for character in constraint.chars() {
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
            '(' | '[' | '{' | '<' => delimiters.push(character),
            ')' => {
                if delimiters.pop() != Some('(') {
                    return false;
                }
            }
            ']' => {
                if delimiters.pop() != Some('[') {
                    return false;
                }
            }
            '}' => {
                if delimiters.pop() != Some('{') {
                    return false;
                }
            }
            '>' if delimiters.last() == Some(&'<') => {
                delimiters.pop();
            }
            _ if delimiters.is_empty()
                && (character.is_whitespace()
                    || matches!(character, '|' | '&' | '=' | '?' | ':' | ',')) =>
            {
                return false;
            }
            _ => {}
        }
    }
    quote.is_none() && delimiters.is_empty()
}

fn precedence_safe_ts_constraint(constraint: &str) -> Cow<'_, str> {
    let constraint = constraint.trim();
    if ts_constraint_is_atomic(constraint) {
        Cow::Borrowed(constraint)
    } else {
        Cow::Owned(format!("({constraint})"))
    }
}

impl FunctionInfo {
    pub fn resolved_type_annotation<'a>(&'a self, annotation: &'a str) -> Cow<'a, str> {
        if self.type_parameter_constraints.is_empty() {
            return Cow::Borrowed(annotation);
        }

        let mut resolved = String::with_capacity(annotation.len());
        let mut copied_until = 0usize;
        let mut token_start = None;
        for (index, character) in annotation
            .char_indices()
            .chain(std::iter::once((annotation.len(), ' ')))
        {
            let identifier_character =
                character.is_alphanumeric() || matches!(character, '_' | '$');
            match (token_start, identifier_character) {
                (None, true) => token_start = Some(index),
                (Some(start), false) => {
                    let token = &annotation[start..index];
                    if let Some(constraint) = self.type_parameter_constraints.get(token) {
                        resolved.push_str(&annotation[copied_until..start]);
                        resolved.push_str(&precedence_safe_ts_constraint(constraint));
                        copied_until = index;
                    }
                    token_start = None;
                }
                _ => {}
            }
        }
        if copied_until == 0 {
            Cow::Borrowed(annotation)
        } else {
            resolved.push_str(&annotation[copied_until..]);
            Cow::Owned(resolved)
        }
    }
}

impl ParamInfo {
    pub fn is_variadic(&self) -> bool {
        self.variadic.is_some()
    }

    pub fn is_positional_variadic(&self) -> bool {
        self.variadic == Some(VariadicKind::Positional)
    }

    pub fn is_keyword_variadic(&self) -> bool {
        self.variadic == Some(VariadicKind::Keyword)
    }
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
    #[serde(default)]
    pub source_mode: SourceMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parse_diagnostics: Vec<ParseDiagnostic>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub memory_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<ProcessTermination>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FailureDiagnostic>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputOrigin {
    Generated,
    ObservedCall,
    Fixture,
    SafeDependencySubstitute,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnitOutcome {
    Passed,
    Rejected,
    TargetException,
    InvalidGeneratedInput,
    UnclassifiedException,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessEventRecord {
    pub protocol_version: u32,
    pub sequence: u64,
    #[serde(flatten)]
    pub event: HarnessEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum HarnessEvent {
    BootstrapStarted,
    TargetResolved {
        module: String,
    },
    BootstrapFailed {
        domain: FailureDomain,
        kind: FailureKind,
        message: String,
    },
    TargetReady,
    UnitStarted {
        surface_id: String,
        iteration: usize,
        input_classification: InputClassification,
        input_origin: InputOrigin,
    },
    Finding {
        finding: VerificationFinding,
    },
    OracleEvaluated {
        surface_id: String,
        iteration: usize,
        oracle_id: String,
        passed: bool,
    },
    UnitCompleted {
        surface_id: String,
        iteration: usize,
        outcome: UnitOutcome,
    },
    HarnessCompleted {
        completed_units: usize,
    },
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
    #[serde(default)]
    pub occurrences: usize,
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
    #[serde(default = "one")]
    pub occurrences: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_inputs: Vec<ReproCase>,
    pub id: String,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub category: FindingCategory,
    pub location: FindingLocation,
    pub oracle: OracleInfo,
    pub input_classification: InputClassification,
    pub repro: StructuredRepro,
    pub minimization: MinimizationInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_context: Option<ReproLaunchContext>,
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
    ReachedDirect,
    ReachedViaFactory,
    ReachedViaAuthoritativeTest,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAdapterKind {
    Standalone,
    Nuxt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRuntimeAdapterKind {
    PlainPython,
    PlainTypeScript,
    Bun,
    VitestVite,
    Nuxt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectAdapterCapabilities {
    pub authoritative_source_overlay: bool,
    pub package_runtime: bool,
    pub project_test_runner: bool,
    pub framework_auto_import_runtime: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectAdapterContract {
    pub kind: ProjectAdapterKind,
    pub root: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub package_root: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_runner: Option<ProjectRuntimeAdapterKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rationale: Vec<String>,
    pub capabilities: ProjectAdapterCapabilities,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceExecutionStrategy {
    GeneratedHarness,
    FrameworkRuntime,
    AuthoritativeProjectRunner,
    StaticOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceExecutionPlan {
    pub surface_id: String,
    pub strategy: SurfaceExecutionStrategy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_requirements: Vec<String>,
    pub expected_evidence: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrthogonalOutcome {
    Passed,
    Failed,
    Blocked,
    NotRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationOutcomeMatrix {
    pub static_analysis: OrthogonalOutcome,
    pub generated_execution: OrthogonalOutcome,
    pub authoritative_tests: OrthogonalOutcome,
    pub portability: OrthogonalOutcome,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StagePolicy {
    Gate,
    Advisory,
}

impl DiagnosticsSummary {
    pub fn from_diagnostics(diagnostics: &[FailureDiagnostic]) -> Self {
        let mut summary = Self::default();
        for diagnostic in diagnostics {
            summary.total += 1;
            match diagnostic.impact {
                DiagnosticImpact::Gating => summary.gating += 1,
                DiagnosticImpact::Blocking => summary.blocking += 1,
                DiagnosticImpact::Advisory => summary.advisory += 1,
            }
            let domain = serde_json::to_string(&diagnostic.domain)
                .unwrap_or_else(|_| "unknown".into())
                .trim_matches('"')
                .to_string();
            let kind = serde_json::to_string(&diagnostic.kind)
                .unwrap_or_else(|_| "unknown".into())
                .trim_matches('"')
                .to_string();
            *summary.by_domain.entry(domain).or_default() += 1;
            *summary.by_kind.entry(kind).or_default() += 1;
        }
        summary
    }
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolProvenance {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateProvenance {
    pub content_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: u32,
    #[serde(default)]
    pub tool: ToolProvenance,
    #[serde(default)]
    pub candidate: CandidateProvenance,
    pub stages: Vec<VerificationStage>,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub summary: ReportSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FailureDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics_summary: Option<DiagnosticsSummary>,
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
    #[serde(default)]
    pub diagnostics: DiagnosticsSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedReport {
    pub schema_version: u32,
    pub meta: ReportMeta,
    #[serde(default)]
    pub tool: ToolProvenance,
    #[serde(default)]
    pub candidate: CandidateProvenance,
    pub stages: Vec<VerificationStage>,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub summary: ReportSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FailureDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics_summary: Option<DiagnosticsSummary>,
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
    #[serde(default)]
    pub tool: ToolProvenance,
    #[serde(default)]
    pub candidate: CandidateProvenance,
    pub meta: ReportMeta,
    pub verdict: VerificationVerdict,
    pub strength: VerificationStrength,
    pub summary: ReportSummary,
    pub recommended_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_finding: Option<VerificationFinding>,
    pub findings: Vec<VerificationFinding>,
    pub coverage: CoverageSummary,
    #[serde(default)]
    pub diagnostics: Vec<FailureDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics_summary: Option<DiagnosticsSummary>,
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
    /// Positive completion of the recorded check, not merely absence of the old failure.
    /// Older repros and unsupported observations have no such evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_passed: Option<bool>,
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
    BigInt,
    String,
    Bytes,
    Literal(Vec<DomainLiteral>),
    Nullable(Box<DomainNode>),
    Union(Vec<DomainNode>),
    Array(Box<DomainNode>),
    Tuple(Vec<DomainNode>),
    Set(Box<DomainNode>),
    Map(Box<DomainNode>, Box<DomainNode>),
    NativeMap(Box<DomainNode>, Box<DomainNode>),
    Object(Vec<DomainField>),
    Instance {
        name: String,
        fields: Vec<DomainField>,
    },
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
pub enum UnsafeDefaultReason {
    Untyped,
    Opaque,
    Overloaded,
    Unsynthesizable,
    SubstituteUnavailable,
}

impl UnsafeDefaultReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Untyped => "untyped",
            Self::Opaque => "opaque",
            Self::Overloaded => "overloaded",
            Self::Unsynthesizable => "unsynthesizable",
            Self::SubstituteUnavailable => "substitute_unavailable",
        }
    }
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
    SafeDependencySubstitute,
    CoverageCorpus,
}

/// Stable textual reason used in coverage diagnostics for a dependency whose
/// default cannot be replaced safely.
pub fn unsafe_default_dependency_reason(parameter: &str, reason: UnsafeDefaultReason) -> String {
    format!("unsafe_default_dependency:{parameter}:{}", reason.code())
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keyword_only: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variadic: Option<VariadicKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlannedArgumentSlot {
    Single(DomainLiteral),
    PositionalVariadic(Vec<DomainLiteral>),
    KeywordVariadic(BTreeMap<String, DomainLiteral>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedArgumentSlots {
    pub slots: Vec<PlannedArgumentSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BindingError {
    TooFewSlots { expected: usize, actual: usize },
    TooManySlots { expected: usize, actual: usize },
    MissingSlot { parameter: String },
    InvalidSlot { parameter: String, message: String },
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
