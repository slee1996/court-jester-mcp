//! Changed-file verification and CI report rendering.

use super::args::{read_file, require_base, resolve_complexity_threshold, CliArgs};
use court_jester::tools;
use court_jester::types::{
    DiagnosticImpact, FailureDomain, Language, ReportLevel, SourceMode, StageStatus,
    VerificationReport, VerificationVerdict, DEFAULT_PYTHON_DOCKER_IMAGE,
    DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use tar::Archive;
use tempfile::TempDir;

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

#[derive(Debug)]
struct CiPreparedFile {
    pub(super) relative_path: String,
    pub(super) language: Language,
    pub(super) absolute_string: String,
    pub(super) code: String,
    pub(super) candidate_count: usize,
    test_entrypoint: Option<CiTestEntrypoint>,
}

#[derive(Debug, Clone)]
pub(super) struct CiFileResult {
    pub(super) file: String,
    pub(super) language: Language,
    pub(super) verdict: VerificationVerdict,
    pub(super) failing_gates: Vec<String>,
    pub(super) report: VerificationReport,
}

#[derive(Debug, Clone)]
pub(super) struct CiTestEntrypoint {
    pub(super) source_file: String,
    pub(super) code: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CiTestEntrypoints {
    pub(super) python: Option<CiTestEntrypoint>,
    pub(super) typescript: Option<CiTestEntrypoint>,
}

impl CiTestEntrypoints {
    fn for_language(&self, language: Language) -> Option<&CiTestEntrypoint> {
        match language {
            Language::Python => self.python.as_ref(),
            Language::TypeScript => self.typescript.as_ref(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct CiTestQualitySummary {
    pub(super) max_mutants: usize,
    #[serde(flatten)]
    pub(super) counts: tools::verify::TestQualitySummary,
}

pub(super) fn ci_test_quality_summary(
    max_mutants: usize,
    summaries: impl IntoIterator<Item = tools::verify::TestQualitySummary>,
) -> CiTestQualitySummary {
    let mut counts = tools::verify::TestQualitySummary::default();
    for summary in summaries {
        counts.add(summary);
    }
    CiTestQualitySummary {
        max_mutants,
        counts,
    }
}

#[derive(Debug, Clone)]
pub(super) struct CiRunResult {
    pub(super) base: String,
    pub(super) head: String,
    pub(super) gates: Vec<String>,
    pub(super) changed_files: usize,
    pub(super) checked_files: usize,
    pub(super) skipped_files: Vec<String>,
    pub(super) files: Vec<CiFileResult>,
    pub(super) test_quality: Option<CiTestQualitySummary>,
    pub(super) verdict: VerificationVerdict,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CiJsonFileResult {
    pub(super) file: String,
    pub(super) language: String,
    pub(super) verdict: VerificationVerdict,
    pub(super) failing_gates: Vec<String>,
    pub(super) report: serde_json::Value,
}

pub(super) fn parse_ci_gates(raw: Option<&str>) -> Result<Vec<String>, String> {
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

fn ci_test_language(path: &str) -> Result<Language, String> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("py") => Ok(Language::Python),
        Some("ts") | Some("tsx") => Ok(Language::TypeScript),
        _ => Err(format!(
            "CI --test-file must end in .py, .ts, or .tsx, got '{}'",
            path
        )),
    }
}

pub(super) fn ci_test_entrypoints(
    repo_dir: &Path,
    paths: &[String],
) -> Result<CiTestEntrypoints, String> {
    let mut python_path: Option<&str> = None;
    let mut typescript_path: Option<&str> = None;
    for path in paths {
        let (language, slot) = match ci_test_language(path)? {
            Language::Python => (Language::Python, &mut python_path),
            Language::TypeScript => (Language::TypeScript, &mut typescript_path),
        };
        if let Some(existing) = *slot {
            return Err(format!(
                "`ci --test-quality` accepts at most one {} --test-file; got '{}' and '{}'",
                ci_language_name(&language),
                existing,
                path
            ));
        }
        *slot = Some(path);
    }

    let load = |path: &str| -> Result<CiTestEntrypoint, String> {
        let source_path = {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                repo_dir.join(path)
            }
        };
        let source_file = source_path.to_string_lossy().into_owned();
        Ok(CiTestEntrypoint {
            code: read_file(&source_file)?,
            source_file,
        })
    };
    Ok(CiTestEntrypoints {
        python: python_path.map(load).transpose()?,
        typescript: typescript_path.map(load).transpose()?,
    })
}

fn ci_source_mode(path: &str, language: Language) -> SourceMode {
    if language == Language::Python {
        return SourceMode::Python;
    }
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("tsx") | Some("jsx") => SourceMode::Tsx,
        _ => SourceMode::TypeScript,
    }
}

pub(super) fn ci_quality_allocations(max_mutants: usize, candidate_counts: &[usize]) -> Vec<usize> {
    let mut allocations = vec![0; candidate_counts.len()];
    let mut remaining = max_mutants;
    while remaining > 0 {
        let mut allocated = false;
        for (index, capacity) in candidate_counts.iter().copied().enumerate() {
            if allocations[index] >= capacity {
                continue;
            }
            allocations[index] += 1;
            remaining -= 1;
            allocated = true;
            if remaining == 0 {
                break;
            }
        }
        if !allocated {
            break;
        }
    }
    allocations
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
    files.sort_by(|left, right| left.0.cmp(&right.0));
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
pub(super) async fn run_ci_for_repo(
    repo_dir: &Path,
    args: &CliArgs,
) -> Result<CiRunResult, String> {
    if args.file.is_some() || args.language.is_some() {
        return Err("`court-jester ci` does not accept --file or --language".into());
    }
    if args.tests_only {
        return Err("`court-jester ci` does not support --tests-only".into());
    }
    if args.test_quality_max_mutants.is_some()
        && args.test_files.is_empty()
        && args.config_targets.is_empty()
    {
        return Err(
            "`court-jester ci --test-quality` requires an authoritative --test-file".into(),
        );
    }
    let base = require_base(args)?.to_string();
    let head = args.head.clone().unwrap_or_else(|| "HEAD".into());
    let suppressions = super::args::read_suppressions(args.suppressions_file.as_deref())?;
    let test_entrypoints = ci_test_entrypoints(repo_dir, &args.test_files)?;
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
    let diff = (!diff.is_empty()).then_some(diff);
    let complexity_threshold = resolve_complexity_threshold(args)?;
    let mut prepared_files = Vec::new();
    let mut skipped_files = Vec::new();
    for (relative_path, language) in &changed_files {
        let absolute = repo_dir.join(relative_path);
        if !absolute.is_file() {
            skipped_files.push(relative_path.clone());
            continue;
        }
        let absolute_string = absolute.to_string_lossy().into_owned();
        let code = read_file(&absolute_string)?;
        let test_entrypoint =
            if let Some(tests) = super::config::mapped_tests(&args.config_targets, &absolute)? {
                let mapped = ci_test_entrypoints(repo_dir, tests)?;
                Some(
                    mapped.for_language(*language).cloned().ok_or(
                        "configured source mapping has no test entrypoint for its language",
                    )?,
                )
            } else {
                test_entrypoints.for_language(*language).cloned()
            };
        let candidate_count =
            if args.test_quality_max_mutants.is_some() && test_entrypoint.is_some() {
                tools::verify::test_quality_candidate_count(
                    &code,
                    language,
                    ci_source_mode(relative_path, *language),
                    Some(absolute_string.as_str()),
                    diff.as_deref(),
                )
                // Allocation is advisory; the per-file stage owns and reports planning errors.
                .unwrap_or(0)
            } else {
                0
            };
        prepared_files.push(CiPreparedFile {
            relative_path: relative_path.clone(),
            language: *language,
            absolute_string,
            code,
            candidate_count,
            test_entrypoint,
        });
    }
    let quality_allocations = args
        .test_quality_max_mutants
        .map(|max_mutants| {
            ci_quality_allocations(
                max_mutants,
                &prepared_files
                    .iter()
                    .map(|file| file.candidate_count)
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_else(|| vec![0; prepared_files.len()]);

    let project_dir = args
        .project_dir
        .clone()
        .or_else(|| Some(repo_dir.to_string_lossy().into_owned()));
    let mut files = Vec::new();
    let mut verdict = VerificationVerdict::Pass;
    for (file_index, prepared) in prepared_files.into_iter().enumerate() {
        let baseline_path = baseline_temp.path().join(&prepared.relative_path);
        let baseline_code = baseline_path
            .is_file()
            .then(|| read_file(&baseline_path.to_string_lossy()))
            .transpose()?;
        let test_entrypoint = prepared.test_entrypoint.as_ref();
        let test_quality_max_mutants = args.test_quality_max_mutants.map(|_| {
            test_entrypoint
                .map(|_| quality_allocations[file_index])
                .unwrap_or(0)
        });
        let report = tools::verify::verify(
            &prepared.code,
            &prepared.language,
            tools::verify::VerifyOptions {
                test_code: test_entrypoint.map(|entrypoint| entrypoint.code.as_str()),
                test_source_file: test_entrypoint.map(|entrypoint| entrypoint.source_file.as_str()),
                test_runner: args.test_runner,
                test_quality_max_mutants,
                complexity_threshold,
                complexity_metric: args.complexity_metric,
                project_dir: project_dir.as_deref(),
                lint_config_path: args.config_path.as_deref(),
                lint_virtual_file_path: None,
                diff: diff.as_deref(),
                suppressions: suppressions.as_deref(),
                suppression_source: args.suppressions_file.as_deref(),
                auto_seed: !args.no_auto_seed,
                base_code: baseline_code.as_deref(),
                base_source_file: baseline_path.to_str(),
                base_project_dir: baseline_temp.path().to_str(),
                source_file: Some(prepared.absolute_string.as_str()),
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
                tests_only: false,
            },
        )
        .await;
        let failing_gates = ci_stage_failures(&report, &gates);
        let file_verdict = ci_selected_verdict(&report, &gates);
        verdict = aggregate_verdict(verdict, file_verdict);
        files.push(CiFileResult {
            file: prepared.relative_path,
            language: prepared.language,
            verdict: file_verdict,
            failing_gates,
            report,
        });
    }

    if !skipped_files.is_empty() {
        verdict = VerificationVerdict::Inconclusive;
    }
    let test_quality = args.test_quality_max_mutants.map(|max_mutants| {
        ci_test_quality_summary(
            max_mutants,
            files
                .iter()
                .filter_map(|file| tools::verify::test_quality_summary(&file.report)),
        )
    });
    Ok(CiRunResult {
        base,
        head,
        gates,
        changed_files: changed_files.len(),
        checked_files: files.len(),
        skipped_files,
        files,
        test_quality,
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

pub(super) fn verdict_label(verdict: &VerificationVerdict) -> &'static str {
    match verdict {
        VerificationVerdict::Pass => "PASS",
        VerificationVerdict::Fail => "FAIL",
        VerificationVerdict::Inconclusive => "INCONCLUSIVE",
    }
}

fn ci_test_quality_brief(summary: &CiTestQualitySummary) -> String {
    format!(
        "planned={}, killed={}, survived={}, unjudged={}, coupling={}",
        summary.counts.planned,
        summary.counts.killed,
        summary.counts.survived,
        summary.counts.unjudged,
        summary.counts.coupling
    )
}

pub(super) fn render_ci_human(result: &CiRunResult) -> String {
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
    if let Some(summary) = &result.test_quality {
        out.push_str(&format!(
            "Test quality: {}\n",
            ci_test_quality_brief(summary)
        ));
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
pub(super) fn render_ci_github(result: &CiRunResult) -> String {
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
    if let Some(summary) = &result.test_quality {
        lines.push(format!(
            "court-jester test quality: {}",
            ci_test_quality_brief(summary)
        ));
    }
    lines.push(format!(
        "court-jester ci: {} ({} checked file(s), gates: {})",
        verdict_label(&result.verdict),
        result.checked_files,
        result.gates.join(", ")
    ));
    lines.join("\n")
}

pub(super) fn ci_json_value(result: &CiRunResult, report_level: ReportLevel) -> serde_json::Value {
    let mut value = serde_json::json!({
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
    });
    if let Some(summary) = &result.test_quality {
        value["test_quality"] =
            serde_json::to_value(summary).expect("test-quality summary serialization cannot fail");
    }
    value
}
