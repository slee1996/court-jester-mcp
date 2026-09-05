//! Opt-in, CLI-backed regression tests. Positive check evidence is mandatory.

use super::{decisions::build_report, findings_from_stages, load_persisted_report};
use crate::types::*;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct RegressionExportPlan {
    output: PathBuf,
    project: PathBuf,
    source: String,
    report: PersistedReport,
    finding: VerificationFinding,
    accepted_inferred: bool,
}

/// Validate authority and paths before executing the persisted repro or writing anything.
pub fn prepare_regression_export(
    report_path: &str,
    finding_id: &str,
    project_dir: &str,
    output_dir: &str,
    accept_inferred: bool,
) -> Result<RegressionExportPlan, String> {
    let report = load_persisted_report(report_path)?;
    let mut findings = findings_from_stages(&report.stages)
        .into_iter()
        .filter(|finding| finding.id == finding_id);
    let finding = findings.next().ok_or("regression finding was not found")?;
    if findings.next().is_some() {
        return Err("regression finding id is duplicated".into());
    }
    if finding.suppressed || finding.input_classification != InputClassification::Valid {
        return Err(
            "regression export requires an unsuppressed finding with valid input evidence".into(),
        );
    }
    if finding.repro.differential.is_some() || finding.repro.kind == ReproKind::Differential {
        return Err("differential regression export requires a live-candidate check contract and is not supported yet".into());
    }
    if finding.launch_context.is_none() {
        return Err("regression export requires recorded launch context; reverify first".into());
    }
    let inferred = matches!(
        finding.confidence,
        FindingConfidence::Low | FindingConfidence::Medium
    ) || matches!(
        finding.oracle.confidence,
        FindingConfidence::Low | FindingConfidence::Medium
    ) || finding.oracle.provenance == OracleProvenance::NameHeuristic
        || matches!(
            finding.oracle.kind,
            OracleKind::InferredSemantic | OracleKind::GenericProperty
        );
    if inferred && !accept_inferred {
        return Err("this finding uses an inferred expectation; review it and pass --accept-inferred to export it".into());
    }
    let project = std::fs::canonicalize(project_dir)
        .map_err(|error| format!("invalid regression project: {error}"))?;
    if !project.is_dir() {
        return Err("regression project must be a directory".into());
    }
    let source = report
        .meta
        .source_file
        .as_deref()
        .ok_or("regression export requires a source file")?;
    let source = std::fs::canonicalize(project.join(source))
        .map_err(|error| format!("regression source unavailable: {error}"))?;
    if !source.is_file() {
        return Err("regression source must be a file".into());
    }
    let source = source
        .strip_prefix(&project)
        .map_err(|_| "regression source must be inside the dependency project")?
        .to_str()
        .ok_or("regression source path must be UTF-8")?
        .to_string();
    let requested = Path::new(output_dir);
    if std::fs::symlink_metadata(requested).is_ok() {
        return Err("regression output already exists; choose a new directory".into());
    }
    let parent = requested
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("regression output parent must exist: {error}"))?;
    if !parent.starts_with(&project) {
        return Err(
            "regression output must be inside the dependency project for relocation".into(),
        );
    }
    let output = parent.join(
        requested
            .file_name()
            .ok_or("regression output requires a directory name")?,
    );
    Ok(RegressionExportPlan {
        output,
        project,
        source,
        report,
        finding,
        accepted_inferred: inferred && accept_inferred,
    })
}

/// Write a new bundle only after replay proves it supports a positive check outcome.
pub fn write_regression_export(
    mut plan: RegressionExportPlan,
    replay: &ReplayReport,
    launch: ReproLaunchContext,
) -> Result<serde_json::Value, String> {
    if replay.finding_id != plan.finding.id
        || replay.outcome == ReplayOutcome::Inconclusive
        || replay.check_passed.is_none()
    {
        return Err("regression export requires conclusive positive-check evidence; reverify with a current build or resolve the replay blocker".into());
    }
    let language =
        Language::parse(&plan.report.meta.language).ok_or("unsupported regression language")?;
    plan.finding.launch_context = Some(launch);
    plan.finding.repro.command = None;
    plan.finding.location.source_file = plan.source.clone();
    let summary = super::findings_summary(
        std::slice::from_ref(&plan.finding),
        &[],
        InferredOracleGate::Advisory,
    );
    let stage = VerificationStage {
        name: "execute".into(),
        status: StageStatus::Advisory,
        duration_ms: 0,
        detail: Some(
            serde_json::json!({"findings": [plan.finding], "suppressed_findings": [], "findings_summary": summary}),
        ),
        message: Some("Selected regression evidence, not a new verification run".into()),
    };
    let selected = build_report(vec![stage], CoverageGate::None, "", Some(&plan.source));
    plan.report.meta.source_file = Some(plan.source.clone());
    plan.report.candidate.source_file = Some(plan.source.clone());
    plan.report.stages = selected.stages;
    plan.report.summary = selected.summary;
    plan.report.verdict = selected.verdict;
    plan.report.strength = selected.strength;
    plan.report.diagnostics = selected.diagnostics;
    plan.report.diagnostics_summary = selected.diagnostics_summary;
    let levels = plan
        .output
        .strip_prefix(&plan.project)
        .map_err(|_| "invalid regression output")?
        .components()
        .count();
    let (filename, template) = match language {
        Language::Python => ("test_regression.py", include_str!("regression/python.py")),
        Language::TypeScript => ("regression.test.mjs", include_str!("regression/node.mjs")),
    };
    let manifest = serde_json::json!({
        "artifact_schema_version": 1, "artifact_type": "court_jester_regression",
        "finding_id": replay.finding_id, "source_file": plan.source,
        "project_levels": levels, "accepted_inferred": plan.accepted_inferred,
        "export_replay": replay.outcome, "export_check_passed": replay.check_passed,
        "test_file": filename, "requires": "court-jester with replay check_passed support",
    });
    // Atomic directory claim prevents concurrent exports and never overwrites user files.
    // The manifest is written last; interrupted bundles cannot run as passing tests.
    std::fs::create_dir(&plan.output)
        .map_err(|error| format!("cannot claim new regression output: {error}"))?;
    let write = |name: &str, bytes: &[u8]| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(plan.output.join(name))
            .map_err(|error| format!("cannot create regression {name}: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write regression {name}: {error}"))
    };
    write(
        "report.json",
        &serde_json::to_vec_pretty(&plan.report).map_err(|error| error.to_string())?,
    )?;
    write(filename, template.as_bytes())?;
    write("README.md", b"# Court Jester regression\n\nThis test requires Court Jester on PATH (or COURT_JESTER_BINARY pointing to the binary). Run `python3 test_regression.py` for Python or `node --test regression.test.mjs` for TypeScript. It checks the current project source, not an embedded snapshot. Keep this directory at the same relative location when moving the checkout.\n\nThe report retains the finding's original confidence and candidate digest. Explicit acceptance of an inferred expectation is recorded separately; it does not rewrite the finding as authoritative. Runtime profile, limits, and harness arguments are preserved. Repro snippets execute code: review this bundle before running it. Local-trusted is host execution, not isolation.\n\nA pass requires positive completion of the recorded check. A different error, inconclusive replay, missing source/tool, or older replay without check evidence fails the test. This one check is not a specification of the whole API.\n")?;
    write(
        "regression.json",
        &serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )?;
    Ok(
        serde_json::json!({"directory": plan.output, "test_file": filename, "accepted_inferred": plan.accepted_inferred}),
    )
}
