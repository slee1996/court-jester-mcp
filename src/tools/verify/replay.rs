//! Loading and replaying persisted verification findings.

use super::{
    compatible_surface, differential_binding_failure, differential_case_from_arguments,
    differential_probe, differential_snapshot, err_execution_result, findings_from_stages,
    generated_target_source, stable_digest, tree_digest,
};
use crate::tools::{analyze, sandbox};
use crate::types::{
    ContextRequest, DifferentialRepro, EmbeddedSource, ExecutionResult, FindingSeverity,
    HarnessArg, Language, NetworkPolicy, OracleKind, PersistedReport, ProcessTermination,
    ProcessTerminationKind, RepairSummary, ReplayOutcome, ReplayReport, ReproLaunchContext,
    RuntimeProfile, SandboxOptions, StageStatus, VerificationFinding, VerificationStage,
    VerificationVerdict, REPORT_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};

fn persisted_findings(report: &PersistedReport) -> Vec<VerificationFinding> {
    findings_from_stages(&report.stages)
}

pub(super) fn render_native_replay(
    arguments: &[crate::types::ReproValue],
    contract: &crate::types::NativeReplayContract,
    language: &Language,
) -> Result<String, String> {
    if contract.schema_version != 1 || contract.body.trim().is_empty() {
        return Err("unsupported or empty native replay binding contract".into());
    }
    if arguments.len() != contract.argument_count {
        return Err("native replay binding contract argument count mismatch".into());
    }
    if arguments
        .iter()
        .any(|argument| argument.expression.trim().is_empty())
    {
        return Err("native replay arguments require executable expressions".into());
    }
    let arguments = arguments
        .iter()
        .map(|argument| argument.expression.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let binding = match language {
        Language::Python => format!("_cj_args = [{arguments}]\n"),
        Language::TypeScript => format!("const _cj_args = [{arguments}];\n"),
    };
    Ok(binding + &contract.body)
}

pub fn load_persisted_report(path: &str) -> Result<PersistedReport, String> {
    let bytes = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read report '{path}': {error}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&bytes).map_err(|error| format!("invalid report JSON: {error}"))?;
    let report = if value.get("meta").is_some() && value.get("stages").is_some() {
        serde_json::from_value::<PersistedReport>(value)
            .map_err(|error| format!("invalid persisted report: {error}"))?
    } else {
        let repair = serde_json::from_value::<RepairSummary>(value)
            .map_err(|error| format!("invalid persisted or repair report: {error}"))?;
        let status = match repair.verdict {
            VerificationVerdict::Pass => StageStatus::Passed,
            VerificationVerdict::Fail => StageStatus::Failed,
            VerificationVerdict::Inconclusive => StageStatus::Inconclusive,
        };
        PersistedReport {
            schema_version: repair.schema_version,
            meta: repair.meta,
            tool: repair.tool,
            candidate: repair.candidate,
            stages: vec![VerificationStage {
                name: "execute".into(),
                status,
                duration_ms: 0,
                detail: Some(serde_json::json!({
                    "findings": repair.findings,
                    "suppressed_findings": [],
                })),
                message: None,
            }],
            verdict: repair.verdict,
            strength: repair.strength,
            summary: repair.summary,
            diagnostics: repair.diagnostics,
            diagnostics_summary: repair.diagnostics_summary,
        }
    };
    if report.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported report schema {}; expected {}",
            report.schema_version, REPORT_SCHEMA_VERSION
        ));
    }
    Ok(report)
}
pub fn replay_launch_context(
    report_path: &str,
    finding_id: &str,
) -> Result<Option<ReproLaunchContext>, String> {
    let report = load_persisted_report(report_path)?;
    let mut matches = persisted_findings(&report)
        .into_iter()
        .filter(|finding| finding.id == finding_id);
    let finding = matches
        .next()
        .ok_or_else(|| format!("finding '{finding_id}' was not found in report"))?;
    if matches.next().is_some() {
        return Err(format!("finding id '{finding_id}' is duplicated"));
    }
    Ok(finding.launch_context)
}

