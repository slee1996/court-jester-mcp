use std::collections::BTreeSet;
use std::env;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, ExitCode};

use tar::Archive;
use tempfile::TempDir;

use court_jester::types::{
    ComplexityMetric, CoverageGate, DiagnosticImpact, ExecuteGate, FailureDomain, HarnessArg,
    InferredOracleGate, Language, NetworkPolicy, ReportLevel, RuntimeProfile, StageStatus,
    SummaryFormat, TestRunner, VerificationReport, VerificationVerdict,
    DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use court_jester::{parse_language, tools};

const USAGE: &str = "\
court-jester — code verification CLI for Python and TypeScript

  court-jester verify   [OPTIONS]   Verify a file and print a JSON report
  court-jester ci       [OPTIONS]   Verify changed files for PR/CI workflows
  court-jester analyze  [OPTIONS]   Run tree-sitter analysis
  court-jester lint     [OPTIONS]   Run Ruff or Biome
  court-jester execute  [OPTIONS]   Run code in the sandbox
  court-jester replay   [OPTIONS]   Replay a persisted finding
  court-jester doctor   [OPTIONS]   Check runtime and sandbox readiness
  court-jester --help               Print this help
  court-jester --version            Print the version

COMMON OPTIONS:
  --file <PATH>              Source file (required for all subcommands)
  --language <LANG>          python | typescript (doctor: all; required otherwise)
  --project-dir <PATH>       venv / node_modules root (auto-detected if omitted)
  --config-path <PATH>       Explicit Ruff/Biome config path for lint + verify
  --virtual-file-path <PATH> Virtual lint path for temp or generated source files
  --runtime-profile <PROFILE> local-trusted | isolated (default local-trusted)
  --python-docker-image <IMAGE> Python isolated image (default python:3.12-slim)
  --typescript-docker-image <IMAGE> TypeScript isolated image (default node:24-bookworm-slim)

VERIFY OPTIONS:
  --test-file <PATH>         Test file to include as an authoritative stage
  --test-runner <MODE>       auto | node | bun | repo-native (default auto)
  --tests-only               Skip fuzz-execute and run only the authoritative test stage
  --output-dir <PATH>        Directory to write persistent JSON reports
  --base-file <PATH>         Candidate's base source file for differential verification
  --base-project-dir <PATH>  Read-only project root for the base source tree
  --report-level <LEVEL>     full | minimal (default full)
  --summary <FORMAT>         json | human | repair-json (default json)
  --profile <NAME>           Verification profile preset (currently: security => complexity 20)
  --complexity-metric <NAME> cyclomatic | cognitive (default cyclomatic)
  --complexity-threshold <N> Fail if any function exceeds this complexity (changed functions only when --diff-file is set)
  --execute-gate <MODE>      all | crash | none (default all; no_inputs_reached is always diagnostic)
  --coverage-gate <MODE>     changed-exports | none (default changed-exports)
  --inferred-oracle-gate <MODE> advisory | fail (default advisory)
  --timeout-seconds <F>      Fuzz/test timeout override
  --memory-mb <N>            Memory cap MB (default 512)
  --network <POLICY>         deny | allow (isolated requires deny)
  --harness-args-json <JSON> Ordered literal/project_path argument array

CI OPTIONS:
  --base <REV>               Base revision for changed-file diffing (required for `ci`)
  --head <REV>               Head revision for changed-file diffing (default HEAD)
  --gate <LIST>              Comma-separated stage gates or all (default parse,lint,coverage,portability,execute,test)
  --report <FORMAT>          human | github | json (default human)
  --timeout-seconds <F>      Stage timeout override
  --memory-mb <N>            Memory cap MB (default 512)
  --network <POLICY>         deny | allow (isolated requires deny)
  --harness-args-json <JSON> Ordered arguments (only one changed target)

EXECUTE OPTIONS:
  --timeout-seconds <F>      Sandbox timeout (default 10)
  --memory-mb <N>            Sandbox memory cap MB (default 128)
REPLAY OPTIONS:
  --report <PATH>            Persisted schema-v3 report to replay
  --finding <ID>             Finding id to replay (must be unique)
  --dependency-project-dir <PATH>  Dependency project root for replay
  --timeout-seconds <F>      Replay timeout override
  --memory-mb <N>            Replay memory cap override
  --network <POLICY>         Replay network policy override
  --harness-args-json <JSON> Replay ordered argument override
DOCTOR OPTIONS:
  --language <LANG>          python | typescript | all (default all)
  --summary <FORMAT>         json | human (default json)

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
const CI_DEFAULT_GATES: [&str; 6] = [
    "parse",
    "lint",
    "coverage",
    "portability",
    "execute",
    "test",
];

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
        "doctor" | "verify" | "ci" | "analyze" | "lint" | "execute" | "replay" => {
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
    base_file: Option<String>,
    base_project_dir: Option<String>,
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
    coverage_gate: CoverageGate,
    inferred_oracle_gate: InferredOracleGate,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
    network: NetworkPolicy,
    network_explicit: bool,
    harness_args: Vec<HarnessArg>,
    harness_args_explicit: bool,
    runtime_profile: RuntimeProfile,
    runtime_profile_explicit: bool,
    python_docker_image: Option<String>,
    typescript_docker_image: Option<String>,
    report_path: Option<String>,
    finding_id: Option<String>,
    dependency_project_dir: Option<String>,
}
/// Apply the CLI verification timeout to every verification stage without
/// expanding the public `VerifyOptions` literal compatibility surface.
struct VerifyTimeoutEnv {
    previous: Vec<(String, Option<std::ffi::OsString>)>,
}

impl VerifyTimeoutEnv {
    fn install(timeout_seconds: Option<f64>) -> Self {
        let mut previous = Vec::new();
        if let Some(timeout) = timeout_seconds {
            for key in [
                "COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS",
                "COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS",
                "COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS",
            ] {
                previous.push((key.to_string(), env::var_os(key)));
                env::set_var(key, timeout.to_string());
            }
        }
        Self { previous }
    }
}

impl Drop for VerifyTimeoutEnv {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CiReportFormat {
    #[default]
    Human,
    Github,
    Json,
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
            "--runtime-profile" => {
                let raw = take_value(&mut i)?;
                out.runtime_profile = RuntimeProfile::parse(&raw).ok_or_else(|| {
                    format!(
                        "--runtime-profile must be one of: local-trusted, isolated (got '{}')",
                        raw
                    )
                })?;
                out.runtime_profile_explicit = true;
            }
            "--python-docker-image" => out.python_docker_image = Some(take_value(&mut i)?),
            "--typescript-docker-image" => out.typescript_docker_image = Some(take_value(&mut i)?),
            "--base-file" => out.base_file = Some(take_value(&mut i)?),
            "--base-project-dir" => out.base_project_dir = Some(take_value(&mut i)?),
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
                    format!(
                        "--summary must be one of: json, human, repair-json (got '{}')",
                        raw
                    )
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
            "--coverage-gate" => {
                let raw = take_value(&mut i)?;
                out.coverage_gate = CoverageGate::parse(&raw).ok_or_else(|| {
                    format!(
                        "--coverage-gate must be one of: changed-exports, none (got '{}')",
                        raw
                    )
                })?;
            }
            "--inferred-oracle-gate" => {
                let raw = take_value(&mut i)?;
                out.inferred_oracle_gate = InferredOracleGate::parse(&raw).ok_or_else(|| {
                    format!(
                        "--inferred-oracle-gate must be one of: advisory, fail (got '{}')",
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
            "--network" => {
                let raw = take_value(&mut i)?;
                out.network = match raw.as_str() {
                    "deny" => NetworkPolicy::Deny,
                    "allow" => NetworkPolicy::Allow,
                    _ => return Err("--network must be one of: deny, allow".into()),
                };
                out.network_explicit = true;
            }
            "--harness-args-json" => {
                let raw = take_value(&mut i)?;
                out.harness_args = parse_harness_args(&raw)?;
                out.harness_args_explicit = true;
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

fn parse_harness_args(raw: &str) -> Result<Vec<HarnessArg>, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|error| {
        format!("--harness-args-json must be an ordered JSON array of harness arguments: {error}")
    })?;
    let items = value.as_array().ok_or_else(|| {
        "--harness-args-json must be an ordered JSON array of harness arguments".to_string()
    })?;
    let mut arguments = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            format!("harness argument {index} must be an object with exactly one key")
        })?;
        if object.len() != 1 {
            return Err(format!(
                "harness argument {index} must contain exactly one of 'literal' or 'project_path'"
            ));
        }
        if let Some(value) = object.get("literal") {
            let literal = value
                .as_str()
                .ok_or_else(|| format!("harness argument {index}.literal must be a string"))?;
            if literal.contains('\0') {
                return Err(format!("harness argument {index}.literal contains NUL"));
            }
            arguments.push(HarnessArg::Literal {
                literal: literal.to_string(),
            });
            continue;
        }
        if let Some(value) = object.get("project_path") {
            let project_path = value
                .as_str()
                .ok_or_else(|| format!("harness argument {index}.project_path must be a string"))?;
            let path = Path::new(project_path);
            if project_path.contains('\0') || path.is_absolute() {
                return Err(format!(
                    "harness argument {index}.project_path must be a relative path without NUL"
                ));
            }
            if path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "harness argument {index}.project_path must not escape the project root"
                ));
            }
            arguments.push(HarnessArg::ProjectPath {
                project_path: project_path.to_string(),
            });
            continue;
        }
        return Err(format!(
            "harness argument {index} must contain exactly one of 'literal' or 'project_path'"
        ));
    }
    Ok(arguments)
}
fn parse_replay_flags(rest: &[String]) -> Result<CliArgs, String> {
    let mut out = CliArgs::default();
    let mut i = 0;
    while i < rest.len() {
        let flag = rest[i].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            if *index + 1 >= rest.len() {
                return Err(format!("flag {flag} requires a value"));
            }
            *index += 1;
            Ok(rest[*index].clone())
        };
        match flag {
            "--report" => out.report_path = Some(value(&mut i)?),
            "--finding" => out.finding_id = Some(value(&mut i)?),
            "--dependency-project-dir" => out.dependency_project_dir = Some(value(&mut i)?),
            "--runtime-profile" => {
                out.runtime_profile = RuntimeProfile::parse(&value(&mut i)?).ok_or_else(|| {
                    "--runtime-profile must be one of: local-trusted, isolated".to_string()
                })?;
                out.runtime_profile_explicit = true;
            }
            "--python-docker-image" => out.python_docker_image = Some(value(&mut i)?),
            "--typescript-docker-image" => out.typescript_docker_image = Some(value(&mut i)?),
            "--timeout-seconds" => {
                out.timeout_seconds = Some(
                    value(&mut i)?
                        .parse::<f64>()
                        .map_err(|_| "--timeout-seconds must be a number".to_string())?,
                );
            }
            "--memory-mb" => {
                out.memory_mb = Some(
                    value(&mut i)?
                        .parse::<u64>()
                        .map_err(|_| "--memory-mb must be a positive integer".to_string())?,
                );
            }
            "--network" => {
                out.network = match value(&mut i)?.as_str() {
                    "deny" => NetworkPolicy::Deny,
                    "allow" => NetworkPolicy::Allow,
                    _ => return Err("--network must be one of: deny, allow".into()),
                };
                out.network_explicit = true;
            }
            "--harness-args-json" => {
                out.harness_args = parse_harness_args(&value(&mut i)?)?;
                out.harness_args_explicit = true;
            }
            other => return Err(format!("unknown replay flag '{other}'")),
        }
        i += 1;
    }
    Ok(out)
}

