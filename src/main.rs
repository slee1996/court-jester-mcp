use std::collections::BTreeSet;
use std::env;
use std::path::Path;
use std::process::{Command, ExitCode};

use court_jester_mcp::types::{
    ComplexityMetric, ExecuteGate, Language, ReportLevel, SummaryFormat, TestRunner,
};
use court_jester_mcp::{detect_project_dir, parse_language, tools};

const USAGE: &str = "\
court-jester — code verification CLI for Python and TypeScript

USAGE:
  court-jester verify   [OPTIONS]   Verify a file and print a JSON report
  court-jester ci       [OPTIONS]   Verify changed files for PR/CI workflows
  court-jester analyze  [OPTIONS]   Run tree-sitter analysis
  court-jester lint     [OPTIONS]   Run Ruff or Biome
  court-jester execute  [OPTIONS]   Run code in the sandbox
  court-jester --help               Print this help
  court-jester --version            Print the version

COMMON OPTIONS:
  --file <PATH>              Source file (required for all subcommands)
  --language <LANG>          python | typescript (required)
  --project-dir <PATH>       venv / node_modules root (auto-detected if omitted)
  --config-path <PATH>       Explicit Ruff/Biome config path for lint + verify
  --virtual-file-path <PATH> Virtual lint path for temp or generated source files

VERIFY OPTIONS:
  --test-file <PATH>         Test file to include as an authoritative stage
  --test-runner <MODE>       auto | node | bun | repo-native (default auto)
  --tests-only               Skip fuzz-execute and run only the authoritative test stage
  --output-dir <PATH>        Directory to write persistent JSON reports
  --report-level <LEVEL>     full | minimal (default full)
  --summary <FORMAT>         json | human (default json)
  --suppressions-file <PATH> JSON suppression rules for known findings
  --no-auto-seed             Disable seed extraction from nearby tests and literal call sites
  --diff-file <PATH>         Unified-diff file — only inspect changed functions
  --profile <NAME>           Verification profile preset (currently: security => complexity 20)
  --complexity-metric <NAME> cyclomatic | cognitive (default cyclomatic)
  --complexity-threshold <N> Fail if any function exceeds this complexity (changed functions only when --diff-file is set)
  --execute-gate <MODE>      all | crash | none (default all; no_inputs_reached is always diagnostic)

CI OPTIONS:
  --base <REV>               Base revision for changed-file diffing (required for `ci`)
  --head <REV>               Head revision for changed-file diffing (default HEAD)
  --gate <LIST>              Comma-separated stage gates or all (default parse,lint,portability,execute,test)
  --report <FORMAT>          human | github | json (default human)

EXECUTE OPTIONS:
  --timeout-seconds <F>      Sandbox timeout (default 10)
  --memory-mb <N>            Sandbox memory cap MB (default 128)

ENVIRONMENT:
  COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS      Python fuzz-exec timeout (default 10)
  COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS  TS fuzz-exec timeout (default 25)
  COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS        Test stage timeout (default 30)

EXAMPLES:
  court-jester verify --file src/profile.py --language python
  court-jester verify --file src/semver.ts --language typescript \\
      --test-file tests/semver.test.ts --output-dir .court-jester/reports
  court-jester ci --base origin/main --gate complexity,portability --report github
  court-jester lint --file src/parser.py --language python --config-path pyproject.toml
";