fn replay_payload(stdout: &str) -> Result<serde_json::Value, String> {
    const MARKER: &str = "__COURT_JESTER_REPLAY_JSON__";
    if stdout.matches(MARKER).count() != 1 {
        return Err("replay sentinel must occur exactly once".into());
    }
    let after = stdout
        .split_once(MARKER)
        .map(|(_, value)| value.trim())
        .unwrap_or_default();
    let line = after.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return Err("replay sentinel has no JSON payload".into());
    }
    serde_json::from_str(line).map_err(|error| format!("invalid replay sentinel JSON: {error}"))
}

fn validate_differential_repro(
    differential: &DifferentialRepro,
    dependency_project_dir: Option<&str>,
) -> Result<(), String> {
    for source in differential
        .base_files
        .iter()
        .chain(differential.candidate_files.iter())
    {
        if stable_digest(&source.content) != source.sha256 {
            return Err(format!(
                "embedded source digest mismatch for {}",
                source.relative_path
            ));
        }
    }
    if tree_digest(&differential.base_files) != differential.base_tree_sha256 {
        return Err("embedded base tree digest mismatch".into());
    }
    if tree_digest(&differential.candidate_files) != differential.candidate_tree_sha256 {
        return Err("embedded candidate tree digest mismatch".into());
    }
    if !differential
        .dependency_contract
        .third_party_modules
        .is_empty()
        && dependency_project_dir.is_none()
    {
        return Err("replay requires --dependency-project-dir for third-party modules".into());
    }
    if let Some(root) = dependency_project_dir {
        for lockfile in &differential.dependency_contract.lockfiles {
            let path = Path::new(root).join(&lockfile.relative_path);
            let content = std::fs::read_to_string(&path)
                .map_err(|error| format!("dependency lockfile unavailable: {error}"))?;
            if stable_digest(&content) != lockfile.sha256 {
                return Err(format!(
                    "dependency lockfile digest mismatch for {}",
                    lockfile.relative_path
                ));
            }
        }
    }
    Ok(())
}
fn materialize_embedded_tree(
    files: &[EmbeddedSource],
    relative_entry: &str,
    label: &str,
) -> Result<(tempfile::TempDir, String, String), String> {
    let root = tempfile::tempdir()
        .map_err(|error| format!("failed to create {label} replay root: {error}"))?;
    let mut entry_content = None;
    let mut entry_path = None;
    for embedded in files {
        let relative = Path::new(&embedded.relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "invalid embedded source path '{}'",
                embedded.relative_path
            ));
        }
        let destination = root.path().join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to materialize {label} replay source: {error}"))?;
        }
        std::fs::write(&destination, &embedded.content)
            .map_err(|error| format!("failed to materialize {label} replay source: {error}"))?;
        if embedded.relative_path == relative_entry {
            entry_content = Some(embedded.content.clone());
            entry_path = Some(destination.to_string_lossy().to_string());
        }
    }
    match (entry_content, entry_path) {
        (Some(content), Some(path)) => Ok((root, content, path)),
        _ => Err(format!(
            "differential entry '{relative_entry}' is absent from embedded {label} sources"
        )),
    }
}

fn live_candidate_tree(
    project: &str,
    relative_entry: &str,
) -> Result<(PathBuf, String, String), String> {
    let relative = Path::new(relative_entry);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("live candidate entry must be a normalized project-relative path".into());
    }
    let root = std::fs::canonicalize(project)
        .map_err(|error| format!("live candidate project unavailable: {error}"))?;
    if !root.is_dir() {
        return Err("live candidate project must be a directory".into());
    }
    let entry = std::fs::canonicalize(root.join(relative))
        .map_err(|error| format!("live candidate source unavailable: {error}"))?;
    if !entry.starts_with(&root) || !entry.is_file() {
        return Err("live candidate source must be a file inside the candidate project".into());
    }
    let source = std::fs::read_to_string(&entry)
        .map_err(|error| format!("live candidate source unreadable: {error}"))?;
    let entry = entry
        .to_str()
        .ok_or("live candidate source path must be UTF-8")?
        .to_string();
    Ok((root, source, entry))
}