fn validate_runtime_flags(cmd: &str, args: &CliArgs) -> Result<(), String> {
    let runtime_cmd = matches!(cmd, "verify" | "execute" | "ci" | "doctor" | "replay");
    if !runtime_cmd
        && (args.runtime_profile != RuntimeProfile::LocalTrusted
            || args.python_docker_image.is_some()
            || args.typescript_docker_image.is_some())
    {
        return Err(format!(
            "runtime profile and docker image flags are not supported for `{cmd}`"
        ));
    }
    if args.runtime_profile == RuntimeProfile::LocalTrusted
        && (args.python_docker_image.is_some() || args.typescript_docker_image.is_some())
    {
        return Err("docker image overrides are valid only with --runtime-profile isolated".into());
    }
    if let Some(timeout) = args.timeout_seconds {
        if !timeout.is_finite() || timeout <= 0.0 {
            return Err("--timeout-seconds must be finite and greater than zero".into());
        }
        if !matches!(cmd, "verify" | "execute" | "ci" | "replay") {
            return Err(format!("--timeout-seconds is not supported for `{cmd}`"));
        }
    }
    if let Some(memory) = args.memory_mb {
        if memory == 0 {
            return Err("--memory-mb must be greater than zero".into());
        }
        if memory
            .checked_mul(1024)
            .and_then(|bytes| bytes.checked_mul(1024))
            .is_none()
        {
            return Err("--memory-mb is too large".into());
        }
    }
    if matches!(cmd, "verify" | "execute") {
        if let Some(raw) = args.language.as_deref() {
            let language = parse_language(raw).map_err(|_| "invalid --language".to_string())?;
            if matches!(language, Language::Python) && args.typescript_docker_image.is_some() {
                return Err("--typescript-docker-image is not valid for a Python command".into());
            }
            if matches!(language, Language::TypeScript) && args.python_docker_image.is_some() {
                return Err("--python-docker-image is not valid for a TypeScript command".into());
            }
        }
    }
    for image in [
        args.python_docker_image.as_deref(),
        args.typescript_docker_image.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if image.trim().is_empty() || image.starts_with('-') {
            return Err("docker image must be non-empty and must not begin with '-'".into());
        }
    }
    Ok(())
}
fn resolve_cli_context(
    args: &CliArgs,
    language: Language,
    target_file: &str,
) -> Result<court_jester::types::ExecutionContext, String> {
    let invocation_dir =
        env::current_dir().map_err(|error| format!("cannot resolve current directory: {error}"))?;
    court_jester::resolve_execution_context(court_jester::types::ContextRequest {
        invocation_dir: &invocation_dir,
        explicit_project_dir: args.project_dir.as_deref().map(Path::new),
        target_file: Some(Path::new(target_file)),
        test_file: args.test_file.as_deref().map(Path::new),
        language,
        virtual_file_path: args.virtual_file_path.as_deref().map(Path::new),
    })
    .map_err(|error| error.to_string())
}

