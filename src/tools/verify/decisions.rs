//! Stage evidence, diagnostic precedence, coverage accounting, and final verdicts.

use crate::types::{
    CandidateProvenance, CoverageGate, CoverageSummary, DiagnosticComponent, DiagnosticImpact,
    DiagnosticsSummary, FailureDiagnostic, FailureDomain, FailureKind, FindingsSummary,
    OrthogonalOutcome, ProcessTermination, ProcessTerminationKind, ReportSummary, StageStatus,
    ToolProvenance, VerificationEvidence, VerificationOutcomeMatrix, VerificationReport,
    VerificationStage, VerificationStrength, VerificationVerdict, REPORT_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};

fn coverage_has_gap(coverage: &CoverageSummary, gate: CoverageGate) -> bool {
    gate == CoverageGate::ChangedExports && coverage.required > coverage.behaviorally_checked
}

/// Compute the schema-v3 verdict and evidence strength from typed stage/evidence data.
pub fn final_verdict(
    stages: &[VerificationStage],
    coverage: &CoverageSummary,
    gate: CoverageGate,
    evidence: &VerificationEvidence,
) -> (VerificationVerdict, VerificationStrength) {
    let parse_failed = stages
        .iter()
        .any(|stage| stage.name == "parse" && stage.status == StageStatus::Failed);
    let strength = if parse_failed {
        VerificationStrength::ParseOnly
    } else if evidence.authoritative_test_completed {
        VerificationStrength::AuthoritativeTests
    } else if evidence.evaluated_oracles > 0 {
        VerificationStrength::PropertyChecked
    } else if evidence.valid_invocations > 0 {
        VerificationStrength::RuntimeSmoke
    } else if evidence.static_checks_completed {
        VerificationStrength::StaticChecked
    } else if evidence.parsed {
        VerificationStrength::ParseOnly
    } else {
        VerificationStrength::None
    };
    // Typed causes outrank the lossy stage status: a gating target cause
    // remains a failure even when the process also reported a resource or
    // harness termination, while a blocking non-target cause is inconclusive.
    let diagnostics = diagnostics_from_relevant_stages(stages, coverage, evidence);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.impact == DiagnosticImpact::Gating)
    {
        return (VerificationVerdict::Fail, strength);
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.impact == DiagnosticImpact::Blocking)
    {
        return (VerificationVerdict::Inconclusive, strength);
    }
    if stages.iter().any(|stage| {
        !generated_execution_is_superseded(stage, coverage, evidence)
            && stage.status == StageStatus::Failed
    }) {
        return (VerificationVerdict::Fail, strength);
    }
    if coverage_has_gap(coverage, gate)
        || (!evidence.authoritative_test_completed
            && evidence.valid_invocations == 0
            && evidence.evaluated_oracles == 0)
        || stages.iter().any(|stage| {
            !generated_execution_is_superseded(stage, coverage, evidence)
                && (stage.status == StageStatus::Inconclusive
                    || (stage.name == "execute"
                        && stage.status == StageStatus::Skipped
                        && !evidence.authoritative_test_completed))
        })
    {
        return (VerificationVerdict::Inconclusive, strength);
    }
    (VerificationVerdict::Pass, strength)
}

pub(super) fn is_typescript_portability_error(stderr: &str) -> bool {
    stderr.contains("ERR_MODULE_NOT_FOUND")
        || stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
        || stderr.contains("Cannot find module 'bun'")
        || stderr.contains("Cannot find package 'bun'")
        || stderr.contains("Bun is not defined")
        || stderr.contains("needs an import attribute of \"type: json\"")
}

pub(super) fn is_typescript_module_load_error(stderr: &str) -> bool {
    is_typescript_portability_error(stderr)
        || stderr.contains("Cannot find module")
        || stderr.contains("Cannot find package")
        || stderr.contains("The requested module")
}

