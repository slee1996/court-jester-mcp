//! Command-line configuration, parsing, and validation.

use super::USAGE;
use court_jester::parse_language;
use court_jester::tools;
use court_jester::types::{
    ComplexityMetric, CoverageGate, ExecuteGate, HarnessArg, InferredOracleGate, Language,
    NativeFuzzEngine, NetworkPolicy, ReportLevel, RuntimeProfile, SummaryFormat, TestRunner,
};
use std::env;
use std::path::Path;

#[derive(Debug, Default)]
pub(super) struct CliArgs {
    pub(super) base_file: Option<String>,
    pub(super) base_project_dir: Option<String>,
    pub(super) file: Option<String>,
    pub(super) language: Option<String>,
    pub(super) base: Option<String>,
    pub(super) head: Option<String>,
    pub(super) gate: Option<String>,
    pub(super) ci_report_format: CiReportFormat,
    pub(super) project_dir: Option<String>,
    pub(super) config_path: Option<String>,
    pub(super) virtual_file_path: Option<String>,
    pub(super) test_files: Vec<String>,
    pub(super) test_runner: TestRunner,
    pub(super) tests_only: bool,
    pub(super) test_quality_max_mutants: Option<usize>,
    pub(super) output_dir: Option<String>,
    pub(super) report_level: ReportLevel,
    pub(super) summary_format: SummaryFormat,
    pub(super) suppressions_file: Option<String>,
    pub(super) no_auto_seed: bool,
    pub(super) native_fuzz_engine: NativeFuzzEngine,
    pub(super) native_fuzz_runs: Option<usize>,
    pub(super) llm_plateau_command: Option<String>,
    pub(super) diff_file: Option<String>,
    pub(super) profile: Option<String>,
    pub(super) complexity_metric: ComplexityMetric,
    pub(super) complexity_threshold: Option<usize>,
    pub(super) execute_gate: ExecuteGate,
    pub(super) coverage_gate: CoverageGate,
    pub(super) inferred_oracle_gate: InferredOracleGate,
    pub(super) timeout_seconds: Option<f64>,
    pub(super) memory_mb: Option<u64>,
    pub(super) network: NetworkPolicy,
    pub(super) network_explicit: bool,
    pub(super) harness_args: Vec<HarnessArg>,
    pub(super) harness_args_explicit: bool,
    pub(super) runtime_profile: RuntimeProfile,
    pub(super) runtime_profile_explicit: bool,
    pub(super) python_docker_image: Option<String>,
    pub(super) typescript_docker_image: Option<String>,
    pub(super) report_path: Option<String>,
    pub(super) finding_id: Option<String>,
    pub(super) dependency_project_dir: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CiReportFormat {
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

pub(super) fn parse_flags(rest: &[String]) -> Result<CliArgs, String> {
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
            "--test-file" => out.test_files.push(take_value(&mut i)?),
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
            "--test-quality" => {
                let max_mutants = rest
                    .get(i + 1)
                    .filter(|value| !value.starts_with("--"))
                    .map(|raw| {
                        let value = raw.parse::<usize>().map_err(|_| {
                            format!(
                                "--test-quality must be an integer from 1 to 32, got '{}'",
                                raw
                            )
                        })?;
                        if !(1..=tools::test_quality::MAX_MUTANTS).contains(&value) {
                            return Err(format!(
                                "--test-quality must be between 1 and 32, got '{}'",
                                raw
                            ));
                        }
                        i += 1;
                        Ok(value)
                    })
                    .transpose()?
                    .unwrap_or(tools::test_quality::DEFAULT_MAX_MUTANTS);
                out.test_quality_max_mutants = Some(max_mutants);
            }
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
            "--native-fuzz-engine" => {
                let raw = take_value(&mut i)?;
                out.native_fuzz_engine = NativeFuzzEngine::parse(&raw).ok_or_else(|| {
                    format!(
                        "--native-fuzz-engine must be one of: off, auto, atheris, jazzer (got '{}')",
                        raw
                    )
                })?;
            }
            "--native-fuzz-runs" => {
                let raw = take_value(&mut i)?;
                let runs = raw.parse::<usize>().map_err(|_| {
                    format!(
                        "--native-fuzz-runs must be a positive integer, got '{}'",
                        raw
                    )
                })?;
                if runs == 0 || runs > 1_000_000 {
                    return Err(format!(
                        "--native-fuzz-runs must be between 1 and 1000000, got '{}'",
                        raw
                    ));
                }
                out.native_fuzz_runs = Some(runs);
            }
            "--llm-plateau-command" => {
                let command = take_value(&mut i)?;
                if command.trim().is_empty() {
                    return Err("--llm-plateau-command must not be empty".into());
                }
                out.llm_plateau_command = Some(command);
            }
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
pub(super) fn parse_replay_flags(rest: &[String]) -> Result<CliArgs, String> {
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

pub(super) fn validate_runtime_flags(cmd: &str, args: &CliArgs) -> Result<(), String> {
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
pub(super) fn resolve_cli_context(
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
        test_file: args.test_files.first().map(String::as_str).map(Path::new),
        language,
        virtual_file_path: args.virtual_file_path.as_deref().map(Path::new),
    })
    .map_err(|error| error.to_string())
}

pub(super) fn validate_harness_args_in_context(
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

pub(super) fn validate_policy_flags(cmd: &str, args: &CliArgs) -> Result<(), String> {
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

pub(super) fn validate_test_quality_flag(cmd: &str, args: &CliArgs) -> Result<(), String> {
    if args.test_quality_max_mutants.is_some() && !matches!(cmd, "verify" | "ci") {
        return Err(format!("--test-quality is not supported for `{cmd}`"));
    }
    if args.test_quality_max_mutants.is_some() && args.test_runner == TestRunner::RepoNative {
        return Err(
            "--test-quality does not support --test-runner repo-native; use auto, node, or bun"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn require_file(args: &CliArgs) -> Result<&str, String> {
    args.file
        .as_deref()
        .ok_or_else(|| "--file is required".to_string())
}

pub(super) fn require_language(args: &CliArgs) -> Result<Language, String> {
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

pub(super) fn require_base(args: &CliArgs) -> Result<&str, String> {
    args.base
        .as_deref()
        .ok_or_else(|| "--base is required for `court-jester ci`".to_string())
}

pub(super) fn validate_base_pair(
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

pub(super) fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {}", path, e))
}

pub(super) fn read_optional_file(path: Option<&str>) -> Result<Option<String>, String> {
    match path {
        Some(path) => Ok(Some(read_file(path)?)),
        None => Ok(None),
    }
}

pub(super) fn resolve_complexity_threshold(args: &CliArgs) -> Result<Option<usize>, String> {
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