fn validate_harness_args_in_context(
    harness_args: &[HarnessArg],
    context: &court_jester::types::ExecutionContext,
) -> Result<(), String> {
    for argument in harness_args {
        let HarnessArg::ProjectPath { project_path } = argument else {
            continue;
        };
        let candidate = context.workspace_root.join(project_path);
        let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "harness project path '{}' is unavailable under '{}': {}",
                project_path,
                context.workspace_root.display(),
                error
            )
        })?;
        if !canonical.starts_with(&context.workspace_root) {
            return Err(format!(
                "harness project path '{}' escapes project root '{}'",
                project_path,
                context.workspace_root.display()
            ));
        }
    }
    Ok(())
}

fn validate_policy_flags(cmd: &str, args: &CliArgs) -> Result<(), String> {
    let unsupported = matches!(cmd, "analyze" | "lint" | "doctor");
    if unsupported
        && (args.memory_mb.is_some() || args.network_explicit || args.harness_args_explicit)
    {
        return Err(format!(
            "memory, network, and harness-args flags are not supported for `{cmd}`"
        ));
    }
    if cmd == "execute" && (args.network_explicit || args.harness_args_explicit) {
        return Err("`execute` does not accept --network or --harness-args-json".into());
    }
    if args.runtime_profile == RuntimeProfile::Isolated && args.network == NetworkPolicy::Allow {
        return Err("isolated execution requires --network deny".into());
    }
    Ok(())
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

fn validate_base_pair(
    args: &CliArgs,
    candidate_file: &str,
    language: &Language,
) -> Result<Option<(String, String)>, String> {
    if args.base_file.is_some() != args.base_project_dir.is_some() {
        return Err("--base-file and --base-project-dir must be supplied together".into());
    }
    let Some(base_file) = args.base_file.as_deref() else {
        return Ok(None);
    };
    let base_root = args.base_project_dir.as_deref().unwrap_or("");
    let candidate_rel = Path::new(candidate_file)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(candidate_file);
    let base_rel = Path::new(base_file)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(base_file);
    if candidate_rel != base_rel {
        return Err("--base-file must have the same relative filename as --file".into());
    }
    let extension_language = |path: &str| {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".d.ts") {
            return Some(Language::TypeScript);
        }
        match Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref()
        {
            Some("py") => Some(Language::Python),
            Some("ts") | Some("tsx") | Some("jsx") => Some(Language::TypeScript),
            _ => None,
        }
    };
    let base_language = extension_language(base_file);
    if base_language.as_ref() != Some(language) {
        return Err("--base-file language does not match --language".into());
    }
    if !Path::new(base_root).is_dir() {
        return Err("--base-project-dir must be an existing directory".into());
    }
    Ok(Some((base_file.to_string(), base_root.to_string())))
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
    verdict: VerificationVerdict,
    failing_gates: Vec<String>,
    report: VerificationReport,
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
    verdict: VerificationVerdict,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CiJsonFileResult {
    file: String,
    language: String,
    verdict: VerificationVerdict,
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

fn stage_verdict(status: &StageStatus) -> VerificationVerdict {
    match status {
        StageStatus::Failed => VerificationVerdict::Fail,
        StageStatus::Inconclusive | StageStatus::Skipped => VerificationVerdict::Inconclusive,
        StageStatus::Passed | StageStatus::Advisory => VerificationVerdict::Pass,
    }
}

fn aggregate_verdict(
    current: VerificationVerdict,
    next: VerificationVerdict,
) -> VerificationVerdict {
    match (current, next) {
        (VerificationVerdict::Fail, _) | (_, VerificationVerdict::Fail) => {
            VerificationVerdict::Fail
        }
        (VerificationVerdict::Inconclusive, _) | (_, VerificationVerdict::Inconclusive) => {
            VerificationVerdict::Inconclusive
        }
        _ => VerificationVerdict::Pass,
    }
}

fn ci_stage_verdict(stage: &court_jester::types::VerificationStage) -> VerificationVerdict {
    let diagnostics = tools::verify::stage_diagnostics(stage);
    if diagnostics.iter().any(|diagnostic| {
        diagnostic.domain == FailureDomain::TargetCode
            && diagnostic.impact == DiagnosticImpact::Gating
    }) {
        VerificationVerdict::Fail
    } else if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.impact == DiagnosticImpact::Blocking)
    {
        VerificationVerdict::Inconclusive
    } else {
        stage_verdict(&stage.status)
    }
}

fn ci_selected_verdict(report: &VerificationReport, gates: &[String]) -> VerificationVerdict {
    let selected: BTreeSet<&str> = gates.iter().map(String::as_str).collect();
    let mut verdict = VerificationVerdict::Pass;
    let mut found = false;
    for stage in &report.stages {
        if selected.contains(stage.name.as_str()) {
            found = true;
            verdict = aggregate_verdict(verdict, ci_stage_verdict(stage));
        }
    }
    if found {
        verdict
    } else {
        VerificationVerdict::Inconclusive
    }
}

fn ci_stage_failures(report: &VerificationReport, gates: &[String]) -> Vec<String> {
    let selected: BTreeSet<&str> = gates.iter().map(String::as_str).collect();
    report
        .stages
        .iter()
        .filter(|stage| {
            selected.contains(stage.name.as_str())
                && ci_stage_verdict(stage) != VerificationVerdict::Pass
        })
        .map(|stage| stage.name.clone())
        .collect()
}

fn ci_language_for_path(path: &str) -> Option<Language> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".d.ts") {
        return None;
    }
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("py") => Some(Language::Python),
        Some("ts") | Some("tsx") | Some("jsx") => Some(Language::TypeScript),
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