fn diagnostic_from_termination(
    termination: &ProcessTermination,
    message: String,
) -> FailureDiagnostic {
    let (domain, kind) = match termination.kind {
        ProcessTerminationKind::TimedOut => (FailureDomain::Resource, FailureKind::Timeout),
        ProcessTerminationKind::MemoryLimit => (FailureDomain::Resource, FailureKind::MemoryLimit),
        ProcessTerminationKind::Signaled => (FailureDomain::Environment, FailureKind::Signal),
        ProcessTerminationKind::LaunchFailed => {
            (FailureDomain::Environment, FailureKind::LauncherFailure)
        }
        ProcessTerminationKind::WaitFailed => {
            (FailureDomain::Environment, FailureKind::ToolFailure)
        }
        ProcessTerminationKind::Exited => (FailureDomain::Environment, FailureKind::NonzeroExit),
    };
    FailureDiagnostic {
        domain,
        kind,
        component: DiagnosticComponent::Sandbox,
        impact: DiagnosticImpact::Blocking,
        message,
        process: Some(termination.clone()),
        limits: None,
    }
}

fn is_non_target_blocker(diagnostic: &FailureDiagnostic) -> bool {
    diagnostic.impact == DiagnosticImpact::Blocking
        && diagnostic.domain != FailureDomain::TargetCode
        && diagnostic.kind != FailureKind::NonzeroExit
}

pub(super) fn has_non_target_blocking_diagnostic(diagnostics: &[FailureDiagnostic]) -> bool {
    diagnostics.iter().any(is_non_target_blocker)
}

fn detail_has_non_target_blocker(detail: Option<&serde_json::Value>) -> bool {
    let Some(detail) = detail else {
        return false;
    };
    if detail
        .get("non_target_blocking")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return true;
    }
    let has_blocker = |value: &serde_json::Value| {
        ["diagnostics", "failure_diagnostics"]
            .iter()
            .filter_map(|key| value.get(key).and_then(|entries| entries.as_array()))
            .flatten()
            .filter_map(|entry| serde_json::from_value::<FailureDiagnostic>(entry.clone()).ok())
            .any(|d| is_non_target_blocker(&d))
    };
    has_blocker(detail) || detail.get("execution").is_some_and(has_blocker)
}

fn detail_has_target_finding(detail: Option<&serde_json::Value>) -> bool {
    let Some(detail) = detail else {
        return false;
    };
    detail
        .get("findings")
        .and_then(|value| value.as_array())
        .is_some_and(|findings| !findings.is_empty())
        || detail
            .get("suppressed_findings")
            .and_then(|value| value.as_array())
            .is_some_and(|findings| !findings.is_empty())
        || detail
            .get("finding_count")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0)
}