const CI_ALL_GATES: [&str; 7] = [
    "parse",
    "complexity",
    "lint",
    "coverage",
    "portability",
    "execute",
    "test",
];
const CI_DEFAULT_GATES: [&str; 5] = ["parse", "lint", "portability", "execute", "test"];

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        eprint!("{}", USAGE);
        return ExitCode::from(2);
    }

    match args[0].as_str() {
        "-h" | "--help" => {
            print!("{}", USAGE);
            ExitCode::SUCCESS
        }
        "-V" | "--version" => {
            println!("court-jester {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "verify" | "ci" | "analyze" | "lint" | "execute" => {
            match run_subcommand(&args[0], &args[1..]).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {}", e);
                    ExitCode::from(2)
                }
            }
        }
        other => {
            eprintln!("error: unknown subcommand '{}'\n", other);
            eprint!("{}", USAGE);
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Default)]
struct CliArgs {
    file: Option<String>,
    language: Option<String>,
    base: Option<String>,
    head: Option<String>,
    gate: Option<String>,
    ci_report_format: CiReportFormat,
    project_dir: Option<String>,
    config_path: Option<String>,
    virtual_file_path: Option<String>,
    test_file: Option<String>,
    test_runner: TestRunner,
    tests_only: bool,
    output_dir: Option<String>,
    report_level: ReportLevel,
    summary_format: SummaryFormat,
    suppressions_file: Option<String>,
    no_auto_seed: bool,
    diff_file: Option<String>,
    profile: Option<String>,
    complexity_metric: ComplexityMetric,
    complexity_threshold: Option<usize>,
    execute_gate: ExecuteGate,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CiReportFormat {
    Human,
    Github,
    Json,
}

impl Default for CiReportFormat {
    fn default() -> Self {
        Self::Human
    }
}

impl CiReportFormat {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "human" => Some(Self::Human),
            "github" => Some(Self::Github),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

fn parse_flags(rest: &[String]) -> Result<CliArgs, String> {
    let mut out = CliArgs::default();
    let mut i = 0;
    while i < rest.len() {
        let flag = rest[i].as_str();
        let take_value = |i: &mut usize| -> Result<String, String> {
            if *i + 1 >= rest.len() {
                return Err(format!("flag {} requires a value", flag));
            }
            *i += 1;
            Ok(rest[*i].clone())
        };
        match flag {
            "--file" => out.file = Some(take_value(&mut i)?),
            "--language" => out.language = Some(take_value(&mut i)?),
            "--base" => out.base = Some(take_value(&mut i)?),
            "--head" => out.head = Some(take_value(&mut i)?),
            "--gate" => out.gate = Some(take_value(&mut i)?),
            "--report" => {
                let raw = take_value(&mut i)?;
                out.ci_report_format = CiReportFormat::parse(&raw).ok_or_else(|| {
                    format!(
                        "--report must be one of: human, github, json (got '{}')",
                        raw
                    )
                })?;
            }
            "--project-dir" => out.project_dir = Some(take_value(&mut i)?),
            "--config-path" => out.config_path = Some(take_value(&mut i)?),
            "--virtual-file-path" => out.virtual_file_path = Some(take_value(&mut i)?),
            "--test-file" => out.test_file = Some(take_value(&mut i)?),
            "--test-runner" => {
                let raw = take_value(&mut i)?;
                out.test_runner = TestRunner::parse(&raw).ok_or_else(|| {
                    format!(
                        "--test-runner must be one of: auto, node, bun, repo-native (got '{}')",
                        raw
                    )
                })?;
            }
            "--tests-only" => out.tests_only = true,
            "--output-dir" => out.output_dir = Some(take_value(&mut i)?),
            "--report-level" => {
                let raw = take_value(&mut i)?;
                out.report_level = ReportLevel::parse(&raw).ok_or_else(|| {
                    format!(
                        "--report-level must be one of: full, minimal (got '{}')",
                        raw
                    )
                })?;
            }
            "--summary" => {
                let raw = take_value(&mut i)?;
                out.summary_format = SummaryFormat::parse(&raw).ok_or_else(|| {
                    format!("--summary must be one of: json, human (got '{}')", raw)
                })?;
            }
            "--suppressions-file" => out.suppressions_file = Some(take_value(&mut i)?),
            "--no-auto-seed" => out.no_auto_seed = true,
            "--diff-file" => out.diff_file = Some(take_value(&mut i)?),
            "--profile" => out.profile = Some(take_value(&mut i)?),
            "--complexity-metric" => {
                let raw = take_value(&mut i)?;
                out.complexity_metric = ComplexityMetric::parse(&raw).ok_or_else(|| {
                    format!(
                        "--complexity-metric must be one of: cyclomatic, cognitive (got '{}')",
                        raw
                    )
                })?;
            }
            "--complexity-threshold" => {
                let raw = take_value(&mut i)?;
                out.complexity_threshold = Some(raw.parse::<usize>().map_err(|_| {
                    format!(
                        "--complexity-threshold must be a non-negative integer, got '{}'",
                        raw
                    )
                })?);
            }
            "--execute-gate" => {
                let raw = take_value(&mut i)?;
                out.execute_gate = ExecuteGate::parse(&raw).ok_or_else(|| {
                    format!(
                        "--execute-gate must be one of: all, crash, none (got '{}')",
                        raw
                    )
                })?;
            }
            "--timeout-seconds" => {
                let raw = take_value(&mut i)?;
                out.timeout_seconds =
                    Some(raw.parse::<f64>().map_err(|_| {
                        format!("--timeout-seconds must be a number, got '{}'", raw)
                    })?);
            }
            "--memory-mb" => {
                let raw = take_value(&mut i)?;
                out.memory_mb = Some(raw.parse::<u64>().map_err(|_| {
                    format!("--memory-mb must be a positive integer, got '{}'", raw)
                })?);
            }
            "-h" | "--help" => {
                print!("{}", USAGE);
                std::process::exit(0);
            }
            other => {
                if other.starts_with("--") && other.contains(' ') {
                    let mut parts = other.split_whitespace();
                    if let (Some(flag_name), Some(flag_value)) = (parts.next(), parts.next()) {
                        return Err(format!(
                            "unknown flag '{}'; did you mean '{}' and '{}' as separate arguments?",
                            other, flag_name, flag_value
                        ));
                    }
                }
                return Err(format!("unknown flag '{}'", other));
            }
        }
        i += 1;
    }
    Ok(out)
}

fn require_file(args: &CliArgs) -> Result<&str, String> {
    args.file
        .as_deref()
        .ok_or_else(|| "--file is required".to_string())
}

fn require_language(args: &CliArgs) -> Result<Language, String> {
    let raw = args
        .language
        .as_deref()
        .ok_or_else(|| "--language is required".to_string())?;
    parse_language(raw).map_err(|json_error| {
        serde_json::from_str::<serde_json::Value>(&json_error)
            .ok()
            .and_then(|value| value["error"].as_str().map(ToOwned::to_owned))
            .unwrap_or(json_error)
    })
}

fn require_base(args: &CliArgs) -> Result<&str, String> {
    args.base
        .as_deref()
        .ok_or_else(|| "--base is required for `court-jester ci`".to_string())
}

fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {}", path, e))
}

fn read_optional_file(path: Option<&str>) -> Result<Option<String>, String> {
    match path {
        Some(path) => Ok(Some(read_file(path)?)),
        None => Ok(None),
    }
}

fn resolve_complexity_threshold(args: &CliArgs) -> Result<Option<usize>, String> {
    if let Some(threshold) = args.complexity_threshold {
        return Ok(Some(threshold));
    }

    match args.profile.as_deref() {
        None => Ok(None),
        Some("security") => Ok(Some(20)),
        Some(other) => Err(format!(
            "unknown profile '{}'; supported profiles: security",
            other
        )),
    }
}

#[derive(Debug, Clone)]
struct CiFileResult {
    file: String,
    language: Language,
    selected_gate_ok: bool,
    failing_gates: Vec<String>,
    report: court_jester_mcp::types::VerificationReport,
}

#[derive(Debug, Clone)]
struct CiRunResult {
    base: String,
    head: String,
    gates: Vec<String>,
    changed_files: usize,
    checked_files: usize,
    skipped_files: Vec<String>,
    files: Vec<CiFileResult>,
    overall_ok: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CiJsonFileResult {
    file: String,
    language: String,
    selected_gate_ok: bool,
    failing_gates: Vec<String>,
    report: serde_json::Value,
}

fn parse_ci_gates(raw: Option<&str>) -> Result<Vec<String>, String> {
    let requested: Vec<&str> = match raw {
        Some("all") => CI_ALL_GATES.to_vec(),
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|gate| !gate.is_empty())
            .collect(),
        None => CI_DEFAULT_GATES.to_vec(),
    };
    if requested.is_empty() {
        return Err("--gate requires at least one stage name".into());
    }
    let allowed: BTreeSet<&str> = CI_ALL_GATES.iter().copied().collect();
    let mut gates = BTreeSet::new();
    for gate in requested {
        if !allowed.contains(gate) {
            return Err(format!(
                "unsupported ci gate '{}'; expected one of: {}",
                gate,
                CI_ALL_GATES.join(", ")
            ));
        }
        gates.insert(gate.to_string());
    }
    Ok(gates.into_iter().collect())
}

fn ci_stage_failures(
    report: &court_jester_mcp::types::VerificationReport,
    gates: &[String],
) -> Vec<String> {
    let selected: BTreeSet<&str> = gates.iter().map(String::as_str).collect();
    report
        .stages
        .iter()
        .filter(|stage| selected.contains(stage.name.as_str()) && !stage.ok)
        .map(|stage| stage.name.clone())
        .collect()
}

fn ci_language_for_path(path: &str) -> Option<Language> {
    let path = Path::new(path);
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Some(Language::Python),
        Some("ts") => Some(Language::TypeScript),
        _ => None,
    }
}