fn archive_baseline_tree(repo_dir: &Path, revision: &str) -> Result<TempDir, String> {
    let output = Command::new("git")
        .current_dir(repo_dir)
        .args(["archive", "--format=tar", revision])
        .output()
        .map_err(|e| format!("failed to run git archive: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git archive failed for {revision}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let temp =
        tempfile::tempdir().map_err(|e| format!("failed to create baseline tempdir: {e}"))?;
    Archive::new(Cursor::new(output.stdout))
        .unpack(temp.path())
        .map_err(|e| format!("failed to unpack baseline archive: {e}"))?;
    Ok(temp)
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
    let baseline_temp = archive_baseline_tree(repo_dir, &base)?;
    let gates = parse_ci_gates(args.gate.as_deref())?;
    let changed_files = ci_changed_source_files(repo_dir, &base, &head)?;
    if args.harness_args_explicit && changed_files.len() != 1 {
        return Err(
            "`ci` accepts --harness-args-json only when exactly one changed target is selected"
                .into(),
        );
    }
    let diff = if changed_files.is_empty() {
        String::new()
    } else {
        ci_unified_diff(repo_dir, &base, &head)?
    };
    let complexity_threshold = resolve_complexity_threshold(args)?;
    let mut files = Vec::new();
    let mut skipped_files = Vec::new();
    let mut verdict = VerificationVerdict::Pass;

    for (relative_path, language) in &changed_files {
        let baseline_path = baseline_temp.path().join(relative_path);
        let baseline_code = baseline_path
            .is_file()
            .then(|| read_file(&baseline_path.to_string_lossy()))
            .transpose()?;
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
            .or_else(|| Some(repo_dir.to_string_lossy().into_owned()));
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
                base_code: baseline_code.as_deref(),
                base_source_file: baseline_path.to_str(),
                base_project_dir: baseline_temp.path().to_str(),
                source_file: Some(absolute_string.as_str()),
                output_dir: args.output_dir.as_deref(),
                report_level: args.report_level,
                execute_gate: args.execute_gate,
                coverage_gate: args.coverage_gate,
                inferred_oracle_gate: args.inferred_oracle_gate,
                runtime_profile: args.runtime_profile,
                memory_mb: args.memory_mb.unwrap_or(512),
                network: args.network,
                harness_args: args.harness_args.clone(),
                python_docker_image: args
                    .python_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE),
                typescript_docker_image: args
                    .typescript_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE),
                tests_only: false,
            },
        )
        .await;
        let failing_gates = ci_stage_failures(&report, &gates);
        let file_verdict = ci_selected_verdict(&report, &gates);
        verdict = aggregate_verdict(verdict, file_verdict);
        files.push(CiFileResult {
            file: relative_path.clone(),
            language: *language,
            verdict: file_verdict,
            failing_gates,
            report,
        });
    }

    if !skipped_files.is_empty() {
        verdict = VerificationVerdict::Inconclusive;
    }
    Ok(CiRunResult {
        base,
        head,
        gates,
        changed_files: changed_files.len(),
        checked_files: files.len(),
        skipped_files,
        files,
        verdict,
    })
}