fn diagnostic_from_stage(
    stage: &VerificationStage,
    coverage_gate: CoverageGate,
) -> Option<FailureDiagnostic> {
    let detail = stage.detail.as_ref();
    let message = stage
        .message
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{} stage did not complete successfully", stage.name));

    let simple = |domain, kind, component, impact| FailureDiagnostic {
        domain,
        kind,
        component,
        impact,
        message: message.clone(),
        process: None,
        limits: None,
    };

    match stage.name.as_str() {
        "context" => Some(simple(
            FailureDomain::Environment,
            FailureKind::ContextResolution,
            DiagnosticComponent::ModuleLoader,
            DiagnosticImpact::Blocking,
        )),
        "parse" => {
            let unsupported = detail
                .and_then(|value| value.get("parse_diagnostics"))
                .and_then(|value| value.as_array())
                .is_some_and(|values| {
                    values.iter().any(|value| {
                        value.get("kind").and_then(|kind| kind.as_str()) == Some("unsupported")
                    })
                });
            let parse_message = detail
                .and_then(|value| value.get("parse_diagnostics"))
                .and_then(|value| value.as_array())
                .and_then(|values| values.first())
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
                .map(|value| format!("{value} ({}).", message))
                .unwrap_or(message);
            Some(FailureDiagnostic {
                domain: if unsupported {
                    FailureDomain::VerifierHarness
                } else {
                    FailureDomain::TargetCode
                },
                kind: if unsupported {
                    FailureKind::Instrumentation
                } else {
                    FailureKind::SyntaxError
                },
                component: if unsupported {
                    DiagnosticComponent::Instrumentation
                } else {
                    DiagnosticComponent::Target
                },
                impact: if unsupported {
                    DiagnosticImpact::Blocking
                } else {
                    DiagnosticImpact::Gating
                },
                message: parse_message,
                process: None,
                limits: None,
            })
        }
        "complexity" => Some(simple(
            FailureDomain::TargetCode,
            FailureKind::ComplexityThreshold,
            DiagnosticComponent::Target,
            DiagnosticImpact::Gating,
        )),
        "lint" => {
            let runner_failed = detail
                .and_then(|value| value.get("runner_failed"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if runner_failed || stage.status != StageStatus::Advisory {
                Some(simple(
                    FailureDomain::Environment,
                    if runner_failed {
                        FailureKind::ToolFailure
                    } else {
                        FailureKind::LauncherFailure
                    },
                    DiagnosticComponent::LintRunner,
                    DiagnosticImpact::Advisory,
                ))
            } else {
                None
            }
        }
        "coverage" => {
            let required_functions = detail
                .and_then(|value| value.get("functions"))
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter(|function| {
                    function
                        .get("required")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            let every_required_surface_accounted_for = !required_functions.is_empty()
                && required_functions.iter().all(|function| {
                    matches!(
                        function.get("status").and_then(|value| value.as_str()),
                        Some(
                            "checked_direct"
                                | "checked_via_factory"
                                | "checked_via_caller"
                                | "checked_via_authoritative_test"
                                | "reached_via_factory"
                                | "reached_direct"
                                | "reached_via_authoritative_test"
                        )
                    )
                });
            (!every_required_surface_accounted_for).then(|| {
                simple(
                    FailureDomain::VerifierHarness,
                    FailureKind::ContractViolation,
                    DiagnosticComponent::FuzzHarness,
                    if coverage_gate == CoverageGate::ChangedExports {
                        DiagnosticImpact::Blocking
                    } else {
                        DiagnosticImpact::Advisory
                    },
                )
            })
        }
        "portability" => Some(simple(
            FailureDomain::Environment,
            FailureKind::ToolFailure,
            DiagnosticComponent::Sandbox,
            DiagnosticImpact::Blocking,
        )),
        "differential" => Some(simple(
            FailureDomain::VerifierHarness,
            FailureKind::AmbiguousGeneratedInput,
            DiagnosticComponent::DifferentialRunner,
            DiagnosticImpact::Advisory,
        )),
        "llm_plateau_escape" => Some(
            if detail
                .and_then(|value| value.get("gating_finding_count"))
                .and_then(|value| value.as_u64())
                .is_some_and(|count| count > 0)
            {
                simple(
                    FailureDomain::TargetCode,
                    FailureKind::TargetException,
                    DiagnosticComponent::Target,
                    DiagnosticImpact::Gating,
                )
            } else if detail
                .and_then(|value| value.get("unknown_finding_count"))
                .and_then(|value| value.as_u64())
                .is_some_and(|count| count > 0)
            {
                simple(
                    FailureDomain::VerifierHarness,
                    FailureKind::AmbiguousGeneratedInput,
                    DiagnosticComponent::FuzzHarness,
                    DiagnosticImpact::Blocking,
                )
            } else {
                simple(
                    FailureDomain::Environment,
                    FailureKind::ToolFailure,
                    DiagnosticComponent::Sandbox,
                    DiagnosticImpact::Blocking,
                )
            },
        ),
        "execute" | "test" => {
            let is_test = stage.name == "test";
            let non_target_blocked = detail_has_non_target_blocker(detail);
            let assertion_failure = is_test
                && !non_target_blocked
                && (message.contains("Assertion failed")
                    || message.contains("AssertionError")
                    || detail
                        .and_then(|value| value.get("assertion_failure"))
                        .and_then(|value| value.as_bool())
                        == Some(true));
            let has_target_finding = detail_has_target_finding(detail);
            let execution = detail.and_then(|value| value.get("execution")).or(detail);
            let module_load_blocked = detail
                .and_then(|value| value.get("module_load_blocked"))
                .and_then(|value| value.as_bool())
                == Some(true)
                || execution
                    .and_then(|value| value.get("stderr"))
                    .and_then(|value| value.as_str())
                    .is_some_and(is_typescript_module_load_error);
            if module_load_blocked {
                return Some(simple(
                    FailureDomain::Environment,
                    FailureKind::ModuleLoad,
                    DiagnosticComponent::ModuleLoader,
                    DiagnosticImpact::Blocking,
                ));
            }
            if non_target_blocked {
                return None;
            }
            if let Some(termination) = execution
                .and_then(|value| value.get("termination"))
                .and_then(|value| serde_json::from_value::<ProcessTermination>(value.clone()).ok())
            {
                // A target finding is authoritative and must not be replaced by
                // a generic nonzero-exit diagnostic. Resource/process causes are
                // retained alongside that finding.
                if !(termination.kind == ProcessTerminationKind::Exited
                    && termination.exit_code == Some(0)
                    || assertion_failure
                    || has_target_finding
                        && termination.kind == ProcessTerminationKind::Exited
                        && termination.exit_code != Some(0))
                {
                    return Some(diagnostic_from_termination(&termination, message));
                }
            }

            let overlay_unsupported = detail
                .and_then(|value| value.get("instrumentation_overlay"))
                .and_then(|value| value.get("supported"))
                .and_then(|value| value.as_bool())
                == Some(false);
            if overlay_unsupported {
                return Some(simple(
                    FailureDomain::VerifierHarness,
                    FailureKind::Instrumentation,
                    DiagnosticComponent::Instrumentation,
                    DiagnosticImpact::Blocking,
                ));
            }

            if assertion_failure {
                return Some(simple(
                    FailureDomain::TargetCode,
                    FailureKind::AssertionFailure,
                    DiagnosticComponent::AuthoritativeTestRunner,
                    DiagnosticImpact::Gating,
                ));
            }
            if has_target_finding {
                return Some(simple(
                    FailureDomain::TargetCode,
                    if is_test {
                        FailureKind::AssertionFailure
                    } else {
                        FailureKind::TargetException
                    },
                    if is_test {
                        DiagnosticComponent::AuthoritativeTestRunner
                    } else {
                        DiagnosticComponent::Target
                    },
                    DiagnosticImpact::Gating,
                ));
            }
            if !has_target_finding {
                return Some(simple(
                    if is_test {
                        FailureDomain::Environment
                    } else {
                        FailureDomain::VerifierHarness
                    },
                    if is_test {
                        FailureKind::ToolFailure
                    } else if detail
                        .and_then(|value| value.get("no_inputs_reached"))
                        .and_then(|value| value.as_u64())
                        .is_some_and(|count| count > 0)
                    {
                        FailureKind::AmbiguousGeneratedInput
                    } else {
                        FailureKind::HarnessProtocol
                    },
                    if is_test {
                        DiagnosticComponent::AuthoritativeTestRunner
                    } else {
                        DiagnosticComponent::FuzzHarness
                    },
                    DiagnosticImpact::Blocking,
                ));
            }
            None
        }
        _ => Some(simple(
            FailureDomain::Environment,
            FailureKind::ToolFailure,
            DiagnosticComponent::Sandbox,
            DiagnosticImpact::Blocking,
        )),
    }
}

fn append_unique_diagnostic(
    diagnostics: &mut Vec<FailureDiagnostic>,
    diagnostic: FailureDiagnostic,
) {
    let key = serde_json::to_string(&diagnostic).unwrap_or_else(|_| diagnostic.message.clone());
    if !diagnostics.iter().any(|existing| {
        serde_json::to_string(existing)
            .map(|value| value == key)
            .unwrap_or(false)
    }) {
        diagnostics.push(diagnostic);
    }
}

fn diagnostics_from_stage_detail(detail: Option<&serde_json::Value>) -> Vec<FailureDiagnostic> {
    let mut diagnostics = Vec::new();
    let Some(detail) = detail else {
        return diagnostics;
    };
    let target_cause = detail_has_target_finding(Some(detail))
        || detail
            .get("assertion_failure")
            .and_then(|value| value.as_bool())
            == Some(true);
    let non_target_blocked = detail_has_non_target_blocker(Some(detail));
    for key in ["diagnostics", "failure_diagnostics"] {
        if let Some(values) = detail.get(key).and_then(|value| value.as_array()) {
            for value in values {
                if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
                    if (target_cause || non_target_blocked)
                        && diagnostic.kind == FailureKind::NonzeroExit
                    {
                        continue;
                    }
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }
    }
    if let Some(value) = detail.get("diagnostic") {
        if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
            append_unique_diagnostic(&mut diagnostics, diagnostic);
        }
    }
    // ExecutionResult is deliberately kept as a nested legacy-compatible
    // object. Promote its typed diagnostics and authoritative termination to
    if let Some(execution) = detail.get("execution") {
        if let Some(values) = execution
            .get("diagnostics")
            .and_then(|value| value.as_array())
        {
            for value in values {
                if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
                    let target_cause = detail_has_target_finding(Some(detail))
                        || detail
                            .get("assertion_failure")
                            .and_then(|value| value.as_bool())
                            == Some(true);
                    if (target_cause || non_target_blocked)
                        && diagnostic.kind == FailureKind::NonzeroExit
                    {
                        continue;
                    }
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }
    }
    diagnostics
}
pub fn stage_diagnostics(stage: &VerificationStage) -> Vec<FailureDiagnostic> {
    diagnostics_from_stage_detail(stage.detail.as_ref())
}

fn annotate_stage_diagnostics(stages: &mut [VerificationStage], coverage_gate: CoverageGate) {
    for stage in stages {
        let mut diagnostics = diagnostics_from_stage_detail(stage.detail.as_ref());
        let should_infer = matches!(
            stage.status,
            StageStatus::Failed | StageStatus::Inconclusive
        ) || (stage.name == "lint"
            && stage
                .detail
                .as_ref()
                .and_then(|value| value.get("runner_failed"))
                .and_then(|value| value.as_bool())
                == Some(true));

        if stage.name == "execute" || stage.name == "test" {
            let execution = stage
                .detail
                .as_ref()
                .and_then(|value| value.get("execution"))
                .or(stage.detail.as_ref());
            if let Some(execution) = execution {
                if let Some(termination) = execution.get("termination").and_then(|value| {
                    serde_json::from_value::<ProcessTermination>(value.clone()).ok()
                }) {
                    let non_target_blocked = detail_has_non_target_blocker(stage.detail.as_ref());
                    let assertion_failure = stage.name == "test"
                        && !non_target_blocked
                        && (stage.message.as_deref().is_some_and(|message| {
                            message.contains("Assertion failed")
                                || message.contains("AssertionError")
                        }) || stage
                            .detail
                            .as_ref()
                            .and_then(|value| value.get("assertion_failure"))
                            .and_then(|value| value.as_bool())
                            == Some(true));
                    let has_target_finding = detail_has_target_finding(stage.detail.as_ref());
                    let module_load_blocked = stage
                        .detail
                        .as_ref()
                        .and_then(|value| value.get("module_load_blocked"))
                        .and_then(|value| value.as_bool())
                        == Some(true)
                        || execution
                            .get("stderr")
                            .and_then(|value| value.as_str())
                            .is_some_and(is_typescript_module_load_error);
                    let should_record_exit = !module_load_blocked
                        && !non_target_blocked
                        && !assertion_failure
                        && (termination.kind != ProcessTerminationKind::Exited
                            || (termination.exit_code != Some(0) && !has_target_finding));
                    if should_record_exit {
                        append_unique_diagnostic(
                            &mut diagnostics,
                            diagnostic_from_termination(
                                &termination,
                                stage.message.clone().unwrap_or_else(|| {
                                    format!("{} process did not exit successfully", stage.name)
                                }),
                            ),
                        );
                    }
                }
            }
        }

        if should_infer {
            if let Some(diagnostic) = diagnostic_from_stage(stage, coverage_gate) {
                // Preserve typed target/resource causes already emitted by the
                // harness, while still guaranteeing one cause for this stage.
                let same_kind = diagnostics
                    .iter()
                    .any(|existing| existing.kind == diagnostic.kind);
                if !same_kind {
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }

        if !diagnostics.is_empty() {
            let detail = stage.detail.get_or_insert_with(|| serde_json::json!({}));
            if let Some(object) = detail.as_object_mut() {
                if object
                    .get("diagnostic")
                    .and_then(|value| {
                        serde_json::from_value::<FailureDiagnostic>(value.clone()).ok()
                    })
                    .is_none()
                {
                    let diagnostics_key = if stage.name == "lint" {
                        "failure_diagnostics"
                    } else {
                        "diagnostics"
                    };
                    object.insert(
                        diagnostics_key.into(),
                        serde_json::to_value(&diagnostics)
                            .unwrap_or_else(|_| serde_json::json!([])),
                    );
                }
            }
        }
    }
}

fn generated_execution_is_superseded(
    stage: &VerificationStage,
    coverage: &CoverageSummary,
    evidence: &VerificationEvidence,
) -> bool {
    if stage.name != "execute"
        || !evidence.authoritative_test_completed
        || coverage.required == 0
        || coverage.behaviorally_checked < coverage.required
    {
        return false;
    }
    let diagnostics = diagnostics_from_stage_detail(stage.detail.as_ref());
    !diagnostics.is_empty()
        && diagnostics
            .iter()
            .all(|diagnostic| diagnostic.domain != FailureDomain::TargetCode)
}

fn diagnostics_from_relevant_stages(
    stages: &[VerificationStage],
    coverage: &CoverageSummary,
    evidence: &VerificationEvidence,
) -> Vec<FailureDiagnostic> {
    let mut diagnostics = Vec::new();
    for stage in stages {
        if generated_execution_is_superseded(stage, coverage, evidence) {
            continue;
        }
        for diagnostic in diagnostics_from_stage_detail(stage.detail.as_ref()) {
            append_unique_diagnostic(&mut diagnostics, diagnostic);
        }
    }
    diagnostics
}
fn orthogonal_outcome(stages: &[VerificationStage], names: &[&str]) -> OrthogonalOutcome {
    let matching = stages
        .iter()
        .filter(|stage| names.contains(&stage.name.as_str()))
        .collect::<Vec<_>>();
    if matching.is_empty()
        || matching
            .iter()
            .all(|stage| stage.status == StageStatus::Skipped)
    {
        return OrthogonalOutcome::NotRun;
    }
    if matching
        .iter()
        .any(|stage| stage.status == StageStatus::Failed)
    {
        return OrthogonalOutcome::Failed;
    }
    if matching
        .iter()
        .any(|stage| stage.status == StageStatus::Inconclusive)
    {
        return OrthogonalOutcome::Blocked;
    }
    OrthogonalOutcome::Passed
}

fn verification_outcome_matrix(stages: &[VerificationStage]) -> VerificationOutcomeMatrix {
    VerificationOutcomeMatrix {
        static_analysis: orthogonal_outcome(stages, &["parse", "complexity", "lint"]),
        generated_execution: orthogonal_outcome(stages, &["execute"]),
        authoritative_tests: orthogonal_outcome(stages, &["test"]),
        portability: orthogonal_outcome(stages, &["portability"]),
    }
}

pub(super) fn build_report(
    mut stages: Vec<VerificationStage>,
    gate: CoverageGate,
    code: &str,
    source_file: Option<&str>,
) -> VerificationReport {
    // Normalize stage-local causes before computing the report-level verdict.
    // This keeps old stage JSON readable while ensuring every failed or
    // inconclusive stage has a typed, deduplicated provenance record.
    annotate_stage_diagnostics(&mut stages, gate);
    let mut summary = compute_report_summary(&stages);
    let evidence = evidence_from_stages(&stages);
    let diagnostics = diagnostics_from_relevant_stages(&stages, &summary.coverage, &evidence);
    summary.diagnostics = DiagnosticsSummary::from_diagnostics(&diagnostics);
    let (verdict, strength) = final_verdict(&stages, &summary.coverage, gate, &evidence);
    let outcome_matrix = verification_outcome_matrix(&stages);
    stages.push(VerificationStage {
        name: "outcome_matrix".into(),
        status: StageStatus::Passed,
        duration_ms: 0,
        detail: Some(
            serde_json::to_value(outcome_matrix).unwrap_or_else(|_| serde_json::json!({})),
        ),
        message: None,
    });
    VerificationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        tool: ToolProvenance {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        candidate: CandidateProvenance {
            content_sha256: format!("{:x}", Sha256::digest(code.as_bytes())),
            source_file: source_file.map(ToOwned::to_owned),
        },
        stages,
        verdict,
        strength,
        summary,
        diagnostics_summary: (!diagnostics.is_empty())
            .then(|| DiagnosticsSummary::from_diagnostics(&diagnostics)),
        diagnostics,
        report_path: None,
    }
}

fn evidence_from_stages(stages: &[VerificationStage]) -> VerificationEvidence {
    let parsed = stages
        .iter()
        .any(|stage| stage.name == "parse" && stage.status != StageStatus::Failed);
    let static_checks_completed = parsed
        && stages
            .iter()
            .any(|stage| matches!(stage.name.as_str(), "lint" | "complexity"));
    let mut evidence = VerificationEvidence {
        parsed,
        static_checks_completed,
        ..Default::default()
    };
    for stage in stages {
        if stage.name == "execute" {
            if let Some(detail) = &stage.detail {
                evidence.valid_invocations += detail
                    .get("valid_invocations")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                evidence.evaluated_oracles += detail
                    .get("evaluated_oracles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
            }
        }
        if stage.name == "test" && stage.status == StageStatus::Passed {
            evidence.authoritative_test_completed = true;
        }
    }
    evidence
}

fn coverage_summary_from_stages(stages: &[VerificationStage]) -> CoverageSummary {
    let mut summary = CoverageSummary::default();
    for stage in stages.iter().filter(|stage| stage.name == "coverage") {
        let Some(functions) = stage
            .detail
            .as_ref()
            .and_then(|d| d.get("functions"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for function in functions {
            let required = function
                .get("required")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if required {
                summary.required += 1;
            }
            match function.get("status").and_then(|v| v.as_str()) {
                Some(
                    "checked_direct"
                    | "checked_via_factory"
                    | "checked_via_caller"
                    | "checked_via_authoritative_test",
                ) if required => summary.behaviorally_checked += 1,
                Some(
                    "reached_direct" | "reached_via_factory" | "reached_via_authoritative_test",
                ) if required => summary.reached_only += 1,
                Some("blocked_module_load") => summary.blocked += 1,
                Some(
                    "skipped_no_fuzzable_surface"
                    | "skipped_unsupported_type"
                    | "skipped_internal_helper"
                    | "skipped_method"
                    | "skipped_nested"
                    | "skipped_private_name"
                    | "skipped_diff_filtered",
                ) => summary.skipped += 1,
                _ => {}
            }
        }
        summary.no_inputs_reached += stage
            .detail
            .as_ref()
            .and_then(|d| d.get("no_inputs_reached"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
    }
    summary
}

fn compute_report_summary(stages: &[VerificationStage]) -> ReportSummary {
    let mut summary = ReportSummary {
        functions_analyzed: 0,
        functions_fuzzed: 0,
        functions_skipped: 0,
        functions_blocked_module_load: 0,
        fuzz_pass: 0,
        fuzz_no_inputs_reached: 0,
        findings: FindingsSummary::default(),
        suppressed_complexity_violations: 0,
        suppressed_portability_warnings: 0,
        lint_issues: 0,
        lint_runner_failures: 0,
        complexity_violations: 0,
        coverage: CoverageSummary::default(),
        diagnostics: DiagnosticsSummary::default(),
    };
    summary.coverage = coverage_summary_from_stages(stages);
    for stage in stages {
        let Some(detail) = &stage.detail else {
            continue;
        };
        match stage.name.as_str() {
            "parse" => {
                summary.functions_analyzed = detail
                    .get("functions")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0)
            }
            "coverage" => {
                if let Some(funcs) = detail.get("functions").and_then(|v| v.as_array()) {
                    for func in funcs {
                        match func.get("status").and_then(|v| v.as_str()) {
                            Some(
                                "checked_direct"
                                | "checked_via_factory"
                                | "checked_via_caller"
                                | "checked_via_authoritative_test"
                                | "reached_direct",
                            ) => summary.functions_fuzzed += 1,
                            Some("blocked_module_load") => {
                                summary.functions_blocked_module_load += 1
                            }
                            Some(_) => summary.functions_skipped += 1,
                            None => {}
                        }
                    }
                }
            }
            "execute" => {
                summary.findings = detail
                    .get("findings_summary")
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .unwrap_or_default();
                summary.fuzz_pass = detail
                    .get("functions_with_valid_invocations")
                    .or_else(|| detail.get("valid_invocations"))
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                summary.fuzz_no_inputs_reached = detail
                    .get("no_inputs_reached")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
            }
            "lint" => {
                let runner_failed = detail
                    .get("runner_failed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                summary.lint_issues = if runner_failed {
                    0
                } else {
                    detail
                        .get("diagnostics")
                        .and_then(|v| v.as_array())
                        .map(|v| v.len())
                        .unwrap_or(0)
                };
                if runner_failed {
                    summary.lint_runner_failures += 1;
                }
            }
            "complexity" => {
                summary.complexity_violations = detail
                    .get("violations")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                summary.suppressed_complexity_violations = detail
                    .get("suppressed_violations")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
            }
            "portability"
                if detail
                    .get("suppressed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false) =>
            {
                summary.suppressed_portability_warnings += 1
            }
            _ => {}
        }
    }
    summary.coverage.no_inputs_reached = summary.fuzz_no_inputs_reached;
    summary
}
