pub mod tools;
pub mod types;

use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::types::Language;

// ── MCP tool parameter types ────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeParams {
    /// Source code to analyze (provide this or file_path, not both)
    #[serde(default)]
    pub code: String,
    /// Path to source file (alternative to inline code)
    #[serde(default)]
    pub file_path: Option<String>,
    /// Programming language: "python" or "typescript"
    pub language: String,
    /// Functions exceeding this complexity are flagged
    #[serde(default)]
    pub complexity_threshold: Option<usize>,
    /// Unified diff string — only report functions overlapping changed lines
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Source code to execute in sandbox (provide this or file_path, not both)
    #[serde(default)]
    pub code: String,
    /// Path to source file (alternative to inline code)
    #[serde(default)]
    pub file_path: Option<String>,
    /// Programming language: "python" or "typescript"
    pub language: String,
    /// Timeout in seconds (default: 10)
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    /// Memory limit in MB (default: 128)
    #[serde(default)]
    pub memory_mb: Option<u64>,
    /// Project directory for venv/node_modules resolution
    #[serde(default)]
    pub project_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LintParams {
    /// Source code to lint (provide this or file_path, not both)
    #[serde(default)]
    pub code: String,
    /// Path to source file (alternative to inline code)
    #[serde(default)]
    pub file_path: Option<String>,
    /// Programming language: "python" or "typescript"
    pub language: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct VerifyParams {
    /// Source code to verify (provide this or file_path, not both)
    #[serde(default)]
    pub code: String,
    /// Path to source file (alternative to inline code)
    #[serde(default)]
    pub file_path: Option<String>,
    /// Programming language: "python" or "typescript"
    pub language: String,
    /// Optional test code to run against the source
    #[serde(default)]
    pub test_code: Option<String>,
    /// Path to test file (alternative to inline test_code)
    #[serde(default)]
    pub test_file_path: Option<String>,
    /// Functions exceeding this complexity are flagged
    #[serde(default)]
    pub complexity_threshold: Option<usize>,
    /// Project directory for venv/node_modules resolution
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Unified diff string — only fuzz functions overlapping changed lines
    #[serde(default)]
    pub diff: Option<String>,
    /// Directory to write structured JSON report files
    #[serde(default)]
    pub output_dir: Option<String>,
}

// ── MCP Server ──────────────────────────────────────────────────────────────

/// Default cap on concurrent subprocess executions (lint, execute, verify all spawn
/// processes). Set to 1 historically to avoid interleaved sandboxes fighting over
/// the same on-disk artifacts. Overridable via `COURT_JESTER_MAX_CONCURRENT_EXEC`.
const DEFAULT_MAX_CONCURRENT_EXEC: usize = 1;

fn max_concurrent_exec() -> usize {
    std::env::var("COURT_JESTER_MAX_CONCURRENT_EXEC")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_CONCURRENT_EXEC)
}

#[derive(Debug, Clone)]
pub struct CourtJester {
    tool_router: ToolRouter<Self>,
    exec_semaphore: Arc<Semaphore>,
}

/// Build a uniform JSON error response for pre-tool validation failures.
/// Every tool surfaces the same shape so agents can rely on a single contract:
/// `{"error": "...", "error_kind": "..."}`.
pub fn tool_error(kind: &str, message: impl AsRef<str>) -> String {
    let value = serde_json::json!({
        "error": message.as_ref(),
        "error_kind": kind,
    });
    serde_json::to_string_pretty(&value).expect("serde_json::to_string_pretty on json! never fails")
}

pub fn resolve_code(code: &str, file_path: Option<&str>) -> Result<String, String> {
    match (code.is_empty(), file_path) {
        (false, None) => Ok(code.to_string()),
        (true, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| tool_error("read_failed", format!("Cannot read '{}': {}", path, e))),
        (false, Some(_)) => Err(tool_error(
            "ambiguous_input",
            "Provide either 'code' or 'file_path', not both",
        )),
        (true, None) => Err(tool_error(
            "missing_input",
            "Must provide 'code' or 'file_path'",
        )),
    }
}

fn parse_lang(s: &str) -> Result<Language, String> {
    Language::parse(s).ok_or_else(|| {
        tool_error(
            "unsupported_language",
            format!(
                "Unsupported language '{}'. Use 'python' or 'typescript'.",
                s
            ),
        )
    })
}