fn ci_stage_brief(stage: &court_jester::types::VerificationStage) -> String {
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
            let findings = stage
                .detail
                .as_ref()
                .and_then(|detail| detail.get("findings"))
                .and_then(|value| value.as_array());
            let count_severity = |wanted: &str| {
                findings
                    .map(|items| {
                        items
                            .iter()
                            .filter(|finding| {
                                finding.get("severity").and_then(|value| value.as_str())
                                    == Some(wanted)
                            })
                            .count()
                    })
                    .unwrap_or(0)
            };
            format!(
                "{} crash(es), {} property violation(s)",
                count_severity("crash"),
                count_severity("property_violation")
            )
        }
        "test" | "parse" | "portability" | "lint" | "coverage" => stage
            .message
            .clone()
            .unwrap_or_else(|| "stage did not complete".into()),
        _ => stage
            .message
            .clone()
            .unwrap_or_else(|| "stage did not complete".into()),
    }
}

fn verdict_label(verdict: &VerificationVerdict) -> &'static str {
    match verdict {
        VerificationVerdict::Pass => "PASS",
        VerificationVerdict::Fail => "FAIL",
        VerificationVerdict::Inconclusive => "INCONCLUSIVE",
    }
}

fn render_ci_human(result: &CiRunResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("CI: {}\n", verdict_label(&result.verdict)));
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
    if result
        .files
        .iter()
        .all(|file| matches!(file.verdict, VerificationVerdict::Pass))
    {
        return out;
    }
    out.push_str("\nFailing Files:\n");
    for file in result
        .files
        .iter()
        .filter(|file| !matches!(file.verdict, VerificationVerdict::Pass))
    {
        out.push_str(&format!(
            "- {} [{}]\n",
            file.file,
            file.failing_gates.join(", ")
        ));
        for gate in &file.failing_gates {
            if let Some(stage) = file.report.stages.iter().find(|stage| stage.name == *gate) {
                out.push_str(&format!("  {}: {}\n", gate, ci_stage_brief(stage)));
                for diagnostic in tools::verify::stage_diagnostics(stage) {
                    out.push_str(&format!(
                        "    - [{:?}/{:?}, {:?}] {}\n",
                        diagnostic.domain, diagnostic.kind, diagnostic.impact, diagnostic.message
                    ));
                }
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
    for file in result
        .files
        .iter()
        .filter(|file| !matches!(file.verdict, VerificationVerdict::Pass))
    {
        for gate in &file.failing_gates {
            let Some(stage) = file.report.stages.iter().find(|stage| stage.name == *gate) else {
                continue;
            };
            let diagnostics = tools::verify::stage_diagnostics(stage);
            let confirmed_target_failure = diagnostics.iter().any(|diagnostic| {
                diagnostic.domain == FailureDomain::TargetCode
                    && diagnostic.impact == DiagnosticImpact::Gating
            });
            let blocking_non_target: Vec<_> = diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.impact == DiagnosticImpact::Blocking
                        && !(diagnostic.domain == FailureDomain::TargetCode
                            && diagnostic.impact == DiagnosticImpact::Gating)
                })
                .collect();
            for diagnostic in &blocking_non_target {
                lines.push(format!(
                    "::warning file={}::{}",
                    file.file,
                    github_escape(&format!(
                        "{:?}/{:?}: {}",
                        diagnostic.domain, diagnostic.kind, diagnostic.message
                    ))
                ));
            }
            if !confirmed_target_failure && !blocking_non_target.is_empty() {
                continue;
            }
            if !confirmed_target_failure && diagnostics.is_empty() {
                lines.push(format!(
                    "::warning file={}::{}",
                    file.file,
                    github_escape(&ci_stage_brief(stage))
                ));
                continue;
            }
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
                    let findings = stage
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.get("findings"))
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if findings.is_empty() {
                        lines.push(format!(
                            "::error file={}::{}",
                            file.file,
                            github_escape(&ci_stage_brief(stage))
                        ));
                    }
                    for finding in findings {
                        let function = finding
                            .pointer("/location/function")
                            .and_then(|value| value.as_str())
                            .or_else(|| finding.get("function").and_then(|value| value.as_str()))
                            .unwrap_or("unknown");
                        let message = finding
                            .get("message")
                            .and_then(|value| value.as_str())
                            .unwrap_or("execution finding");
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
        verdict_label(&result.verdict),
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
        "verdict": result.verdict,
        "changed_files": result.changed_files,
        "checked_files": result.checked_files,
        "skipped_files": result.skipped_files,
        "files": result.files.iter().map(|file| CiJsonFileResult {
            file: file.file.clone(),
            language: ci_language_name(&file.language).to_string(),
            verdict: file.verdict,
            failing_gates: file.failing_gates.clone(),
            report: tools::verify::report_json_value(&file.report, report_level),
        }).collect::<Vec<_>>(),
    })
}

