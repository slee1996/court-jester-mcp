//! JSON and human report views, test-quality summaries, and report persistence.

use super::report_text::{
    clip_report_text, sanitize_report_text, sanitize_report_value, MAX_REPORT_MESSAGE_CHARS,
};
use crate::types::{
    Language, PersistedReport, ReportLevel, ReportMeta, StageStatus, VerificationReport,
    VerificationStage, REPORT_SCHEMA_VERSION,
};
use serde::Serialize;
use std::fmt::Write as _;

const MAX_MINIMAL_FINDINGS_PER_GROUP: usize = 16;

fn minimal_plan_counts(detail: &serde_json::Value) -> serde_json::Value {
    let plan = detail
        .get("verification_plan")
        .unwrap_or(&serde_json::Value::Null);
    let parameter_domains = plan
        .get("parameter_domains")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let inputs = plan
        .get("inputs")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut source_kind_counts = serde_json::Map::new();
    for parameter in parameter_domains {
        for source in parameter
            .get("sources")
            .and_then(|value| value.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if let Some(kind) = source.get("kind").and_then(|value| value.as_str()) {
                let count = source_kind_counts
                    .get(kind)
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                source_kind_counts.insert(kind.to_string(), serde_json::Value::from(count + 1));
            }
        }
    }
    serde_json::json!({
        "domain_param_count": parameter_domains.len(),
        "closed_domain_param_count": parameter_domains.iter().filter(|parameter| parameter.get("closed").and_then(|value| value.as_bool()).unwrap_or(false)).count(),
        "valid_case_count": inputs.iter().filter(|input| input.get("classification").and_then(|value| value.as_str()) == Some("valid")).count(),
        "invalid_case_count": inputs.iter().filter(|input| input.get("classification").and_then(|value| value.as_str()) == Some("invalid")).count(),
        "source_kind_counts": source_kind_counts,
    })
}