/// Walk up from a file path to find the nearest project root with dependencies.
/// Prefers directories with actual node_modules/.venv (installed deps) over bare
/// package.json/pyproject.toml, to handle monorepos where deps are hoisted.
pub fn detect_project_dir(file_path: &str) -> Option<String> {
    let path = std::path::Path::new(file_path);
    let mut dir = path.parent()?;
    let mut fallback: Option<String> = None;
    loop {
        // Strong signal: actual installed dependencies
        if dir.join("node_modules").is_dir() || dir.join(".venv").is_dir() {
            return Some(dir.to_string_lossy().to_string());
        }
        // Weak signal: project marker without installed deps (e.g. monorepo sub-package)
        if fallback.is_none()
            && (dir.join("package.json").is_file() || dir.join("pyproject.toml").is_file())
        {
            fallback = Some(dir.to_string_lossy().to_string());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    fallback
}

#[tool_router]
impl CourtJester {
    /// Parse code with tree-sitter to extract function signatures, classes, imports,
    /// and cyclomatic complexity.
    #[tool(
        name = "analyze",
        description = "Static analysis via tree-sitter AST: extracts functions, classes, imports, and complexity"
    )]
    async fn analyze(&self, Parameters(p): Parameters<AnalyzeParams>) -> String {
        let code = match resolve_code(&p.code, p.file_path.as_deref()) {
            Ok(c) => c,
            Err(e) => return e,
        };
        match parse_lang(&p.language) {
            Ok(lang) => {
                let analysis = tools::analyze::analyze(&code, &lang);
                let mut value = serde_json::to_value(&analysis).unwrap();

                // Diff-aware filtering
                if let Some(diff_str) = &p.diff {
                    let changed_ranges = p
                        .file_path
                        .as_deref()
                        .map(|path| tools::diff::parse_changed_lines_for_file(diff_str, path))
                        .unwrap_or_else(|| tools::diff::parse_changed_lines(diff_str));
                    let changed_fns =
                        tools::analyze::filter_changed_functions(&analysis, &changed_ranges);
                    value["changed_functions"] = serde_json::to_value(&changed_fns).unwrap();
                }

                // Complexity threshold
                if let Some(threshold) = p.complexity_threshold {
                    let violations =
                        tools::analyze::check_complexity_threshold(&analysis, threshold);
                    value["complexity_violations"] = serde_json::to_value(&violations).unwrap();
                    value["complexity_ok"] = serde_json::Value::Bool(violations.is_empty());
                }

                serde_json::to_string_pretty(&value).unwrap()
            }
            Err(e) => e,
        }
    }

    /// Execute code in a sandboxed subprocess with memory, CPU, and time limits.
    #[tool(
        name = "execute",
        description = "Run code in a sandboxed subprocess with resource limits (memory, CPU, timeout)"
    )]
    async fn execute(&self, Parameters(p): Parameters<ExecuteParams>) -> String {
        let code = match resolve_code(&p.code, p.file_path.as_deref()) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let project_dir = p
            .project_dir
            .or_else(|| p.file_path.as_deref().and_then(detect_project_dir));
        match parse_lang(&p.language) {
            Ok(lang) => {
                let _permit = self.exec_semaphore.acquire().await.unwrap();
                let timeout = p.timeout_seconds.unwrap_or(10.0);
                let memory = p.memory_mb.unwrap_or(128);
                let result = tools::sandbox::execute(
                    &code,
                    &lang,
                    timeout,
                    memory,
                    project_dir.as_deref(),
                    p.file_path.as_deref(),
                )
                .await;
                serde_json::to_string_pretty(&result).unwrap()
            }
            Err(e) => e,
        }
    }

    /// Lint code using ruff (Python) or biome (TypeScript).
    #[tool(
        name = "lint",
        description = "Run language-aware linting: ruff for Python, biome for TypeScript"
    )]
    async fn lint(&self, Parameters(p): Parameters<LintParams>) -> String {
        let code = match resolve_code(&p.code, p.file_path.as_deref()) {
            Ok(c) => c,
            Err(e) => return e,
        };
        match parse_lang(&p.language) {
            Ok(lang) => {
                let _permit = self.exec_semaphore.acquire().await.unwrap();
                let result = tools::lint::lint(&code, &lang).await;
                serde_json::to_string_pretty(&result).unwrap()
            }
            Err(e) => e,
        }
    }

    /// Full verification pipeline: parse → lint → synthesize+execute → test.
    #[tool(
        name = "verify",
        description = "Full pipeline: parse AST, lint, synthesize test inputs from types, execute in sandbox, optionally run tests"
    )]
    async fn verify(&self, Parameters(p): Parameters<VerifyParams>) -> String {
        let code = match resolve_code(&p.code, p.file_path.as_deref()) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let test_code = match p.test_file_path.as_deref() {
            Some(path) => match resolve_code("", Some(path)) {
                Ok(c) => Some(c),
                Err(e) => return e,
            },
            None => p.test_code,
        };
        let project_dir = p
            .project_dir
            .or_else(|| p.file_path.as_deref().and_then(detect_project_dir));
        match parse_lang(&p.language) {
            Ok(lang) => {
                let _permit = self.exec_semaphore.acquire().await.unwrap();
                let opts = tools::verify::VerifyOptions {
                    test_code: test_code.as_deref(),
                    test_source_file: p.test_file_path.as_deref(),
                    complexity_threshold: p.complexity_threshold,
                    project_dir: project_dir.as_deref(),
                    diff: p.diff.as_deref(),
                    source_file: p.file_path.as_deref(),
                    output_dir: p.output_dir.as_deref(),
                };
                let result = tools::verify::verify(&code, &lang, opts).await;
                serde_json::to_string_pretty(&result).unwrap()
            }
            Err(e) => e,
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CourtJester {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "court-jester",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Code verification gate: analyze, execute, lint, and verify Python/TypeScript code",
            )
    }
}

impl CourtJester {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            exec_semaphore: Arc::new(Semaphore::new(max_concurrent_exec())),
        }
    }
}