fn exit_for_verdict(verdict: &VerificationVerdict) {
    let code = match verdict {
        VerificationVerdict::Pass => 0,
        VerificationVerdict::Fail => 1,
        VerificationVerdict::Inconclusive => 3,
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn command_version(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program} unavailable: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn doctor_check(
    name: &str,
    language: Option<Language>,
    status: StageStatus,
    detail: serde_json::Value,
    message: Option<String>,
) -> court_jester::types::DoctorCheck {
    court_jester::types::DoctorCheck {
        name: name.into(),
        language,
        status,
        detail,
        message,
    }
}

async fn run_doctor(args: &CliArgs) -> Result<court_jester::types::DoctorReport, String> {
    if args.file.is_some() || args.project_dir.is_some() {
        return Err("doctor does not accept --file or --project-dir".into());
    }
    let selected = match args
        .language
        .as_deref()
        .unwrap_or("all")
        .to_ascii_lowercase()
        .as_str()
    {
        "all" => vec![Language::Python, Language::TypeScript],
        "python" | "py" => vec![Language::Python],
        "typescript" | "ts" => vec![Language::TypeScript],
        other => {
            return Err(format!(
                "--language for doctor must be python, typescript, or all (got '{other}')"
            ))
        }
    };
    let mut checks = Vec::new();
    if args.runtime_profile == RuntimeProfile::LocalTrusted {
        for language in &selected {
            let (program, version_args) = match language {
                Language::Python => ("python3", vec!["--version"]),
                Language::TypeScript => ("node", vec!["--version"]),
            };
            match command_version(program, &version_args) {
                Ok(version) => {
                    let node_bad = matches!(language, Language::TypeScript)
                        && version
                            .split('.')
                            .next()
                            .and_then(|v| v.trim_start_matches('v').parse::<u32>().ok())
                            .is_some_and(|v| v < 24);
                    checks.push(doctor_check(
                        "runtime",
                        Some(*language),
                        if node_bad {
                            StageStatus::Failed
                        } else {
                            StageStatus::Passed
                        },
                        serde_json::json!({"version": version}),
                        node_bad.then(|| "Node.js >=24 is required".into()),
                    ));
                }
                Err(error) => checks.push(doctor_check(
                    "runtime",
                    Some(*language),
                    StageStatus::Failed,
                    serde_json::Value::Null,
                    Some(error),
                )),
            }
            let linter = match language {
                Language::Python => "ruff",
                Language::TypeScript => "biome",
            };
            let status = if Command::new(linter).arg("--version").output().is_ok() {
                StageStatus::Passed
            } else {
                StageStatus::Advisory
            };
            checks.push(doctor_check(
                "linter",
                Some(*language),
                status,
                serde_json::json!({"program": linter}),
                (status == StageStatus::Advisory)
                    .then(|| format!("optional linter {linter} is unavailable")),
            ));
        }
    } else {
        match tools::sandbox::docker_daemon_ready().await {
            Ok(()) => checks.push(doctor_check(
                "docker_daemon",
                None,
                StageStatus::Passed,
                serde_json::json!({"network": "none", "read_only": true}),
                None,
            )),
            Err(error) => checks.push(doctor_check(
                "docker_daemon",
                None,
                StageStatus::Failed,
                serde_json::Value::Null,
                Some(error),
            )),
        }
        for language in &selected {
            let image = match language {
                Language::Python => args
                    .python_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE),
                Language::TypeScript => args
                    .typescript_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE),
            };
            match tools::sandbox::docker_image_id(image).await {
                Ok(id) => checks.push(doctor_check(
                    "docker_image",
                    Some(*language),
                    StageStatus::Passed,
                    serde_json::json!({"image": image, "id": id}),
                    None,
                )),
                Err(error) => checks.push(doctor_check(
                    "docker_image",
                    Some(*language),
                    StageStatus::Failed,
                    serde_json::json!({"image": image}),
                    Some(error),
                )),
            }
            let code = match language {
                Language::Python => "print('court-jester doctor')",
                Language::TypeScript => "console.log(process.versions.node)",
            };
            let project = TempDir::new()
                .map_err(|error| format!("failed to create doctor workspace: {error}"))?;
            let project_path = project.path().to_path_buf();
            let source_mode = match language {
                Language::Python => court_jester::types::SourceMode::Python,
                Language::TypeScript => court_jester::types::SourceMode::TypeScript,
            };
            let context = court_jester::types::ExecutionContext {
                invocation_dir: project_path.clone(),
                workspace_root: project_path.clone(),
                target_package_root: project_path.clone(),
                test_package_root: None,
                dependency_roots: Vec::new(),
                target_source: court_jester::types::SourceContext {
                    language: *language,
                    mode: source_mode,
                    source_file: None,
                    virtual_file_path: None,
                },
                test_source: None,
            };
            let project_dir_owned = project_path.to_string_lossy().into_owned();
            let options = court_jester::types::SandboxOptions {
                timeout_seconds: 10.0,
                memory_mb: 128,
                runtime_profile: RuntimeProfile::Isolated,
                network_policy: NetworkPolicy::Deny,
                harness_args: &[],
                docker_image: Some(image),
                project_dir: Some(project_dir_owned.as_str()),
                source_file: None,
            };
            let runtime = match source_mode {
                court_jester::types::SourceMode::Python => {
                    court_jester::types::HarnessRuntime::Python
                }
                court_jester::types::SourceMode::TypeScript => {
                    court_jester::types::HarnessRuntime::NodeScript
                }
                court_jester::types::SourceMode::Tsx => {
                    court_jester::types::HarnessRuntime::TsxScript
                }
            };
            let result = tools::sandbox::execute_harness(
                &context,
                court_jester::types::HarnessSpec {
                    kind: court_jester::types::HarnessKind::Standalone,
                    runtime,
                    test_adapter: None,
                    source_mode,
                    artifact: court_jester::types::HarnessArtifact::Generated {
                        code: code.to_string(),
                        relative_path: std::path::PathBuf::from(format!(
                            ".court-jester/doctor.{extension}",
                            extension =
                                if matches!(source_mode, court_jester::types::SourceMode::Python) {
                                    "py"
                                } else {
                                    "ts"
                                }
                        )),
                    },
                    args: Vec::new(),
                    network: NetworkPolicy::Deny,
                },
                options,
            )
            .await
            .process;
            let smoke_ok = result.exit_code == Some(0) && !result.timed_out && !result.memory_error;
            let node_bad = matches!(language, Language::TypeScript)
                && result
                    .stdout
                    .trim()
                    .split('.')
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some_and(|v| v < 24);
            checks.push(doctor_check("runtime_smoke", Some(*language), if smoke_ok && !node_bad { StageStatus::Passed } else { StageStatus::Failed }, serde_json::json!({"image": image, "stdout": result.stdout, "stderr": result.stderr, "network": "none", "read_only": true, "memory_mb": 128}), (!smoke_ok).then(|| "isolated runtime smoke failed".into()).or_else(|| node_bad.then(|| "Node.js >=24 is required".into()))));
        }
    }
    let verdict = if checks
        .iter()
        .any(|check| check.status == StageStatus::Failed)
    {
        VerificationVerdict::Fail
    } else {
        VerificationVerdict::Pass
    };
    Ok(court_jester::types::DoctorReport {
        schema_version: court_jester::types::REPORT_SCHEMA_VERSION,
        verdict,
        runtime_profile: args.runtime_profile,
        checks,
    })
}