fn ci_language_name(language: &Language) -> &'static str {
    match language {
        Language::Python => "python",
        Language::TypeScript => "typescript",
    }
}

fn git_output(repo_dir: &Path, args: &[String]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .output()
        .map_err(|e| format!("failed to run git {}: {}", args.join(" "), e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "git {} failed{}",
            args.join(" "),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {}", stderr)
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn ci_changed_source_files(
    repo_dir: &Path,
    base: &str,
    head: &str,
) -> Result<Vec<(String, Language)>, String> {
    let range = format!("{base}...{head}");
    let output = git_output(
        repo_dir,
        &[
            "diff".into(),
            "--name-only".into(),
            "--diff-filter=ACMRTUXB".into(),
            range,
        ],
    )?;
    let mut files = Vec::new();
    for line in output.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        if let Some(language) = ci_language_for_path(path) {
            files.push((path.to_string(), language));
        }
    }
    Ok(files)
}

fn ci_unified_diff(repo_dir: &Path, base: &str, head: &str) -> Result<String, String> {
    let range = format!("{base}...{head}");
    git_output(repo_dir, &["diff".into(), "--unified=0".into(), range])
}

async fn run_ci_for_repo(repo_dir: &Path, args: &CliArgs) -> Result<CiRunResult, String> {
    if args.file.is_some() || args.language.is_some() {
        return Err("`court-jester ci` does not accept --file or --language".into());
    }
    if args.test_file.is_some() || args.tests_only {
        return Err("`court-jester ci` does not support --test-file or --tests-only yet".into());
    }
    let base = require_base(args)?.to_string();
    let head = args.head.clone().unwrap_or_else(|| "HEAD".into());
    let gates = parse_ci_gates(args.gate.as_deref())?;
    let changed_files = ci_changed_source_files(repo_dir, &base, &head)?;
    let diff = if changed_files.is_empty() {
        String::new()
    } else {
        ci_unified_diff(repo_dir, &base, &head)?
    };
    let complexity_threshold = resolve_complexity_threshold(args)?;
    let mut files = Vec::new();
    let mut skipped_files = Vec::new();
    let mut overall_ok = true;

    for (relative_path, language) in &changed_files {
        let absolute = repo_dir.join(relative_path);
        if !absolute.is_file() {
            skipped_files.push(relative_path.clone());
            continue;
        }
        let absolute_string = absolute.to_string_lossy().to_string();
        let code = read_file(&absolute_string)?;
        let project_dir = args
            .project_dir
            .clone()
            .or_else(|| detect_project_dir(&absolute_string));
        let report = tools::verify::verify(
            &code,
            language,
            tools::verify::VerifyOptions {
                test_code: None,
                test_source_file: None,
                test_runner: args.test_runner,
                complexity_threshold,
                complexity_metric: args.complexity_metric,
                project_dir: project_dir.as_deref(),
                lint_config_path: args.config_path.as_deref(),
                lint_virtual_file_path: None,
                diff: if diff.is_empty() {
                    None
                } else {
                    Some(diff.as_str())
                },
                suppressions: None,
                suppression_source: None,
                auto_seed: !args.no_auto_seed,
                source_file: Some(absolute_string.as_str()),
                output_dir: args.output_dir.as_deref(),
                report_level: args.report_level,
                execute_gate: args.execute_gate,
                tests_only: false,
            },
        )
        .await;
        let failing_gates = ci_stage_failures(&report, &gates);
        let selected_gate_ok = failing_gates.is_empty();
        if !selected_gate_ok {
            overall_ok = false;
        }
        files.push(CiFileResult {
            file: relative_path.clone(),
            language: language.clone(),
            selected_gate_ok,
            failing_gates,
            report,
        });
    }

    Ok(CiRunResult {
        base,
        head,
        gates,
        changed_files: changed_files.len(),
        checked_files: files.len(),
        skipped_files,
        files,
        overall_ok,
    })
}

fn ci_stage_brief(stage: &court_jester_mcp::types::VerificationStage) -> String {
    match stage.name.as_str() {
        "complexity" => {
            let count = stage
                .detail
                .as_ref()
                .and_then(|detail| detail.get("violations"))
                .and_then(|value| value.as_array())
                .map(|value| value.len())
                .unwrap_or(0);
            format!("{count} violation(s)")
        }
        "execute" => {
            let crashes = stage
                .detail
                .as_ref()
                .and_then(|detail| detail.get("finding_counts"))
                .and_then(|counts| counts.get("crash"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let property = stage
                .detail
                .as_ref()
                .and_then(|detail| detail.get("finding_counts"))
                .and_then(|counts| counts.get("property_violation"))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            format!("{crashes} crash(es), {property} property violation(s)")
        }
        "test" | "parse" | "portability" | "lint" => {
            stage.error.clone().unwrap_or_else(|| "stage failed".into())
        }
        _ => stage.error.clone().unwrap_or_else(|| "stage failed".into()),
    }
}

fn render_ci_human(result: &CiRunResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "CI: {}\n",
        if result.overall_ok { "PASS" } else { "FAIL" }
    ));
    out.push_str(&format!("Range: {}...{}\n", result.base, result.head));
    out.push_str(&format!(
        "Files: {} changed, {} checked, {} skipped\n",
        result.changed_files,
        result.checked_files,
        result.skipped_files.len()
    ));
    out.push_str(&format!("Gates: {}\n", result.gates.join(", ")));
    if !result.skipped_files.is_empty() {
        out.push_str(&format!("Skipped: {}\n", result.skipped_files.join(", ")));
    }
    if result.files.iter().all(|file| file.selected_gate_ok) {
        return out;
    }
    out.push_str("\nFailing Files:\n");
    for file in result.files.iter().filter(|file| !file.selected_gate_ok) {
        out.push_str(&format!(
            "- {} [{}]\n",
            file.file,
            file.failing_gates.join(", ")
        ));
        for gate in &file.failing_gates {
            if let Some(stage) = file.report.stages.iter().find(|stage| stage.name == *gate) {
                out.push_str(&format!("  {}: {}\n", gate, ci_stage_brief(stage)));
            }
        }
    }
    out
}

fn github_escape(message: &str) -> String {
    message
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn render_ci_github(result: &CiRunResult) -> String {
    let mut lines = Vec::new();
    for file in result.files.iter().filter(|file| !file.selected_gate_ok) {
        for gate in &file.failing_gates {
            let Some(stage) = file.report.stages.iter().find(|stage| stage.name == *gate) else {
                continue;
            };
            match gate.as_str() {
                "complexity" => {
                    let violations = stage
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.get("violations"))
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for violation in violations {
                        let function = violation
                            .get("function")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown");
                        let line = violation
                            .get("line")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(1);
                        let complexity = violation
                            .get("complexity")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        let threshold = violation
                            .get("threshold")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0);
                        lines.push(format!(
                            "::error file={},line={}::{}",
                            file.file,
                            line,
                            github_escape(&format!(
                                "{} exceeded complexity threshold {} with {}",
                                function, threshold, complexity
                            ))
                        ));
                    }
                }
                "execute" => {
                    let failures = stage
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.get("fuzz_failures"))
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if failures.is_empty() {
                        lines.push(format!(
                            "::error file={}::{}",
                            file.file,
                            github_escape(&ci_stage_brief(stage))
                        ));
                    }
                    for failure in failures {
                        let function = failure
                            .get("function")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown");
                        let message = failure
                            .get("message")
                            .and_then(|value| value.as_str())
                            .unwrap_or("execution failure");
                        lines.push(format!(
                            "::error file={}::{}",
                            file.file,
                            github_escape(&format!("{}: {}", function, message))
                        ));
                    }
                }
                "portability" => {
                    let reason = stage
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.get("reason"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("Node portability failed");
                    let imports = stage
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.get("failing_imports"))
                        .and_then(|value| value.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let suffix = if imports.is_empty() {
                        String::new()
                    } else {
                        format!(" ({imports})")
                    };
                    lines.push(format!(
                        "::error file={}::{}",
                        file.file,
                        github_escape(&format!("{reason}{suffix}"))
                    ));
                }
                _ => {
                    lines.push(format!(
                        "::error file={}::{}",
                        file.file,
                        github_escape(&ci_stage_brief(stage))
                    ));
                }
            }
        }
    }
    lines.push(format!(
        "court-jester ci: {} ({} checked file(s), gates: {})",
        if result.overall_ok { "PASS" } else { "FAIL" },
        result.checked_files,
        result.gates.join(", ")
    ));
    lines.join("\n")
}

fn ci_json_value(result: &CiRunResult, report_level: ReportLevel) -> serde_json::Value {
    serde_json::json!({
        "base": result.base,
        "head": result.head,
        "gates": result.gates,
        "overall_ok": result.overall_ok,
        "changed_files": result.changed_files,
        "checked_files": result.checked_files,
        "skipped_files": result.skipped_files,
        "files": result.files.iter().map(|file| CiJsonFileResult {
            file: file.file.clone(),
            language: ci_language_name(&file.language).to_string(),
            selected_gate_ok: file.selected_gate_ok,
            failing_gates: file.failing_gates.clone(),
            report: tools::verify::report_json_value(&file.report, report_level),
        }).collect::<Vec<_>>(),
    })
}

async fn run_subcommand(cmd: &str, rest: &[String]) -> Result<(), String> {
    let args = parse_flags(rest)?;

    match cmd {
        "ci" => {
            let repo_dir = env::current_dir()
                .map_err(|e| format!("failed to resolve current directory for ci: {}", e))?;
            let result = run_ci_for_repo(&repo_dir, &args).await?;
            match args.ci_report_format {
                CiReportFormat::Human => {
                    println!("{}", render_ci_human(&result));
                }
                CiReportFormat::Github => {
                    println!("{}", render_ci_github(&result));
                }
                CiReportFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&ci_json_value(&result, args.report_level))
                            .map_err(|e| format!("failed to serialize ci report: {}", e))?
                    );
                }
            }
            if !result.overall_ok {
                std::process::exit(1);
            }
            Ok(())
        }
        "verify" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let complexity_threshold = resolve_complexity_threshold(&args)?;
            let project_dir = args
                .project_dir
                .clone()
                .or_else(|| detect_project_dir(&file));
            let test_code = read_optional_file(args.test_file.as_deref())?;
            let suppressions = read_optional_file(args.suppressions_file.as_deref())?;
            if let Some(raw) = suppressions.as_deref() {
                serde_json::from_str::<serde_json::Value>(raw).map_err(|e| {
                    format!(
                        "invalid suppressions file '{}': {}",
                        args.suppressions_file.as_deref().unwrap_or("<inline>"),
                        e
                    )
                })?;
            }
            let diff = read_optional_file(args.diff_file.as_deref())?;
            let opts = tools::verify::VerifyOptions {
                test_code: test_code.as_deref(),
                test_source_file: args.test_file.as_deref(),
                test_runner: args.test_runner,
                complexity_threshold,
                complexity_metric: args.complexity_metric,
                project_dir: project_dir.as_deref(),
                lint_config_path: args.config_path.as_deref(),
                lint_virtual_file_path: args.virtual_file_path.as_deref(),
                diff: diff.as_deref(),
                suppressions: suppressions.as_deref(),
                suppression_source: args.suppressions_file.as_deref(),
                auto_seed: !args.no_auto_seed,
                source_file: Some(file.as_str()),
                output_dir: args.output_dir.as_deref(),
                report_level: args.report_level,
                execute_gate: args.execute_gate,
                tests_only: args.tests_only,
            };
            let report = tools::verify::verify(&code, &language, opts).await;
            match args.summary_format {
                SummaryFormat::Json => {
                    let json = serde_json::to_string_pretty(&tools::verify::report_json_value(
                        &report,
                        args.report_level,
                    ))
                    .map_err(|e| format!("failed to serialize verify report: {}", e))?;
                    println!("{}", json);
                }
                SummaryFormat::Human => {
                    println!("{}", tools::verify::report_human_summary(&report));
                }
            }
            if !report.overall_ok {
                std::process::exit(1);
            }
            Ok(())
        }
        "analyze" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let complexity_threshold = resolve_complexity_threshold(&args)?;
            let analysis = tools::analyze::analyze(&code, &language);
            let mut value = serde_json::to_value(&analysis)
                .map_err(|e| format!("failed to serialize analysis: {}", e))?;
            if let Some(diff) = read_optional_file(args.diff_file.as_deref())? {
                let changed_ranges = tools::diff::parse_changed_lines_for_file(&diff, &file);
                let changed_fns =
                    tools::analyze::filter_changed_functions(&analysis, &changed_ranges);
                value["changed_functions"] = serde_json::to_value(&changed_fns).unwrap();
            }
            if let Some(threshold) = complexity_threshold {
                let violations =
                    tools::analyze::check_complexity_threshold_for_functions_with_metric(
                        &analysis.functions,
                        threshold,
                        args.complexity_metric,
                    );
                let (active_violations, suppressed_violations): (Vec<_>, Vec<_>) =
                    violations.into_iter().partition(|violation| {
                        !tools::analyze::source_directive_suppresses_complexity(
                            &code,
                            &language,
                            violation.line,
                        )
                    });
                value["complexity_violations"] = serde_json::to_value(&active_violations).unwrap();
                value["suppressed_complexity_violations"] =
                    serde_json::to_value(&suppressed_violations).unwrap();
                value["complexity_ok"] = serde_json::Value::Bool(active_violations.is_empty());
                value["complexity_metric"] = serde_json::to_value(args.complexity_metric).unwrap();
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value)
                    .map_err(|e| format!("failed to serialize analysis: {}", e))?
            );
            Ok(())
        }
        "lint" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let project_dir = args
                .project_dir
                .clone()
                .or_else(|| detect_project_dir(&file));
            let result = tools::lint::lint_with_options(
                &code,
                &language,
                tools::lint::LintOptions {
                    source_file: Some(file.as_str()),
                    project_dir: project_dir.as_deref(),
                    config_path: args.config_path.as_deref(),
                    virtual_file_path: args.virtual_file_path.as_deref(),
                },
            )
            .await;
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| format!("failed to serialize lint result: {}", e))?;
            println!("{}", json);
            if result.error.is_some() {
                std::process::exit(1);
            }
            Ok(())
        }
        "execute" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let project_dir = args
                .project_dir
                .clone()
                .or_else(|| detect_project_dir(&file));
            let timeout = args.timeout_seconds.unwrap_or(10.0);
            let memory = args.memory_mb.unwrap_or(128);
            let result = tools::sandbox::execute(
                &code,
                &language,
                timeout,
                memory,
                project_dir.as_deref(),
                Some(file.as_str()),
            )
            .await;
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| format!("failed to serialize execute result: {}", e))?;
            println!("{}", json);
            if result.exit_code != Some(0) || result.timed_out || result.memory_error {
                std::process::exit(1);
            }
            Ok(())
        }
        _ => unreachable!("unhandled subcommand '{}'", cmd),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_ci_gates, parse_flags, resolve_complexity_threshold, run_ci_for_repo};
    use court_jester_mcp::types::{
        ComplexityMetric, ExecuteGate, ReportLevel, SummaryFormat, TestRunner,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "Court Jester")
            .env("GIT_AUTHOR_EMAIL", "court-jester@example.com")
            .env("GIT_COMMITTER_NAME", "Court Jester")
            .env("GIT_COMMITTER_EMAIL", "court-jester@example.com")
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn security_profile_maps_to_complexity_threshold_20() {
        let args = parse_flags(&["--profile".into(), "security".into()]).unwrap();
        assert_eq!(resolve_complexity_threshold(&args).unwrap(), Some(20));
    }

    #[test]
    fn explicit_threshold_overrides_profile() {
        let args = parse_flags(&[
            "--profile".into(),
            "security".into(),
            "--complexity-threshold".into(),
            "12".into(),
        ])
        .unwrap();
        assert_eq!(resolve_complexity_threshold(&args).unwrap(), Some(12));
    }

    #[test]
    fn report_level_and_execute_gate_parse() {
        let args = parse_flags(&[
            "--report-level".into(),
            "minimal".into(),
            "--summary".into(),
            "human".into(),
            "--complexity-metric".into(),
            "cognitive".into(),
            "--execute-gate".into(),
            "crash".into(),
        ])
        .unwrap();
        assert_eq!(args.report_level, ReportLevel::Minimal);
        assert_eq!(args.summary_format, SummaryFormat::Human);
        assert_eq!(args.complexity_metric, ComplexityMetric::Cognitive);
        assert_eq!(args.execute_gate, ExecuteGate::Crash);
    }

    #[test]
    fn fused_flag_error_suggests_split_arguments() {
        let error = parse_flags(&["--diff-file /tmp/example.diff".into()]).unwrap_err();
        assert!(error.contains("did you mean '--diff-file' and '/tmp/example.diff'"));
    }

    #[test]
    fn fused_config_flag_error_suggests_split_arguments() {
        let error = parse_flags(&["--config-path biome.json".into()]).unwrap_err();
        assert!(error.contains("did you mean '--config-path' and 'biome.json'"));
    }

    #[test]
    fn no_auto_seed_flag_parses() {
        let args = parse_flags(&["--no-auto-seed".into()]).unwrap();
        assert!(args.no_auto_seed);
    }

    #[test]
    fn test_runner_flag_parses() {
        let args = parse_flags(&["--test-runner".into(), "bun".into()]).unwrap();
        assert_eq!(args.test_runner, TestRunner::Bun);
    }

    #[test]
    fn ci_report_and_gate_flags_parse() {
        let args = parse_flags(&[
            "--base".into(),
            "origin/main".into(),
            "--head".into(),
            "HEAD".into(),
            "--gate".into(),
            "complexity,portability".into(),
            "--report".into(),
            "github".into(),
        ])
        .unwrap();
        assert_eq!(args.base.as_deref(), Some("origin/main"));
        assert_eq!(args.head.as_deref(), Some("HEAD"));
        assert_eq!(args.gate.as_deref(), Some("complexity,portability"));
        assert_eq!(args.ci_report_format, super::CiReportFormat::Github);
    }

    #[test]
    fn ci_gate_parser_defaults_and_dedupes() {
        assert_eq!(
            parse_ci_gates(None).unwrap(),
            vec![
                "execute".to_string(),
                "lint".to_string(),
                "parse".to_string(),
                "portability".to_string(),
                "test".to_string()
            ]
        );
        assert_eq!(
            parse_ci_gates(Some("execute,parse,execute")).unwrap(),
            vec!["execute".to_string(), "parse".to_string()]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn ci_fails_on_changed_file_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init"]);
        fs::write(repo.join("sample.py"), "def ok():\n    return 1\n").unwrap();
        git(repo, &["add", "sample.py"]);
        git(repo, &["commit", "-m", "initial"]);

        fs::write(repo.join("sample.py"), "def broken(:\n    pass\n").unwrap();
        git(repo, &["add", "sample.py"]);
        git(repo, &["commit", "-m", "break syntax"]);

        let args = parse_flags(&[
            "--base".into(),
            "HEAD~1".into(),
            "--report".into(),
            "json".into(),
        ])
        .unwrap();
        let result = run_ci_for_repo(repo, &args).await.unwrap();

        assert!(!result.overall_ok);
        assert_eq!(result.changed_files, 1);
        assert_eq!(result.checked_files, 1);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].file, "sample.py");
        assert_eq!(result.files[0].failing_gates, vec!["parse".to_string()]);
    }
}
