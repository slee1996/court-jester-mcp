//! Runtime and sandbox readiness checks.

use super::args::CliArgs;
use court_jester::tools;
use court_jester::types::{
    Language, NetworkPolicy, RuntimeProfile, StageStatus, VerificationVerdict,
    DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use std::process::Command;

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

pub(super) async fn run_doctor(
    args: &CliArgs,
) -> Result<court_jester::types::DoctorReport, String> {
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
                timeout_seconds: 10.0,
                memory_mb: 128,
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
