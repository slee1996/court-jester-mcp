//! CLI command dispatch; parsing, CI, and readiness live in dedicated modules.

use court_jester::parse_language;
use court_jester::tools;
use court_jester::types::{
    Language, RuntimeProfile, SummaryFormat, VerificationVerdict, DEFAULT_PYTHON_DOCKER_IMAGE,
    DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use std::env;
use std::path::Path;
use std::process::ExitCode;
mod args;
mod candidate;
mod ci;
mod config;
mod doctor;
mod environment;
mod revisions;
use args::{
    parse_flags, parse_replay_flags, read_file, read_optional_file, require_file, require_language,
    resolve_cli_context, resolve_complexity_threshold, validate_base_pair,
    validate_harness_args_in_context, validate_policy_flags, validate_runtime_flags,
    validate_test_quality_flag, CiReportFormat,
};
use ci::{ci_json_value, render_ci_github, render_ci_human, run_ci_for_repo, verdict_label};
use doctor::run_doctor;
use environment::{VerifyLlmPlateauEnv, VerifyNativeFuzzEnv, VerifyTimeoutEnv};

const USAGE: &str = "\
court-jester — code verification CLI for Python and TypeScript

  court-jester verify   [OPTIONS]   Verify a file and print a JSON report
  court-jester ci       [OPTIONS]   Verify changed files for PR/CI workflows
  court-jester analyze  [OPTIONS]   Run tree-sitter analysis
  court-jester lint     [OPTIONS]   Run Ruff or Biome
  court-jester execute  [OPTIONS]   Run code in the sandbox
  court-jester replay   [OPTIONS]   Replay a persisted finding
  court-jester doctor   [OPTIONS]   Check project runtime or isolated image readiness
  court-jester --help               Print this help
  court-jester --version            Print the version

COMMON OPTIONS:
  --file <PATH>              Source file (required for all subcommands)
  --language <LANG>          python | typescript (doctor: all; required otherwise)
  --project-dir <PATH>       venv / node_modules root (auto-detected if omitted)
  --config-path <PATH>       Explicit Ruff/Biome config path for lint + verify
  --repo-config <PATH>       Repository defaults for verify/ci/doctor (CLI overrides)
  --no-repo-config           Disable automatic .court-jester.json discovery
  --show-config             Print selected verify/ci/doctor settings without execution
  --virtual-file-path <PATH> Virtual lint path for temp or generated source files
  --runtime-profile <PROFILE> local-trusted | isolated (default local-trusted)
  --python-docker-image <IMAGE> Python isolated image (default python:3.12-slim)
  --typescript-docker-image <IMAGE> TypeScript isolated image (default node:24-bookworm-slim)

VERIFY OPTIONS:
  --test-file <PATH>         Authoritative test file (exactly one with --test-quality)
  --test-runner <MODE>       auto | node | bun | repo-native (default auto)
  --tests-only               Skip fuzz-execute and run only the authoritative test stage
  --test-quality [N]         Run up to N validated behavior mutants (default 8; range 1..32)
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
  --native-fuzz-engine <E>   off | auto | atheris | jazzer (default off)
  --native-fuzz-runs <N>     Coverage-guided engine iteration cap (default 1000)
  --llm-plateau-command <P> Executable implementing the JSON seed-proposal protocol

CI OPTIONS:
  --base <REV>               Base revision for changed-file diffing (required for `ci`)
  --head <REV>               Head revision for changed-file diffing (default HEAD)
  --candidate-state <STATE>  working-tree (default) | committed (requires --output-dir)
  --gate <LIST>              Comma-separated stage gates or all (default parse,lint,coverage,portability,execute,test)
  --report <FORMAT>          human | github | json (default human)
  --timeout-seconds <F>      Stage timeout override
  --memory-mb <N>            Memory cap MB (default 512)
  --network <POLICY>         deny | allow (isolated requires deny)
  --harness-args-json <JSON> Ordered arguments (only one changed target)
  --test-file <PATH>         Authoritative test entrypoint (repeat once per target language)
  --test-runner <MODE>       auto | node | bun | repo-native (default auto)
  --test-quality [N]         Run up to N mutants globally across changed files (default 8)

DOCTOR OPTIONS:
  --file <PATH>             Source context; requires one explicit language
  --project-dir <PATH>      Project root for local or isolated readiness
  --timeout-seconds <F>     Per readiness probe budget (default 10)
  --memory-mb <N>           Memory cap MB (smoke default 128; entrypoint default 512)
  --show-config             Inspect settings without running readiness probes
  --probe-entrypoint        Opt in to running one selected test entrypoint against --file

EXECUTE OPTIONS:
  --timeout-seconds <F>      Sandbox timeout (default 10)
  --memory-mb <N>            Sandbox memory cap MB (default 128)
REPLAY OPTIONS:
  --report <PATH>            Persisted schema-v3 report to replay
  --finding <ID>             Finding id to replay (must be unique)
  --export-regression <DIR>  Write new CLI-backed test bundle inside dependency project
  --candidate-project-dir <DIR>  Differential replay against current project sources
  --accept-inferred          Explicitly accept an inferred expectation for export
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
  COURT_JESTER_NATIVE_FUZZ_ENGINE                 off | auto | atheris | jazzer
  COURT_JESTER_LLM_PLATEAU_COMMAND                Opt-in seed-proposal executable
  COURT_JESTER_NATIVE_FUZZ_RUNS                   Coverage-guided iteration cap

EXAMPLES:
  court-jester verify --file src/profile.py --language python
  court-jester verify --file src/semver.ts --language typescript \\
      --test-file tests/semver.test.ts --output-dir .court-jester/reports
  court-jester ci --base origin/main --gate complexity,portability --report github
  court-jester lint --file src/parser.py --language python --config-path pyproject.toml
";

pub(super) async fn run() -> ExitCode {
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

async fn run_subcommand(cmd: &str, rest: &[String]) -> Result<(), String> {
    let mut candidate = if cmd == "replay" {
        None
    } else {
        candidate::prepare(cmd, rest, &parse_flags(rest)?)?
    };
    let rest = candidate
        .as_ref()
        .map(|candidate| candidate.flags.as_slice())
        .unwrap_or(rest);
    let mut args = if cmd == "replay" {
        parse_replay_flags(rest)?
    } else {
        config::apply(cmd, rest, parse_flags(rest)?)?
    };
    validate_test_quality_flag(cmd, &args)?;
    validate_runtime_flags(cmd, &args)?;
    validate_policy_flags(cmd, &args)?;
    if let Some(candidate) = &candidate {
        candidate.validate(&args)?;
        args.candidate_root = Some(candidate.root.to_string_lossy().into_owned());
    }
    if args.show_config {
        if !matches!(cmd, "verify" | "ci" | "doctor") {
            return Err("--show-config supports verify, ci, and doctor".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&config::selected_settings(cmd, &args))
                .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if cmd == "replay" {
        if args.accept_inferred && args.regression_output.is_none() {
            return Err("--accept-inferred requires --export-regression".into());
        }
        let report_path = args
            .report_path
            .as_deref()
            .ok_or_else(|| "--report is required for `court-jester replay`".to_string())?;
        let finding_id = args
            .finding_id
            .as_deref()
            .ok_or_else(|| "--finding is required for `court-jester replay`".to_string())?;
        let persisted_context = tools::verify::replay_launch_context(report_path, finding_id)?;
        let export_plan = args
            .regression_output
            .as_deref()
            .map(|output| {
                let project = args
                    .dependency_project_dir
                    .as_deref()
                    .ok_or("--export-regression requires --dependency-project-dir")?;
                tools::verify::prepare_regression_export(
                    report_path,
                    finding_id,
                    project,
                    output,
                    args.accept_inferred,
                )
            })
            .transpose()?;
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
        let report = tools::verify::replay_report_with_candidate_options(
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
            args.replay_candidate_project_dir.as_deref(),
        )
        .await?;
        if let Some(plan) = export_plan {
            let mut launch = persisted_context
                .clone()
                .ok_or("regression export requires launch context")?;
            launch.limits.runtime_profile = runtime_profile;
            if let Some(timeout) = args.timeout_seconds {
                launch.limits.timeout_seconds = timeout;
            }
            if let Some(memory) = args.memory_mb {
                launch.limits.memory_mb = memory;
            }
            if args.network_explicit {
                launch.limits.network_policy = args.network;
            }
            if args.harness_args_explicit {
                launch.harness_args = args.harness_args.clone();
            }
            launch.docker_image =
                (runtime_profile == RuntimeProfile::Isolated).then(|| match replay_language {
                    Language::Python => python_docker_image.to_string(),
                    Language::TypeScript => typescript_docker_image.to_string(),
                });
            let exported = tools::verify::write_regression_export(plan, &report, launch)?;
            let mut value = serde_json::to_value(&report).map_err(|error| error.to_string())?;
            value["regression_export"] = exported;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
            );
            return Ok(());
        }
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
            let _verify_native_fuzz_env =
                VerifyNativeFuzzEnv::install(args.native_fuzz_engine, args.native_fuzz_runs);
            let _verify_llm_plateau_env =
                VerifyLlmPlateauEnv::install(args.llm_plateau_command.as_deref());
            let result = run_ci_for_repo(&repo_dir, &args).await?;
            if let Some(candidate) = &mut candidate {
                candidate.persist();
            }
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
            if args.test_files.len() > 1 {
                return Err("`court-jester verify` accepts exactly one --test-file".into());
            }
            if args.test_quality_max_mutants.is_some() && args.test_files.len() != 1 {
                return Err("--test-quality requires exactly one authoritative --test-file".into());
            }
            let test_file = args.test_files.first().map(String::as_str);
            let file = require_file(&args)?.to_string();
            let language = require_language(&args)?;
            let code = read_file(&file)?;
            let context = resolve_cli_context(&args, language, &file)?;
            validate_harness_args_in_context(&args.harness_args, &context)?;
            let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
            let complexity_threshold = resolve_complexity_threshold(&args)?;
            let test_code = read_optional_file(test_file)?;
            let suppressions = args::read_suppressions(args.suppressions_file.as_deref())?;
            let diff = read_optional_file(args.diff_file.as_deref())?;
            let base_pair = validate_base_pair(&args, &file, &language)?;
            let base_code = base_pair
                .as_ref()
                .map(|(path, _)| read_file(path))
                .transpose()?;
            let opts = tools::verify::VerifyOptions {
                test_code: test_code.as_deref(),
                test_source_file: test_file,
                test_runner: args.test_runner,
                test_quality_max_mutants: args.test_quality_max_mutants,
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
                memory_mb: args.verification_memory_mb(),
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
            let _verify_native_fuzz_env =
                VerifyNativeFuzzEnv::install(args.native_fuzz_engine, args.native_fuzz_runs);
            let _verify_llm_plateau_env =
                VerifyLlmPlateauEnv::install(args.llm_plateau_command.as_deref());
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
                    let summary = tools::verify::repair_summary(&report, &language);
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
                instrumentation_target: None,
                instrumented_source: None,
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
    use super::args::{
        parse_flags, parse_replay_flags, resolve_complexity_threshold, validate_test_quality_flag,
    };
    use super::ci::{
        ci_quality_allocations, ci_test_entrypoints, ci_test_quality_summary, parse_ci_gates,
        render_ci_github, render_ci_human, run_ci_for_repo, CiRunResult, CiTestQualitySummary,
    };
    use court_jester::tools::verify::TestQualitySummary;
    use court_jester::types::{
        ComplexityMetric, ExecuteGate, NativeFuzzEngine, ReportLevel, StageStatus, SummaryFormat,
        TestRunner, VerificationVerdict,
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
    fn native_fuzz_flags_parse_and_validate_bounds() {
        let args = parse_flags(&[
            "--native-fuzz-engine".into(),
            "atheris".into(),
            "--native-fuzz-runs".into(),
            "2500".into(),
        ])
        .unwrap();
        assert_eq!(args.native_fuzz_engine, NativeFuzzEngine::Atheris);
        assert_eq!(args.native_fuzz_runs, Some(2500));

        let error = parse_flags(&["--native-fuzz-runs".into(), "0".into()]).unwrap_err();
        assert!(error.contains("between 1 and 1000000"));
    }

    #[test]
    fn test_runner_flag_parses() {
        let args = parse_flags(&["--test-runner".into(), "bun".into()]).unwrap();
        assert_eq!(args.test_runner, TestRunner::Bun);
    }
    #[test]
    fn stable_test_quality_flag_parses_and_validates_bounds() {
        let default = parse_flags(&["--test-quality".into()]).unwrap();
        assert_eq!(default.test_quality_max_mutants, Some(8));

        let explicit = parse_flags(&[
            "--test-quality".into(),
            "3".into(),
            "--test-file".into(),
            "test_target.py".into(),
            "--test-file".into(),
            "target.test.ts".into(),
        ])
        .unwrap();
        assert_eq!(explicit.test_quality_max_mutants, Some(3));
        assert_eq!(
            explicit.test_files,
            vec!["test_target.py".to_string(), "target.test.ts".to_string()]
        );

        let maximum = parse_flags(&["--test-quality".into(), "32".into()]).unwrap();
        assert_eq!(maximum.test_quality_max_mutants, Some(32));

        let above_maximum = parse_flags(&["--test-quality".into(), "33".into()]).unwrap_err();
        assert!(above_maximum.contains("between 1 and 32"));
        let error = parse_flags(&["--test-quality".into(), "0".into()]).unwrap_err();
        assert!(error.contains("between 1 and 32"));
    }

    #[test]
    fn test_quality_flag_is_limited_to_verify_and_ci() {
        let args = parse_flags(&["--test-quality".into()]).unwrap();
        validate_test_quality_flag("verify", &args).unwrap();
        validate_test_quality_flag("ci", &args).unwrap();
        for command in ["doctor", "analyze", "lint", "execute"] {
            let error = validate_test_quality_flag(command, &args).unwrap_err();
            assert_eq!(
                error,
                format!("--test-quality is not supported for `{command}`")
            );
        }
        let unsupported = parse_flags(&[
            "--test-quality".into(),
            "1".into(),
            "--test-runner".into(),
            "repo-native".into(),
        ])
        .unwrap();
        let error = validate_test_quality_flag("verify", &unsupported).unwrap_err();
        assert_eq!(
            error,
            "--test-quality does not support --test-runner repo-native; use auto, node, or bun"
        );

        assert!(parse_flags(&["--experimental-test-quality".into()]).is_err());
        assert!(parse_replay_flags(&["--experimental-test-quality".into()]).is_err());
        assert!(parse_replay_flags(&["--test-quality".into()]).is_err());
    }

    #[test]
    fn ci_test_entrypoints_are_language_keyed_and_order_independent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("quality.py"), "PYTHON").unwrap();
        fs::write(dir.path().join("quality.tsx"), "TYPESCRIPT").unwrap();

        for paths in [
            vec!["quality.py".to_string(), "quality.tsx".to_string()],
            vec!["quality.tsx".to_string(), "quality.py".to_string()],
        ] {
            let entrypoints = ci_test_entrypoints(dir.path(), &paths).unwrap();
            assert_eq!(entrypoints.python.as_ref().unwrap().code, "PYTHON");
            assert_eq!(entrypoints.typescript.as_ref().unwrap().code, "TYPESCRIPT");
        }

        let error = ci_test_entrypoints(
            dir.path(),
            &["first.py".to_string(), "second.py".to_string()],
        )
        .unwrap_err();
        assert_eq!(
            error,
            "`ci --test-quality` accepts at most one python --test-file; got 'first.py' and 'second.py'"
        );
        let error = ci_test_entrypoints(
            dir.path(),
            &["first.ts".to_string(), "second.tsx".to_string()],
        )
        .unwrap_err();
        assert_eq!(
            error,
            "`ci --test-quality` accepts at most one typescript --test-file; got 'first.ts' and 'second.tsx'"
        );
    }

    #[test]
    fn ci_quality_allocation_redistributes_without_exceeding_global_cap() {
        assert_eq!(ci_quality_allocations(5, &[1, 0, 10]), vec![1, 0, 4]);
        assert_eq!(ci_quality_allocations(5, &[10, 1, 10]), vec![2, 1, 2]);
        assert_eq!(ci_quality_allocations(8, &[2, 1]), vec![2, 1]);
        assert_eq!(ci_quality_allocations(3, &[10, 10, 10]), vec![1, 1, 1]);
        assert_eq!(ci_quality_allocations(0, &[10, 10]), vec![0, 0]);
        assert!(
            ci_quality_allocations(5, &[1, 0, 10])
                .into_iter()
                .sum::<usize>()
                <= 5
        );
    }

    #[test]
    fn ci_test_quality_summary_preserves_outcome_buckets_and_unjudged_math() {
        let summary = ci_test_quality_summary(
            8,
            [
                TestQualitySummary {
                    planned: 4,
                    killed: 1,
                    survived: 0,
                    invalid: 1,
                    blocked: 1,
                    no_coverage: 1,
                    unjudged: 999,
                    coupling: 2,
                },
                TestQualitySummary {
                    planned: 2,
                    killed: 1,
                    survived: 1,
                    invalid: 0,
                    blocked: 0,
                    no_coverage: 0,
                    unjudged: 999,
                    coupling: 0,
                },
            ],
        );
        assert_eq!(summary.counts.invalid, 1);
        assert_eq!(summary.counts.blocked, 1);
        assert_eq!(summary.counts.no_coverage, 1);
        assert_eq!(summary.counts.unjudged, 3);

        let json = serde_json::to_value(summary).unwrap();
        assert_eq!(json["invalid"], 1);
        assert_eq!(json["blocked"], 1);
        assert_eq!(json["no_coverage"], 1);
        assert_eq!(json["unjudged"], 3);
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
    fn ci_human_and_github_quality_summaries_are_advisory_counts_only() {
        let result = CiRunResult {
            base: "base".into(),
            head: "head".into(),
            base_commit: "base-commit".into(),
            head_commit: "head-commit".into(),
            candidate_state: super::args::CandidateState::WorkingTree,
            candidate_workspace: None,
            gates: vec!["test".into()],
            changed_files: 0,
            checked_files: 0,
            skipped_files: Vec::new(),
            files: Vec::new(),
            test_quality: Some(CiTestQualitySummary {
                max_mutants: 9,
                counts: TestQualitySummary {
                    planned: 9,
                    killed: 2,
                    survived: 3,
                    invalid: 1,
                    blocked: 1,
                    no_coverage: 2,
                    unjudged: 4,
                    coupling: 5,
                },
            }),
            verdict: VerificationVerdict::Pass,
        };

        for output in [render_ci_human(&result), render_ci_github(&result)] {
            for expected in [
                "planned=9",
                "killed=2",
                "survived=3",
                "unjudged=4",
                "coupling=5",
            ] {
                assert!(
                    output.contains(expected),
                    "missing '{expected}' in {output}"
                );
            }
            let normalized = output.to_ascii_lowercase();
            for forbidden in ["score", "percentage", "grade", "%"] {
                assert!(
                    !normalized.contains(forbidden),
                    "unexpected '{forbidden}' in {output}"
                );
            }
        }
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