pub async fn replay_report(
    report_path: &str,
    finding_id: &str,
    dependency_project_dir: Option<&str>,
    runtime_profile: RuntimeProfile,
    python_docker_image: &str,
    typescript_docker_image: &str,
) -> Result<ReplayReport, String> {
    replay_report_with_options(
        report_path,
        finding_id,
        dependency_project_dir,
        runtime_profile,
        python_docker_image,
        typescript_docker_image,
        None,
        None,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn replay_report_with_options(
    report_path: &str,
    finding_id: &str,
    dependency_project_dir: Option<&str>,
    runtime_profile: RuntimeProfile,
    python_docker_image: &str,
    typescript_docker_image: &str,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
    network: Option<NetworkPolicy>,
    harness_args: Option<&[HarnessArg]>,
) -> Result<ReplayReport, String> {
    replay_report_with_candidate_options(
        report_path,
        finding_id,
        dependency_project_dir,
        runtime_profile,
        python_docker_image,
        typescript_docker_image,
        timeout_seconds,
        memory_mb,
        network,
        harness_args,
        None,
    )
    .await
}

/// Explicit live-candidate differential replay; the baseline remains embedded.
/// Omitting the candidate directory preserves historical snapshot replay.
#[allow(clippy::too_many_arguments)]
pub async fn replay_report_with_candidate_options(
    report_path: &str,
    finding_id: &str,
    dependency_project_dir: Option<&str>,
    runtime_profile: RuntimeProfile,
    python_docker_image: &str,
    typescript_docker_image: &str,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
    network: Option<NetworkPolicy>,
    harness_args: Option<&[HarnessArg]>,
    candidate_project_dir: Option<&str>,
) -> Result<ReplayReport, String> {
    let report = load_persisted_report(report_path)?;
    let mut matches = persisted_findings(&report)
        .into_iter()
        .filter(|finding| finding.id == finding_id);
    let finding = matches
        .next()
        .ok_or_else(|| format!("finding '{finding_id}' was not found in report"))?;
    if matches.next().is_some() {
        return Err(format!("finding id '{finding_id}' is duplicated"));
    }
    if candidate_project_dir.is_some() && finding.repro.differential.is_none() {
        return Err("--candidate-project-dir requires a differential finding; ordinary replay already checks current source".into());
    }
    let language = Language::parse(&report.meta.language).ok_or_else(|| {
        format!(
            "unsupported report language '{}'; expected python or typescript",
            report.meta.language
        )
    })?;

    let launch_context = finding.launch_context.as_ref();
    let docker_image = match language {
        Language::Python => python_docker_image,
        Language::TypeScript => typescript_docker_image,
    };
    let replay_timeout = timeout_seconds
        .or_else(|| launch_context.map(|context| context.limits.timeout_seconds))
        .unwrap_or(10.0);
    let replay_memory = memory_mb
        .or_else(|| launch_context.map(|context| context.limits.memory_mb))
        .unwrap_or(128);
    let replay_network = network
        .or_else(|| launch_context.map(|context| context.limits.network_policy))
        .unwrap_or(NetworkPolicy::Deny);
    let replay_harness_args = harness_args.unwrap_or_else(|| {
        launch_context
            .map(|context| context.harness_args.as_slice())
            .unwrap_or(&[])
    });
    if let Some(differential) = finding.repro.differential.as_ref() {
        if let Err(reason) = validate_differential_repro(differential, dependency_project_dir)
            .and_then(|()| {
                if let Some(project) = candidate_project_dir {
                    validate_differential_repro(differential, Some(project))
                } else {
                    Ok(())
                }
            })
        {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(&reason),
            });
        }
        let (base_root, base_source, base_entry) = match materialize_embedded_tree(
            &differential.base_files,
            &differential.relative_entry,
            "base",
        ) {
            Ok(materialized) => materialized,
            Err(reason) => {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&reason),
                })
            }
        };
        let candidate_tree = if let Some(project) = candidate_project_dir {
            live_candidate_tree(project, &differential.relative_entry)
                .map(|(root, source, entry)| (None, root, source, entry))
        } else {
            materialize_embedded_tree(
                &differential.candidate_files,
                &differential.relative_entry,
                "candidate",
            )
            .map(|(lease, source, entry)| {
                let root = lease.path().to_path_buf();
                (Some(lease), root, source, entry)
            })
        };
        let (_candidate_lease, candidate_root, candidate_source, candidate_entry) =
            match candidate_tree {
                Ok(materialized) => materialized,
                Err(reason) => {
                    return Ok(ReplayReport {
                        check_passed: None,
                        schema_version: REPORT_SCHEMA_VERSION,
                        finding_id: finding.id,
                        outcome: ReplayOutcome::Inconclusive,
                        execution: err_execution_result(&reason),
                    })
                }
            };
        let Some(symbol) = finding.repro.function.as_deref() else {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result("differential repro has no function symbol"),
            });
        };
        let base_context = match crate::resolve_execution_context(ContextRequest {
            invocation_dir: base_root.path(),
            explicit_project_dir: Some(base_root.path()),
            target_file: Some(Path::new(&base_entry)),
            test_file: None,
            language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay base context unavailable: {error}"
                    )),
                })
            }
        };
        let candidate_context = match crate::resolve_execution_context(ContextRequest {
            invocation_dir: &candidate_root,
            explicit_project_dir: Some(&candidate_root),
            target_file: Some(Path::new(&candidate_entry)),
            test_file: None,
            language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay candidate context unavailable: {error}"
                    )),
                })
            }
        };
        let base_analysis =
            analyze::analyze_with_context(&base_source, &base_context.target_source);
        let candidate_analysis =
            analyze::analyze_with_context(&candidate_source, &candidate_context.target_source);
        let base_function = base_analysis
            .functions
            .iter()
            .find(|function| function.name == symbol);
        let candidate_function = candidate_analysis
            .functions
            .iter()
            .find(|function| function.name == symbol);
        let (Some(base_function), Some(candidate_function)) = (base_function, candidate_function)
        else {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay surface is absent from the selected source tree",
                ),
            });
        };
        if !compatible_surface(candidate_function, base_function) {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result("differential surface signatures are incompatible"),
            });
        }
        let Some(differential_case) = differential_case_from_arguments(
            candidate_function,
            &finding.repro.arguments,
            &language,
        ) else {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay arguments do not match the stored surface bindings",
                ),
            });
        };
        let base_probe =
            differential_probe(&base_source, base_function, &differential_case, &language);
        let candidate_probe = differential_probe(
            &candidate_source,
            candidate_function,
            &differential_case,
            &language,
        );
        let base_options = SandboxOptions {
            timeout_seconds: replay_timeout,
            memory_mb: replay_memory,
            runtime_profile,
            network_policy: replay_network,
            harness_args: replay_harness_args,
            docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
            project_dir: base_root.path().to_str(),
            source_file: Some(&base_entry),
            instrumentation_target: None,
            instrumented_source: None,
        };
        let candidate_options = SandboxOptions {
            timeout_seconds: replay_timeout,
            memory_mb: replay_memory,
            runtime_profile,
            network_policy: replay_network,
            harness_args: replay_harness_args,
            docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
            project_dir: candidate_root.to_str(),
            source_file: Some(&candidate_entry),
            instrumentation_target: None,
            instrumented_source: None,
        };
        base_options.validate()?;
        candidate_options.validate()?;
        let base_execution = sandbox::execute(&base_probe, &language, base_options).await;
        let candidate_execution =
            sandbox::execute(&candidate_probe, &language, candidate_options).await;
        let base_snapshot = match differential_snapshot(&base_execution) {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay baseline snapshot unsupported: {reason}"
                    )),
                })
            }
        };
        let candidate_snapshot = match differential_snapshot(&candidate_execution) {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay candidate snapshot unsupported: {reason}"
                    )),
                })
            }
        };
        if differential_binding_failure(&base_snapshot, &language)
            || differential_binding_failure(&candidate_snapshot, &language)
            || (base_snapshot == candidate_snapshot && base_snapshot.exception_type.is_some())
        {
            return Ok(ReplayReport {
                check_passed: None,
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay case is an invalid generated invocation",
                ),
            });
        }
        let reproduced = base_snapshot != candidate_snapshot;
        let classify = |analysis: &crate::types::AnalysisResult,
                        function: &crate::types::FunctionInfo| {
            let plan = crate::tools::domain::build_verification_plan(
                &analysis.functions,
                &analysis.classes,
                &analysis.aliases,
                &language,
                &[],
                &[],
                &[],
            );
            super::planned_closed_input_classification(
                &function.name,
                function.line,
                &finding.repro.arguments,
                &plan,
            )
        };
        let input_classification = match (
            classify(&base_analysis, base_function),
            classify(&candidate_analysis, candidate_function),
        ) {
            (
                crate::types::InputClassification::Valid,
                crate::types::InputClassification::Valid,
            ) => crate::types::InputClassification::Valid,
            (crate::types::InputClassification::Invalid, _)
            | (_, crate::types::InputClassification::Invalid) => {
                crate::types::InputClassification::Invalid
            }
            _ => crate::types::InputClassification::Unknown,
        };
        let check_passed = candidate_project_dir.map(|_| {
            !reproduced
                && base_snapshot.exception_type.is_none()
                && candidate_snapshot.exception_type.is_none()
        });
        let payload = serde_json::json!({
            "reproduced": reproduced,
            "candidate_mode": if candidate_project_dir.is_some() { "live" } else { "embedded" },
            "input_classification": input_classification,
            "candidate_entry": candidate_entry,
            "candidate_source_sha256": stable_digest(&candidate_source),
            "base_tree_sha256": differential.base_tree_sha256,
            "check_passed": check_passed,
            "severity": finding.repro.expectation.severity,
            "oracle_kind": finding.repro.expectation.oracle_kind,
            "category": finding.repro.expectation.category,
        });
        let execution = ExecutionResult {
            stdout: format!(
                "__COURT_JESTER_REPLAY_JSON__{}\n",
                serde_json::to_string(&payload).unwrap_or_default()
            ),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: base_execution
                .duration_ms
                .saturating_add(candidate_execution.duration_ms),
            timed_out: false,
            memory_error: false,
            termination: Some(ProcessTermination {
                kind: ProcessTerminationKind::Exited,
                exit_code: Some(0),
                signal: None,
                signal_name: None,
            }),
            diagnostics: vec![],
        };
        return Ok(ReplayReport {
            check_passed,
            schema_version: REPORT_SCHEMA_VERSION,
            finding_id: finding.id,
            outcome: if reproduced {
                ReplayOutcome::Reproduced
            } else {
                ReplayOutcome::NotReproduced
            },
            execution,
        });
    }

    let mut source_file_owned = None;
    let mut source = String::new();
    if let Some(path) = report.meta.source_file.as_deref() {
        let source_path =
            if let Some(root) = dependency_project_dir.filter(|_| !Path::new(path).is_absolute()) {
                Path::new(root).join(path)
            } else if Path::new(path).is_file() {
                PathBuf::from(path)
            } else if let Some(root) = dependency_project_dir {
                Path::new(root).join(path)
            } else {
                return Ok(ReplayReport {
                    check_passed: None,
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(
                        "relative replay source requires --dependency-project-dir",
                    ),
                });
            };
        source = std::fs::read_to_string(&source_path)
            .map_err(|error| format!("source context unavailable for replay: {error}"))?;
        source_file_owned = Some(source_path.to_string_lossy().to_string());
    }
    let snippet = if let Some(contract) = &finding.repro.native_replay {
        render_native_replay(&finding.repro.arguments, contract, &language)?
    } else {
        finding.repro.snippet.clone()
    };
    let code = if source.is_empty() {
        snippet
    } else {
        let mut code = generated_target_source(&source, &language);
        code.push('\n');
        code.push_str(&snippet);
        code
    };
    let source_file = source_file_owned.as_deref();
    let project_dir_owned = dependency_project_dir.map(ToOwned::to_owned).or_else(|| {
        source_file.and_then(|path| {
            Path::new(path)
                .parent()
                .and_then(|parent| parent.to_str())
                .map(ToOwned::to_owned)
        })
    });
    let project_dir = project_dir_owned.as_deref();
    let options = SandboxOptions {
        timeout_seconds: replay_timeout,
        memory_mb: replay_memory,
        runtime_profile,
        network_policy: replay_network,
        harness_args: replay_harness_args,
        docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
        project_dir,
        source_file,
        instrumentation_target: None,
        instrumented_source: None,
    };
    options.validate()?;
    let execution = sandbox::execute(&code, &language, options).await;
    let mut check_passed = None;
    let payload =
        if execution.exit_code == Some(0) && !execution.timed_out && !execution.memory_error {
            replay_payload(&execution.stdout)
        } else {
            Err("replay process did not complete successfully".into())
        };
    let outcome = match payload {
        Ok(payload) => {
            let reproduced = payload.get("reproduced").and_then(|value| value.as_bool());
            let expected = |value: serde_json::Value| value.as_str().map(ToOwned::to_owned);
            let matches_expectation = payload.get("severity").and_then(|value| value.as_str())
                == expected(
                    serde_json::to_value(finding.repro.expectation.severity).unwrap_or_default(),
                )
                .as_deref()
                && payload.get("oracle_kind").and_then(|value| value.as_str())
                    == expected(
                        serde_json::to_value(finding.repro.expectation.oracle_kind)
                            .unwrap_or_default(),
                    )
                    .as_deref()
                && payload.get("category").and_then(|value| value.as_str())
                    == expected(
                        serde_json::to_value(finding.repro.expectation.category)
                            .unwrap_or_default(),
                    )
                    .as_deref();
            let positive = payload
                .get("check_passed")
                .and_then(|value| value.as_bool());
            let requires_oracle = finding.repro.expectation.severity
                == FindingSeverity::PropertyViolation
                && matches!(
                    finding.repro.expectation.oracle_kind,
                    OracleKind::DeclaredProperty
                        | OracleKind::GenericProperty
                        | OracleKind::TypeContract
                );
            let required_oracle = payload
                .get("required_oracle")
                .and_then(|value| value.as_str())
                .filter(|id| !id.is_empty());
            let passed_oracles = payload
                .get("passed_oracles")
                .and_then(|value| value.as_array())
                .filter(|values| {
                    values
                        .iter()
                        .all(|value| value.as_str().is_some_and(|id| !id.is_empty()))
                });
            let witnessed = required_oracle
                .zip(passed_oracles)
                .map(|(required, passed)| {
                    passed.iter().any(|value| value.as_str() == Some(required))
                });
            let declares_witness =
                payload.get("required_oracle").is_some() || payload.get("passed_oracles").is_some();
            if matches_expectation
                && reproduced.is_some()
                && !(reproduced == Some(true) && positive == Some(true))
                && (payload.get("check_passed").is_none() || positive.is_some())
                && !(requires_oracle && positive == Some(true) && witnessed == Some(false))
                && !(requires_oracle && declares_witness && witnessed.is_none())
            {
                // Older property snippets claimed normal evaluator return, not
                // execution of the recorded oracle. Preserve replay compatibility
                // without treating that claim as a passing regression contract.
                check_passed = if requires_oracle && witnessed.is_none() {
                    None
                } else {
                    positive
                };
                if reproduced == Some(true) {
                    ReplayOutcome::Reproduced
                } else {
                    ReplayOutcome::NotReproduced
                }
            } else {
                ReplayOutcome::Inconclusive
            }
        }
        Err(_) => ReplayOutcome::Inconclusive,
    };
    Ok(ReplayReport {
        check_passed,
        schema_version: REPORT_SCHEMA_VERSION,
        finding_id: finding.id,
        outcome,
        execution,
    })
}
