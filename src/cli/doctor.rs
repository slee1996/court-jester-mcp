//! Runtime and sandbox readiness checks.

use super::args::CliArgs;
use court_jester::tools;
use court_jester::types::{
    Language, NetworkPolicy, RuntimeProfile, StageStatus, VerificationVerdict,
    DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use std::path::Path;

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

async fn local_runtime_check(
    args: &CliArgs,
    language: Language,
    context: &court_jester::types::ExecutionContext,
) -> court_jester::types::DoctorCheck {
    use court_jester::types::*;
    let (runtime, code, extension) = match context.target_source.mode {
        SourceMode::Python => (HarnessRuntime::Python,
            "import json, sys\nprint('__COURT_JESTER_DOCTOR__' + json.dumps({'version': '.'.join(map(str, sys.version_info[:3])), 'executable': sys.executable}))", "py"),
        SourceMode::TypeScript | SourceMode::Tsx => (
            if context.target_source.mode == SourceMode::Tsx { HarnessRuntime::TsxScript } else { HarnessRuntime::NodeScript },
            "const evidence: {version: string, executable: string} = {version: process.versions.node, executable: process.execPath}; console.log('__COURT_JESTER_DOCTOR__' + JSON.stringify(evidence));",
            if context.target_source.mode == SourceMode::Tsx { "tsx" } else { "ts" }),
    };
    let result = tools::sandbox::execute_harness(
        context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime,
            test_adapter: None,
            source_mode: context.target_source.mode,
            artifact: HarnessArtifact::Generated {
                code: code.into(),
                relative_path: format!(".court-jester/doctor.{extension}").into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        SandboxOptions {
            timeout_seconds: args.timeout_seconds.unwrap_or(10.0),
            memory_mb: args.memory_mb.unwrap_or(128),
            runtime_profile: RuntimeProfile::LocalTrusted,
            network_policy: NetworkPolicy::Deny,
            harness_args: &[],
            docker_image: None,
            // Resolve project runtimes, but do not copy or execute the user's sources.
            project_dir: None,
            source_file: None,
            instrumentation_target: None,
            instrumented_source: None,
        },
    )
    .await
    .process;
    let records: Vec<serde_json::Value> = result
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("__COURT_JESTER_DOCTOR__"))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let evidence = (records.len() == 1).then(|| &records[0]);
    let version = evidence
        .and_then(|value| value.get("version"))
        .and_then(|value| value.as_str());
    let executable = evidence
        .and_then(|value| value.get("executable"))
        .and_then(|value| value.as_str());
    let major = version
        .and_then(|value| value.split('.').next())
        .and_then(|value| value.parse::<u32>().ok());
    let version_ok = match language {
        Language::Python => major == Some(3),
        Language::TypeScript => major.is_some_and(|major| major >= 24),
    };
    let passed = result.exit_code == Some(0)
        && !result.timed_out
        && !result.memory_error
        && version_ok
        && executable.is_some_and(|value| !value.is_empty());
    doctor_check("runtime", Some(language), if passed { StageStatus::Passed } else { StageStatus::Failed },
        serde_json::json!({"version": version, "executable": executable,
            "workspace_root": context.workspace_root, "target_package_root": context.target_package_root,
            "source_mode": context.target_source.mode, "execution": result}),
        (!passed).then(|| match language {
            Language::Python => "Project Python smoke failed; repair the selected virtualenv/runtime (Python 3 required). Target imports were not checked.".into(),
            Language::TypeScript => "Project TypeScript smoke failed; install Node.js >=24 and, for TSX, the project tsx runner. Target imports were not checked.".into(),
        }))
}

pub(super) async fn run_doctor(
    args: &CliArgs,
) -> Result<court_jester::types::DoctorReport, String> {
    if args.probe_entrypoint && (args.file.is_none() || args.test_files.len() != 1) {
        return Err("doctor --probe-entrypoint requires --file, one explicit language, and exactly one configured or explicit --test-file".into());
    }
    if args.file.is_some()
        && args
            .language
            .as_deref()
            .is_none_or(|language| language.eq_ignore_ascii_case("all"))
    {
        return Err("doctor --file requires --language python or typescript".into());
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
    if args.file.is_some() || args.project_dir.is_some() {
        let invocation = std::env::current_dir().map_err(|error| error.to_string())?;
        let project = args
            .project_dir
            .as_deref()
            .map(|path| invocation.join(path));
        for language in &selected {
            let context =
                court_jester::resolve_execution_context(court_jester::types::ContextRequest {
                    invocation_dir: &invocation,
                    explicit_project_dir: project.as_deref(),
                    target_file: args.file.as_deref().map(Path::new),
                    test_file: None,
                    language: *language,
                    virtual_file_path: None,
                })
                .map_err(|error| error.to_string())?;
            checks.push(doctor_check(
                "project_context",
                Some(*language),
                StageStatus::Passed,
                serde_json::json!({"workspace_root": context.workspace_root,
                    "target_package_root": context.target_package_root,
                    "source_mode": context.target_source.mode, "executed": false}),
                Some(
                    "Project paths resolved; imports and test behavior require --probe-entrypoint"
                        .into(),
                ),
            ));
        }
    }
    if args.repo_config.is_some() {
        checks.push(doctor_check("repository_config", None, StageStatus::Passed,
            super::config::selected_settings("doctor", args),
            Some("Repository settings resolved; target imports and test behavior have not been checked".into())));
    }
    if !args.test_files.is_empty() {
        let entrypoints = args
            .test_files
            .iter()
            .map(|path| {
                let error = std::fs::metadata(path)
                    .and_then(|metadata| {
                        if metadata.is_file() {
                            std::fs::File::open(path).map(|_| ())
                        } else {
                            Err(std::io::Error::other("not a regular file"))
                        }
                    })
                    .err()
                    .map(|error| error.to_string());
                serde_json::json!({"path": path, "readable": error.is_none(), "error": error})
            })
            .collect::<Vec<_>>();
        let ready = entrypoints.iter().all(|entry| entry["readable"] == true);
        checks.push(doctor_check("configured_entrypoints", None,
            if ready { StageStatus::Passed } else { StageStatus::Failed },
            serde_json::json!({"files": entrypoints, "executed": false}),
            Some(if ready { "Configured test files are readable; imports and test behavior were not executed".into() }
                else { "Restore missing test files or update test_files in the repository config/CLI selection".into() })));
    }
    if args.runtime_profile == RuntimeProfile::LocalTrusted {
        let invocation = std::env::current_dir().map_err(|error| error.to_string())?;
        let explicit_project = args
            .project_dir
            .as_deref()
            .map(|path| invocation.join(path));
        for language in &selected {
            let context =
                court_jester::resolve_execution_context(court_jester::types::ContextRequest {
                    invocation_dir: &invocation,
                    explicit_project_dir: explicit_project.as_deref(),
                    target_file: args.file.as_deref().map(Path::new),
                    test_file: None,
                    language: *language,
                    virtual_file_path: None,
                })
                .map_err(|error| error.to_string())?;
            checks.push(local_runtime_check(args, *language, &context).await);
            let project = explicit_project
                .as_deref()
                .unwrap_or(&context.workspace_root);
            let program = tools::lint::resolve_linter(language, project.to_str());
            let version = match program.as_deref() {
                Some(program) => {
                    tools::lint::probe_linter_version(
                        program,
                        &context.target_package_root,
                        args.timeout_seconds.unwrap_or(10.0),
                    )
                    .await
                }
                None => Err(format!(
                    "optional {:?} linter is unavailable; install it in the project environment",
                    language
                )),
            };
            checks.push(doctor_check(
                "linter",
                Some(*language),
                if version.is_ok() {
                    StageStatus::Passed
                } else {
                    StageStatus::Advisory
                },
                serde_json::json!({"program": program, "version": version.as_ref().ok()}),
                version.err(),
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
            let project = tools::sandbox::runtime_tempdir(RuntimeProfile::Isolated)
                .map_err(|error| format!("failed to create doctor workspace: {error}"))?;
            let project_path = project.path().to_path_buf();
            let source_mode = match language {
                Language::Python => court_jester::types::SourceMode::Python,
                Language::TypeScript => court_jester::types::SourceMode::TypeScript,
            };
            let context = court_jester::types::ExecutionContext {
                invocation_dir: project_path.clone(),
                workspace_root: project_path.clone(),
                materialization_source_root: None,
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
                timeout_seconds: args.timeout_seconds.unwrap_or(10.0),
                memory_mb: args.memory_mb.unwrap_or(128),
                runtime_profile: RuntimeProfile::Isolated,
                network_policy: NetworkPolicy::Deny,
                harness_args: &[],
                docker_image: Some(image),
                project_dir: Some(project_dir_owned.as_str()),
                source_file: None,
                instrumentation_target: None,
                instrumented_source: None,
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
            checks.push(doctor_check("runtime_smoke", Some(*language), if smoke_ok && !node_bad { StageStatus::Passed } else { StageStatus::Failed }, serde_json::json!({"image": image, "stdout": result.stdout, "stderr": result.stderr, "network": "none", "read_only": true, "memory_mb": args.memory_mb.unwrap_or(128)}), (!smoke_ok).then(|| "isolated runtime smoke failed".into()).or_else(|| node_bad.then(|| "Node.js >=24 is required".into()))));
        }
    }
    if args.probe_entrypoint {
        let language = selected[0];
        let source = args.file.as_deref().unwrap();
        let test = &args.test_files[0];
        let code = super::args::read_file(source);
        let tests = super::args::read_file(test);
        let result = match (code, tests) {
            (Ok(code), Ok(tests)) => {
                tools::verify::probe_authoritative_entrypoint(
                    &code,
                    &tests,
                    &language,
                    tools::verify::EntrypointProbeOptions {
                        source_file: source,
                        test_source_file: test,
                        project_dir: args.project_dir.as_deref(),
                        test_runner: args.test_runner,
                        timeout_seconds: args.timeout_seconds.unwrap_or(10.0),
                        memory_mb: args.verification_memory_mb(),
                        runtime_profile: args.runtime_profile,
                        python_docker_image: args
                            .python_docker_image
                            .as_deref()
                            .unwrap_or(DEFAULT_PYTHON_DOCKER_IMAGE),
                        typescript_docker_image: args
                            .typescript_docker_image
                            .as_deref()
                            .unwrap_or(DEFAULT_TYPESCRIPT_DOCKER_IMAGE),
                    },
                )
                .await
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
        };
        let passed = result
            .as_ref()
            .is_ok_and(|stage| stage.status == StageStatus::Passed);
        checks.push(doctor_check("entrypoint_probe", Some(language), if passed { StageStatus::Passed } else { StageStatus::Failed },
            serde_json::json!({"execution_opt_in": true, "source_file": source, "test_file": test,
                "test_stage": result.as_ref().ok(), "error": result.as_ref().err(),
                "timeout_seconds": args.timeout_seconds.unwrap_or(10.0), "memory_mb": args.verification_memory_mb(),
                "coverage_checked": false, "fuzzing_started": false}),
            Some(if passed { "Selected test entrypoint completed; this does not prove complete target coverage or application correctness".into() }
                else { "Selected entrypoint did not complete successfully; inspect test_stage/error, repair imports or dependencies, or fix failing tests, then rerun doctor --probe-entrypoint".into() })));
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