fn minimal_stage_view(stage: &VerificationStage) -> serde_json::Value {
    let mut value = serde_json::json!({
        "name": stage.name,
        "status": stage.status,
        "duration_ms": stage.duration_ms,
    });
    if let Some(message) = &stage.message {
        value["message"] =
            serde_json::Value::String(clip_report_text(message, MAX_REPORT_MESSAGE_CHARS));
    }
    if let Some(detail) = &stage.detail {
        let trimmed = match stage.name.as_str() {
            "complexity" => Some(serde_json::json!({
                "threshold": detail.get("threshold").cloned().unwrap_or(serde_json::Value::Null),
                "metric": detail.get("metric").cloned().unwrap_or(serde_json::Value::Null),
                "checked_functions": detail.get("checked_functions").cloned().unwrap_or(serde_json::Value::Null),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "violations": detail.get("violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "suppressed_violations": detail.get("suppressed_violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_functions": detail.get("source_directive_functions").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_suppression_count": detail.get("source_directive_suppression_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
            })),
            "coverage" => Some(serde_json::json!({
                "counts": detail.get("counts").cloned().unwrap_or(serde_json::json!({})),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "seed_input_count": detail.get("seed_input_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seeded_functions": detail.get("seeded_functions").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seed_sources": detail.get("seed_sources").cloned().unwrap_or_else(|| serde_json::json!([])),
                "inferred_context_properties": detail.get("inferred_context_properties").cloned().unwrap_or_else(|| serde_json::json!({})),
                "plan": minimal_plan_counts(detail),
            })),
            "execute" => Some(serde_json::json!({
                "runtime": detail.get("runtime").cloned().unwrap_or(serde_json::Value::Null),
                "skipped": detail.get("skipped").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "reason": detail.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                "generated_cases": detail.get("generated_cases").cloned().unwrap_or(serde_json::Value::Null),
                "valid_invocations": detail.get("valid_invocations").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "evaluated_oracles": detail.get("evaluated_oracles").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "no_inputs_reached": detail.get("no_inputs_reached").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "findings": detail
                    .get("findings")
                    .and_then(|value| value.as_array())
                    .map(|items| items.iter().take(MAX_MINIMAL_FINDINGS_PER_GROUP).cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
                "suppressed_findings": detail
                    .get("suppressed_findings")
                    .and_then(|value| value.as_array())
                    .map(|items| items.iter().take(MAX_MINIMAL_FINDINGS_PER_GROUP).cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
                "findings_summary": detail.get("findings_summary").cloned().unwrap_or_else(|| serde_json::json!({})),
                "plan": minimal_plan_counts(detail),
                "findings_omitted": {
                    "active": detail
                        .get("findings")
                        .and_then(|value| value.as_array())
                        .map(|items| items.len().saturating_sub(MAX_MINIMAL_FINDINGS_PER_GROUP))
                        .unwrap_or(0),
                    "suppressed": detail
                        .get("suppressed_findings")
                        .and_then(|value| value.as_array())
                        .map(|items| items.len().saturating_sub(MAX_MINIMAL_FINDINGS_PER_GROUP))
                        .unwrap_or(0),
                },
            })),
            "test_quality" => Some(serde_json::json!({
                "experimental": false,
                "mode": "advisory",
                "max_mutants": detail.get("max_mutants").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "baseline_eligible": detail.get("baseline_eligible").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "counts": detail.get("counts").cloned().unwrap_or_else(|| serde_json::json!({})),
                "mutants": detail.get("mutants").cloned().unwrap_or_else(|| serde_json::json!([])),
                "coupling_findings": detail.get("coupling_findings").cloned().unwrap_or_else(|| serde_json::json!([])),
                "planning_error": detail.get("planning_error").cloned().unwrap_or(serde_json::Value::Null),
                "coupling_error": detail.get("coupling_error").cloned().unwrap_or(serde_json::Value::Null),
            })),
            "lint" => Some(serde_json::json!({
                "diagnostics": detail.get("diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_diagnostics": detail.get("runner_diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_failed": detail.get("runner_failed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "unavailable": detail.get("unavailable").cloned().unwrap_or(serde_json::Value::Bool(false)),
            })),
            "portability" => Some(serde_json::json!({
                "reason": detail.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                "failing_imports": detail.get("failing_imports").cloned().unwrap_or_else(|| serde_json::json!([])),
                "fix_hint": detail.get("fix_hint").cloned().unwrap_or(serde_json::Value::Null),
                "suppressed": detail.get("suppressed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "repo_runtime": detail.get("repo_runtime").cloned().unwrap_or(serde_json::Value::Null),
                "node_result": serde_json::json!({
                    "stderr": detail
                        .get("node_result")
                        .and_then(|node| node.get("stderr"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }),
            })),
            _ => None,
        };
        if let Some(trimmed) = trimmed {
            value["detail"] = trimmed;
        }
    }
    value
}

pub fn report_json_value(
    report: &VerificationReport,
    report_level: ReportLevel,
) -> serde_json::Value {
    let mut value = match report_level {
        ReportLevel::Minimal => serde_json::json!({
            "schema_version": report.schema_version,
            "tool": report.tool,
            "candidate": report.candidate,
            "verdict": report.verdict,
            "strength": report.strength,
            "summary": report.summary,
            "diagnostics": report.diagnostics,
            "diagnostics_summary": report.diagnostics_summary,
            "report_path": report.report_path,
            "stages": report
                .stages
                .iter()
                .map(minimal_stage_view)
                .collect::<Vec<_>>(),
        }),
        ReportLevel::Full => serde_json::to_value(report).unwrap_or_else(|_| serde_json::json!({})),
    };
    sanitize_report_value(&mut value);
    value
}

fn clip_human(text: &str, limit: usize) -> String {
    let sanitized = sanitize_report_text(text);
    let trimmed = sanitized.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(limit).collect();
    format!("{clipped}...")
}

fn human_number(detail: &serde_json::Value, key: &str) -> usize {
    detail
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct TestQualitySummary {
    pub planned: usize,
    pub killed: usize,
    pub survived: usize,
    pub invalid: usize,
    pub blocked: usize,
    pub no_coverage: usize,
    pub unjudged: usize,
    pub coupling: usize,
}

impl TestQualitySummary {
    pub fn add(&mut self, other: Self) {
        self.planned += other.planned;
        self.killed += other.killed;
        self.survived += other.survived;
        self.invalid += other.invalid;
        self.blocked += other.blocked;
        self.no_coverage += other.no_coverage;
        self.unjudged = self.invalid + self.blocked + self.no_coverage;
        self.coupling += other.coupling;
    }
}

pub fn test_quality_summary(report: &VerificationReport) -> Option<TestQualitySummary> {
    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "test_quality")?
        .detail
        .as_ref()?;
    let counts = detail.get("counts");
    let count = |key: &str| {
        counts
            .and_then(|counts| counts.get(key))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
    };
    let invalid = count("invalid");
    let blocked = count("blocked");
    let no_coverage = count("no_coverage");
    Some(TestQualitySummary {
        planned: count("planned"),
        killed: count("killed"),
        survived: count("survived"),
        invalid,
        blocked,
        no_coverage,
        unjudged: invalid + blocked + no_coverage,
        coupling: detail
            .get("coupling_findings")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
    })
}
fn stage_status_text(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Passed => "PASS",
        StageStatus::Failed => "FAIL",
        StageStatus::Inconclusive => "INCONCLUSIVE",
        StageStatus::Advisory => "ADVISORY",
        StageStatus::Skipped => "SKIPPED",
    }
}

pub fn report_human_summary(report: &VerificationReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Overall: {:?} ({:?})", report.verdict, report.strength);
    if let Some(path) = &report.report_path {
        let _ = writeln!(out, "Report Path: {path}");
    }

    let summary = &report.summary;
    let _ = writeln!(
        out,
        "Coverage: {} analyzed, {} fuzzed, {} skipped, {} module-load blocked",
        summary.functions_analyzed,
        summary.functions_fuzzed,
        summary.functions_skipped,
        summary.functions_blocked_module_load
    );
    let _ = writeln!(
        out,
        "Execute: {} findings ({} gating, {} advisory, {} suppressed)",
        summary.findings.total,
        summary.findings.gating,
        summary.findings.advisory,
        summary.findings.suppressed
    );
    let _ = writeln!(
        out,
        "Lint: {} issues, {} runner failures",
        summary.lint_issues, summary.lint_runner_failures
    );
    let _ = writeln!(
        out,
        "Complexity: {} violations, {} suppressed",
        summary.complexity_violations, summary.suppressed_complexity_violations
    );
    if !report.diagnostics.is_empty() {
        let _ = writeln!(out, "Diagnostics: {}", report.diagnostics.len());
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                out,
                "  {:?}/{:?} ({:?}, {:?}): {}",
                diagnostic.domain,
                diagnostic.kind,
                diagnostic.component,
                diagnostic.impact,
                clip_human(&diagnostic.message, 160)
            );
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "Stages:");
    for stage in &report.stages {
        let mut extra = String::new();
        if let Some(detail) = &stage.detail {
            match stage.name.as_str() {
                "execute" => {
                    let crash = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("crash"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let property = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("property_violation"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let no_inputs = human_number(detail, "no_inputs_reached");
                    extra = format!("crash={crash}, property={property}, no_inputs={no_inputs}");
                }
                "coverage" => {
                    let counts = detail.get("counts").cloned().unwrap_or_default();
                    let checked = [
                        "checked_direct",
                        "checked_via_factory",
                        "checked_via_caller",
                        "checked_via_authoritative_test",
                    ]
                    .iter()
                    .map(|key| {
                        counts
                            .get(*key)
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                    })
                    .sum::<u64>();
                    let factory = counts
                        .get("checked_via_factory")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let reached = counts
                        .get("reached_via_factory")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let skipped = counts
                        .as_object()
                        .map(|obj| {
                            obj.iter()
                                .filter(|(key, _)| {
                                    !key.starts_with("checked_") && *key != "reached_via_factory"
                                })
                                .map(|(_, value)| value.as_u64().unwrap_or(0))
                                .sum()
                        })
                        .unwrap_or(0);
                    extra = format!("checked={checked}, factory={factory}, reached={reached}, skipped={skipped}");
                }
                "test_quality" => {
                    let quality = test_quality_summary(report).unwrap_or_default();
                    extra = format!(
                        "planned={}, killed={}, survived={}, unjudged={}, coupling={}",
                        quality.planned,
                        quality.killed,
                        quality.survived,
                        quality.unjudged,
                        quality.coupling
                    );
                }
                "lint" => {
                    let issues = detail
                        .get("diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let runner_failures = detail
                        .get("runner_diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let unavailable = detail
                        .get("unavailable")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    extra = format!(
                        "issues={issues}, runner_failures={runner_failures}, unavailable={unavailable}"
                    );
                }
                "complexity" => {
                    let violations = detail
                        .get("violations")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let threshold = human_number(detail, "threshold");
                    extra = format!("violations={violations}, threshold={threshold}");
                }
                _ => {}
            }
        }
        let _ = if extra.is_empty() {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms",
                stage.name,
                stage_status_text(stage.status),
                stage.duration_ms
            )
        } else {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms  {}",
                stage.name,
                stage_status_text(stage.status),
                stage.duration_ms,
                extra
            )
        };
        if let Some(message) = &stage.message {
            let _ = writeln!(out, "    {}", clip_human(message, 160));
        }
    }

    if let Some(complexity_stage) = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
    {
        if let Some(violations) = complexity_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("violations"))
            .and_then(|value| value.as_array())
        {
            if !violations.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Complexity Offenders:");
                for (idx, violation) in violations.iter().take(5).enumerate() {
                    let function = violation
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let line = violation
                        .get("line")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cyclomatic = violation
                        .get("complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cognitive = violation
                        .get("cognitive_complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let _ = writeln!(
                        out,
                        "  {}. {} (line {}) cyclomatic={} cognitive={}",
                        idx + 1,
                        function,
                        line,
                        cyclomatic,
                        cognitive
                    );
                }
            }
        }
    }

    if let Some(execute_stage) = report.stages.iter().find(|stage| stage.name == "execute") {
        if let Some(failures) = execute_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("findings"))
            .and_then(|value| value.as_array())
        {
            if !failures.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Execute Findings:");
                for (idx, failure) in failures.iter().take(5).enumerate() {
                    let function = failure
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let severity = failure
                        .get("severity")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown");
                    let message = failure
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let _ = writeln!(
                        out,
                        "  {}. {} [{}] {}",
                        idx + 1,
                        function,
                        severity,
                        clip_human(message, 140)
                    );
                }
            }
        }
    }

    out.trim_end().to_string()
}

fn set_repro_commands(value: &mut serde_json::Value, report_path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            let finding_id = map
                .get("id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            if let (Some(finding_id), Some(repro)) = (
                finding_id,
                map.get_mut("repro").and_then(|value| value.as_object_mut()),
            ) {
                repro.insert(
                    "command".into(),
                    serde_json::Value::String(format!(
                        "court-jester replay --report {report_path} --finding {finding_id}"
                    )),
                );
            }
            for child in map.values_mut() {
                set_repro_commands(child, report_path);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                set_repro_commands(child, report_path);
            }
        }
        _ => {}
    }
}

pub(super) fn write_report(
    output_dir: &str,
    report: &VerificationReport,
    source_file: Option<&str>,
    language: &Language,
    report_level: ReportLevel,
) -> Option<String> {
    use chrono::Utc;

    let _ = std::fs::create_dir_all(output_dir);
    let total_duration = report
        .stages
        .iter()
        .map(|stage| stage.duration_ms)
        .sum::<u64>();

    let now = Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let file_timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();

    let persisted = PersistedReport {
        schema_version: REPORT_SCHEMA_VERSION,
        tool: report.tool.clone(),
        candidate: report.candidate.clone(),
        meta: ReportMeta {
            source_file: source_file.map(|s| s.to_string()),
            language: format!("{:?}", language).to_lowercase(),
            timestamp,
            duration_ms: total_duration,
        },
        stages: match report_level {
            ReportLevel::Full => report.stages.clone(),
            ReportLevel::Minimal => report
                .stages
                .iter()
                .filter_map(|stage| serde_json::from_value(minimal_stage_view(stage)).ok())
                .collect(),
        },
        verdict: report.verdict,
        strength: report.strength,
        summary: report.summary.clone(),
        diagnostics: report.diagnostics.clone(),
        diagnostics_summary: report.diagnostics_summary.clone(),
    };
    let basename = source_file
        .map(|s| {
            std::path::Path::new(s)
                .file_stem()
                .and_then(|os| os.to_str())
                .unwrap_or("inline")
                .to_string()
        })
        .unwrap_or_else(|| "inline".to_string());

    let filename = format!("{file_timestamp}-{basename}.json");
    let path = std::path::Path::new(output_dir).join(&filename);

    let mut json_value = serde_json::to_value(&persisted).ok()?;
    set_repro_commands(&mut json_value, path.to_string_lossy().as_ref());
    sanitize_report_value(&mut json_value);

    match serde_json::to_string_pretty(&json_value) {
        Ok(json) => {
            if std::fs::write(&path, &json).is_ok() {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}