async fn run_subcommand(cmd: &str, rest: &[String]) -> Result<(), String> {
    let args = if cmd == "replay" {
        parse_replay_flags(rest)?
    } else {
        parse_flags(rest)?
    };
    validate_runtime_flags(cmd, &args)?;
    validate_policy_flags(cmd, &args)?;
    if cmd == "replay" {
        let report_path = args
            .report_path
            .as_deref()
            .ok_or_else(|| "--report is required for `court-jester replay`".to_string())?;
        let finding_id = args
            .finding_id
            .as_deref()
            .ok_or_else(|| "--finding is required for `court-jester replay`".to_string())?;
        let persisted_context = tools::verify::replay_launch_context(report_path, finding_id)?;
        let persisted_report = tools::verify::load_persisted_report(report_path)?;
        let replay_language = parse_language(&persisted_report.meta.language).map_err(|_| {
            format!(
                "unsupported report language '{}'; expected python or typescript",
                persisted_report.meta.language
            )
        })?;
        let runtime_profile = if args.runtime_profile_explicit {
            args.runtime_profile
        } else {
            persisted_context
                .as_ref()
                .map(|context| context.limits.runtime_profile)
                .unwrap_or(args.runtime_profile)
        };
        let persisted_image = persisted_context
            .as_ref()
            .and_then(|context| context.docker_image.as_deref());
        let python_docker_image = args
            .python_docker_image
            .as_deref()
            .or_else(|| {
                matches!(replay_language, Language::Python)
                    .then_some(persisted_image)
                    .flatten()
            })
            .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE);
        let typescript_docker_image = args
            .typescript_docker_image
            .as_deref()
            .or_else(|| {
                matches!(replay_language, Language::TypeScript)
                    .then_some(persisted_image)
                    .flatten()
            })
            .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE);
        let report = tools::verify::replay_report_with_options(
            report_path,
            finding_id,
            args.dependency_project_dir.as_deref(),
            runtime_profile,
            python_docker_image,
            typescript_docker_image,
            args.timeout_seconds,
            args.memory_mb,
            args.network_explicit.then_some(args.network),
            args.harness_args_explicit
                .then_some(args.harness_args.as_slice()),
        )
        .await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("failed to serialize replay report: {error}"))?
        );
        match report.outcome {
            court_jester::types::ReplayOutcome::Reproduced => {}
            court_jester::types::ReplayOutcome::NotReproduced => std::process::exit(1),
            court_jester::types::ReplayOutcome::Inconclusive => std::process::exit(3),
        }
        return Ok(());
    }
    if cmd == "doctor" {
        let report = run_doctor(&args).await?;
        match args.summary_format {
            SummaryFormat::Json => println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .map_err(|e| format!("failed to serialize doctor report: {e}"))?
            ),
            SummaryFormat::Human => {
                println!(
                    "doctor: {} ({:?})",
                    verdict_label(&report.verdict),
                    report.runtime_profile
                );
                for check in &report.checks {
                    println!(
                        "- {}: {:?}{}",
                        check.name,
                        check.status,
                        check
                            .message
                            .as_deref()
                            .map(|m| format!(": {m}"))
                            .unwrap_or_default()
                    );
                }
            }
            SummaryFormat::RepairJson => println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .map_err(|e| format!("failed to serialize doctor report: {e}"))?
            ),
        }
        if report.verdict == VerificationVerdict::Fail {
            std::process::exit(1);
        }
        return Ok(());
    }

    match cmd {
        "ci" => {
            let repo_dir = env::current_dir()
                .map_err(|e| format!("failed to resolve current directory for ci: {}", e))?;
            let _verify_timeout_env = VerifyTimeoutEnv::install(args.timeout_seconds);
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
            exit_for_verdict(&result.verdict);
            Ok(())
        }
        "verify" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let context = resolve_cli_context(&args, language, &file)?;
            validate_harness_args_in_context(&args.harness_args, &context)?;
            let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
            let complexity_threshold = resolve_complexity_threshold(&args)?;
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
            let base_pair = validate_base_pair(&args, &file, &language)?;
            let base_code = base_pair
                .as_ref()
                .map(|(path, _)| read_file(path))
                .transpose()?;
            let opts = tools::verify::VerifyOptions {
                test_code: test_code.as_deref(),
                test_source_file: args.test_file.as_deref(),
                test_runner: args.test_runner,
                complexity_threshold,
                complexity_metric: args.complexity_metric,
                project_dir: Some(project_dir_owned.as_str()),
                lint_config_path: args.config_path.as_deref(),
                lint_virtual_file_path: args.virtual_file_path.as_deref(),
                diff: diff.as_deref(),
                suppressions: suppressions.as_deref(),
                suppression_source: args.suppressions_file.as_deref(),
                auto_seed: !args.no_auto_seed,
                source_file: Some(file.as_str()),
                base_code: base_code.as_deref(),
                base_source_file: base_pair.as_ref().map(|(path, _)| path.as_str()),
                base_project_dir: base_pair.as_ref().map(|(_, root)| root.as_str()),
                output_dir: args.output_dir.as_deref(),
                report_level: args.report_level,
                execute_gate: args.execute_gate,
                coverage_gate: args.coverage_gate,
                inferred_oracle_gate: args.inferred_oracle_gate,
                runtime_profile: args.runtime_profile,
                memory_mb: args.memory_mb.unwrap_or(512),
                network: args.network,
                harness_args: args.harness_args.clone(),
                python_docker_image: args
                    .python_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE),
                typescript_docker_image: args
                    .typescript_docker_image
                    .as_deref()
                    .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE),
                tests_only: args.tests_only,
            };
            let _verify_timeout_env = VerifyTimeoutEnv::install(args.timeout_seconds);
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
                SummaryFormat::RepairJson => {
                    let summary = tools::verify::repair_summary(&report);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summary).map_err(|error| format!(
                            "failed to serialize repair summary: {error}"
                        ))?
                    );
                }
            }
            exit_for_verdict(&report.verdict);
            Ok(())
        }
        "analyze" => {
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let complexity_threshold = resolve_complexity_threshold(&args)?;
            let invocation_dir = env::current_dir()
                .map_err(|error| format!("cannot resolve current directory: {error}"))?;
            let context =
                court_jester::resolve_execution_context(court_jester::types::ContextRequest {
                    invocation_dir: &invocation_dir,
                    explicit_project_dir: args.project_dir.as_deref().map(Path::new),
                    target_file: Some(Path::new(&file)),
                    test_file: None,
                    language,
                    virtual_file_path: args.virtual_file_path.as_deref().map(Path::new),
                })
                .map_err(|error| error.to_string())?;
            let analysis = tools::analyze::analyze_with_context(&code, &context.target_source);
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
            let context = resolve_cli_context(&args, language, &file)?;
            let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
            let result = tools::lint::lint_with_options(
                &code,
                &language,
                tools::lint::LintOptions {
                    source_file: Some(file.as_str()),
                    project_dir: Some(project_dir_owned.as_str()),
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
            let context = resolve_cli_context(&args, language, &file)?;
            let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
            let timeout = args.timeout_seconds.unwrap_or(10.0);
            let memory = args.memory_mb.unwrap_or(128);
            let docker_image = if args.runtime_profile == RuntimeProfile::Isolated {
                Some(match language {
                    Language::Python => args
                        .python_docker_image
                        .as_deref()
                        .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE),
                    Language::TypeScript => args
                        .typescript_docker_image
                        .as_deref()
                        .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE),
                })
            } else {
                None
            };
            let options = court_jester::types::SandboxOptions {
                timeout_seconds: timeout,
                memory_mb: memory,
                runtime_profile: args.runtime_profile,
                network_policy: args.network,
                harness_args: args.harness_args.as_slice(),
                docker_image,
                project_dir: Some(project_dir_owned.as_str()),
                source_file: Some(file.as_str()),
            };
            options.validate()?;
            let source_mode = context.target_source.mode;
            let (runtime, extension) = match source_mode {
                court_jester::types::SourceMode::Python => {
                    (court_jester::types::HarnessRuntime::Python, "py")
                }
                court_jester::types::SourceMode::TypeScript => {
                    (court_jester::types::HarnessRuntime::NodeScript, "ts")
                }
                court_jester::types::SourceMode::Tsx => {
                    (court_jester::types::HarnessRuntime::TsxScript, "tsx")
                }
            };
            let result = tools::sandbox::execute_harness(
                &context,
                court_jester::types::HarnessSpec {
                    kind: court_jester::types::HarnessKind::Standalone,
                    runtime,
                    test_adapter: None,
                    source_mode,
                    artifact: court_jester::types::HarnessArtifact::Generated {
                        code,
                        relative_path: std::path::PathBuf::from(format!(
                            ".court-jester/generated/execute.{extension}"
                        )),
                    },
                    args: Vec::new(),
                    network: args.network,
                },
                options,
            )
            .await
            .process;
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
    use court_jester::types::{
        ComplexityMetric, ExecuteGate, ReportLevel, StageStatus, SummaryFormat, TestRunner,
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
                "coverage".to_string(),
                "execute".to_string(),
                "lint".to_string(),
                "parse".to_string(),
                "portability".to_string(),
                "test".to_string(),
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

        assert!(matches!(result.verdict, super::VerificationVerdict::Fail));
        assert_eq!(result.changed_files, 1);
        assert_eq!(result.checked_files, 1);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].file, "sample.py");
        assert_eq!(
            result.files[0].failing_gates,
            vec!["parse".to_string(), "execute".to_string()]
        );
        let execute = result.files[0]
            .report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .expect("parse failure should report skipped execution");
        assert_eq!(execute.status, StageStatus::Skipped);
        assert_eq!(
            execute.detail.as_ref().unwrap()["reason"].as_str(),
            Some("parse_failed")
        );
    }
}
