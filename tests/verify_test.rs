use court_jester::tools::verify::{
    final_verdict, load_persisted_report, parse_findings, repair_summary, replay_report,
    report_human_summary, report_json_value, verify, VerifyOptions,
};
use court_jester::types::MinimizationStatus;
use court_jester::types::{
    CandidateProvenance, ComplexityMetric, CoverageGate, CoverageSummary, DiagnosticComponent,
    DiagnosticImpact, ExecuteGate, FailureDiagnostic, FailureDomain, FailureKind, FindingCategory,
    FindingConfidence, FindingSeverity, FindingsSummary, FuzzFunctionCoverage, FuzzFunctionStatus,
    InferredOracleGate, InputClassification, Language, NetworkPolicy, OracleKind, OracleProvenance,
    ReplayOutcome, ReportLevel, ReportSummary, RuntimeProfile, StageStatus, TestRunner,
    ToolProvenance, VerificationEvidence, VerificationReport, VerificationStage,
    VerificationStrength, VerificationVerdict, DEFAULT_BUN_DOCKER_IMAGE,
    DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn install_fake_tool_at(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script_path = dir.join(name);
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&script_path, body).unwrap();
    #[cfg(unix)]
    make_executable(&script_path);
    script_path
}

fn normalize_logged_path(path: &str) -> String {
    path.trim()
        .strip_prefix("/private")
        .unwrap_or(path.trim())
        .to_string()
}

fn assert_log_contains_path(log: &str, prefix: &str, expected: &Path) {
    let expected = normalize_logged_path(&expected.to_string_lossy());
    assert!(
        log.lines().any(|line| {
            line.strip_prefix(prefix)
                .map(normalize_logged_path)
                .as_deref()
                == Some(expected.as_str())
        }),
        "expected log to contain {prefix}{expected}, got:\n{log}"
    );
}

#[tokio::test]
async fn python_named_inputs_reach_the_target_as_instances() {
    let code = "from dataclasses import dataclass\n@dataclass\nclass Payload:\n    value: int\ndef read(value: Payload) -> int:\n    return value.value\n";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "{}",
        report_human_summary(&report)
    );
    assert_eq!(report.summary.findings.total, 0);
    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .unwrap()
        .detail
        .as_ref()
        .unwrap();
    assert!(detail["valid_invocations"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn python_property_replay_handles_containers_and_abstains_on_runtime_values() {
    for (declarations, annotation, expected) in [
        ("", "tuple[int, ...]", ReplayOutcome::Reproduced),
        ("", "set[int]", ReplayOutcome::Reproduced),
        (
            "from typing import Callable\n",
            "Callable[[int], int]",
            ReplayOutcome::Inconclusive,
        ),
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("target.py");
        let code =
            format!("{declarations}\ndef label(value: {annotation}) -> str:\n    return 42\n");
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.source_file = source.to_str();
        opts.project_dir = project.path().to_str();
        let report = verify(&code, &Language::Python, opts).await;
        let repair = repair_summary(&report, &Language::Python);
        let finding = repair
            .findings
            .iter()
            .find(|finding| finding.severity == FindingSeverity::PropertyViolation)
            .unwrap_or_else(|| panic!("{annotation}: {}", report_human_summary(&report)));
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        let replay = replay_report(
            path.to_str().unwrap(),
            &finding.id,
            None,
            RuntimeProfile::LocalTrusted,
            DEFAULT_PYTHON_DOCKER_IMAGE,
            DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        )
        .await
        .unwrap();
        assert_eq!(replay.outcome, expected, "{annotation}: {replay:?}");
        if expected == ReplayOutcome::Reproduced {
            fs::write(
                &source,
                format!("def label(value: {annotation}) -> str:\n    return 'fixed'\n"),
            )
            .unwrap();
            let replay = replay_report(
                path.to_str().unwrap(),
                &finding.id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(
                replay.outcome,
                ReplayOutcome::NotReproduced,
                "{annotation}: {replay:?}"
            );
        }
    }
}

#[tokio::test]
async fn python_exception_replay_does_not_match_an_unrelated_same_class_error() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let code = "def inspect(*, value: str) -> str:\n    raise ValueError('original rejection')\n";
    fs::write(&source, code).unwrap();
    let mut opts = default_opts(None);
    opts.source_file = source.to_str();
    opts.project_dir = project.path().to_str();
    let report = verify(code, &Language::Python, opts).await;
    let repair = repair_summary(&report, &Language::Python);
    let finding = repair.findings.first().expect("exception observation");
    let path = project.path().join("repair.json");
    fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
    for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
        if expected == ReplayOutcome::NotReproduced {
            fs::write(
                &source,
                "def inspect(*, value: str) -> str:\n    raise ValueError('different rejection')\n",
            )
            .unwrap();
        }
        let replay = replay_report(
            path.to_str().unwrap(),
            &finding.id,
            None,
            RuntimeProfile::LocalTrusted,
            DEFAULT_PYTHON_DOCKER_IMAGE,
            DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        )
        .await
        .unwrap();
        assert_eq!(replay.outcome, expected, "{replay:?}");
    }
}

#[tokio::test]
async fn python_property_replay_preserves_nonfinite_inputs() {
    for predicate in [
        "math.isnan(value)",
        "value == float('inf')",
        "value == float('-inf')",
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("target.py");
        let code = format!("import math\ndef metric(value: float) -> float:\n    if {predicate}:\n        return 'wrong'\n    return value\n");
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.source_file = source.to_str();
        opts.project_dir = project.path().to_str();
        let report = verify(&code, &Language::Python, opts).await;
        let repair = repair_summary(&report, &Language::Python);
        let finding = repair
            .findings
            .iter()
            .find(|finding| finding.severity == FindingSeverity::PropertyViolation)
            .expect("nonfinite property finding");
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
            if expected == ReplayOutcome::NotReproduced {
                fs::write(
                    &source,
                    "def metric(value: float) -> float:\n    return value\n",
                )
                .unwrap();
            }
            let replay = replay_report(
                path.to_str().unwrap(),
                &finding.id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(replay.outcome, expected, "{predicate}: {replay:?}");
        }
    }
}

#[tokio::test]
async fn python_assertions_do_not_impersonate_declared_property_checks() {
    for body in ["raise AssertionError('Not idempotent: copied diagnostic')", "global calls\n    calls += 1\n    if calls % 2 == 0:\n        raise AssertionError('Not idempotent: copied diagnostic')\n    return value"] {
        let code = format!("calls = 0\n# court-jester-properties idempotent\ndef echo(value: str) -> str:\n    {body}\n");
        let report = verify(&code, &Language::Python, default_opts(None)).await;
        assert_eq!(report.verdict, VerificationVerdict::Inconclusive, "{}", report_human_summary(&report));
        let repair = repair_summary(&report, &Language::Python);
        assert!(!repair.findings.is_empty());
        assert!(repair.findings.iter().all(|finding| finding.input_classification == InputClassification::Unknown && finding.category == FindingCategory::Exception));
        assert_eq!(report.summary.findings.gating, 0);
    }
    let report = verify(
        "# court-jester-properties bounded\ndef grow(value: str) -> str:\n    return 42\n",
        &Language::Python,
        default_opts(None),
    )
    .await;
    let repair = repair_summary(&report, &Language::Python);
    let finding = repair.findings.first().expect("return-type finding");
    assert_eq!(finding.oracle.kind, OracleKind::GenericProperty);
    assert_eq!(finding.confidence, FindingConfidence::Medium);
}

#[tokio::test]
async fn python_property_replay_repeats_the_recorded_check() {
    for (directive, signature, buggy, repaired) in [
        (
            "bounded",
            "grow(value: str) -> str",
            "return value + '!'",
            "return value",
        ),
        (
            "idempotent",
            "normalize(value: str) -> str",
            "return value + '!'",
            "return value.strip()",
        ),
        (
            "involution",
            "flip(value: str) -> str",
            "return value + '!'",
            "return value[::-1]",
        ),
        (
            "monotonic",
            "scale(value: int) -> int",
            "return -value",
            "return value",
        ),
        (
            "order_invariant",
            "summarize(values: list[int]) -> int",
            "return values[0] if values else 0",
            "return len(values)",
        ),
        (
            "nonneg",
            "score(value: int) -> int",
            "return -1",
            "return abs(value)",
        ),
        (
            "permutation",
            "keep(values: list[int]) -> list[int]",
            "return []",
            "return list(values)",
        ),
        (
            "clamped",
            "clamp(value: int, lo: int, hi: int) -> int",
            "return hi + 1",
            "return min(max(value, min(lo, hi)), max(lo, hi))",
        ),
        (
            "symmetric",
            "combine(left: int, right: int) -> int",
            "return left - right",
            "return left + right",
        ),
        (
            "no_nullish_string",
            "serialize(value: dict[str, str | None]) -> str",
            "return ','.join(map(str, value.values()))",
            "return ','.join(str(item) for item in value.values() if item is not None)",
        ),
        (
            "sorted",
            "arrange(values: list[int]) -> list[int]",
            "return list(reversed(values))",
            "return sorted(values)",
        ),
        (
            "antisymmetric",
            "compare_values(left: int, right: int) -> int",
            "return 1",
            "return (left > right) - (left < right)",
        ),
        (
            "palindrome",
            "mirror(value: str) -> str",
            "return 'ab'",
            "return value + value[::-1]",
        ),
        ("", "label(value: str) -> str", "return 42", "return value"),
    ] {
        for keyword_only in [false, true] {
            let signature = if keyword_only {
                signature.replacen('(', "(*, ", 1)
            } else {
                signature.to_owned()
            };
            let project = tempfile::tempdir().unwrap();
            let source = project.path().join("target.py");
            let code =
                format!("# court-jester-properties {directive}\ndef {signature}:\n    {buggy}\n");
            fs::write(&source, &code).unwrap();
            let mut opts = default_opts(None);
            opts.source_file = source.to_str();
            opts.project_dir = project.path().to_str();
            let report = verify(&code, &Language::Python, opts).await;
            let repair = repair_summary(&report, &Language::Python);
            let finding = repair
                .findings
                .iter()
                .find(|finding| finding.severity == FindingSeverity::PropertyViolation)
                .unwrap_or_else(|| {
                    panic!("missing {directive}: {}", report_human_summary(&report))
                });
            assert_eq!(finding.minimization.status, MinimizationStatus::Preserved);
            let path = project.path().join("repair.json");
            fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
            for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
                if expected == ReplayOutcome::NotReproduced {
                    fs::write(&source, format!("def {signature}:\n    {repaired}\n")).unwrap();
                }
                let replay = replay_report(
                    path.to_str().unwrap(),
                    &finding.id,
                    None,
                    RuntimeProfile::LocalTrusted,
                    DEFAULT_PYTHON_DOCKER_IMAGE,
                    DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
                )
                .await
                .unwrap();
                assert_eq!(replay.outcome, expected, "{directive}: {replay:?}");
            }
            fs::write(
                &source,
                format!(
                    "def {signature}:\n    raise AssertionError({})\n",
                    serde_json::to_string(&finding.message).unwrap()
                ),
            )
            .unwrap();
            let replay = replay_report(
                path.to_str().unwrap(),
                &finding.id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(
                replay.outcome,
                ReplayOutcome::NotReproduced,
                "target assertion must not impersonate {directive}: {replay:?}"
            );
            if directive == "idempotent" {
                fs::write(&source, format!("calls = 0\nclass CopiedFailure(AssertionError):\n    oracle_id = 'idempotent'\nCopiedFailure.__name__ = '_PropertyFailure'\ndef {signature}:\n    global calls\n    calls += 1\n    if calls % 2 == 0:\n        raise CopiedFailure({})\n    return value\n", serde_json::to_string(&finding.message).unwrap())).unwrap();
                let replay = replay_report(
                    path.to_str().unwrap(),
                    &finding.id,
                    None,
                    RuntimeProfile::LocalTrusted,
                    DEFAULT_PYTHON_DOCKER_IMAGE,
                    DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
                )
                .await
                .unwrap();
                assert_eq!(replay.outcome, ReplayOutcome::NotReproduced, "repeat-call exception must not impersonate typed property evidence: {replay:?}");
            }
        }
    }
}

#[tokio::test]
async fn typescript_nullish_domains_do_not_discard_allowed_undefined_exceptions() {
    for parameter in ["flag: boolean | undefined", "flag?: boolean"] {
        let code = format!("export function choose({parameter}): boolean {{ if (flag === undefined) throw new Error('missing undefined branch'); return flag === true; }}");
        let report = verify(&code, &Language::TypeScript, default_opts(None)).await;
        assert_eq!(
            report.verdict,
            VerificationVerdict::Fail,
            "{}",
            report_human_summary(&report)
        );
        assert!(repair_summary(&report, &Language::TypeScript)
            .findings
            .iter()
            .any(
                |finding| finding.message.contains("missing undefined branch")
                    && finding.input_classification == InputClassification::Valid
            ));
        let clean = format!("export function choose({parameter}): boolean {{ if (flag === null) throw new Error('null is outside this contract'); return flag === true; }}");
        let clean_report = verify(&clean, &Language::TypeScript, default_opts(None)).await;
        assert_eq!(
            clean_report.verdict,
            VerificationVerdict::Pass,
            "{}",
            report_human_summary(&clean_report)
        );
        assert_eq!(clean_report.summary.findings.total, 0);
    }
}

#[tokio::test]
async fn primitive_typescript_exception_observations_replay_exactly() {
    for (thrown, different) in [
        ("'plain failure'", "'different failure'"),
        ("null", "undefined"),
        ("undefined", "null"),
        ("NaN", "Infinity"),
        ("-0", "0"),
        ("17", "18"),
        ("17n", "18n"),
        ("true", "false"),
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("target.ts");
        let code = format!("export function inspect(value: string): string {{ throw {thrown}; }}");
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.source_file = source.to_str();
        opts.project_dir = project.path().to_str();
        let report = verify(&code, &Language::TypeScript, opts).await;
        assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
        let repair = repair_summary(&report, &Language::TypeScript);
        let finding = repair.findings.first().expect("retained exception");
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
            if expected == ReplayOutcome::NotReproduced {
                fs::write(
                    &source,
                    format!(
                        "export function inspect(value: string): string {{ throw {different}; }}"
                    ),
                )
                .unwrap();
            }
            let replay = replay_report(
                path.to_str().unwrap(),
                &finding.id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(replay.outcome, expected, "{thrown}: {replay:?}");
        }
    }
}

#[tokio::test]
async fn runtime_only_typescript_thrown_values_abstain_from_replay() {
    for thrown in [
        "({reason: 'opaque'})",
        "Symbol('opaque')",
        "(() => undefined)",
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("target.ts");
        let code = format!("export function inspect(value: string): string {{ throw {thrown}; }}");
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.source_file = source.to_str();
        opts.project_dir = project.path().to_str();
        let report = verify(&code, &Language::TypeScript, opts).await;
        let repair = repair_summary(&report, &Language::TypeScript);
        let finding = repair.findings.first().expect("retained exception");
        assert_eq!(finding.input_classification, InputClassification::Unknown);
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        let replay = replay_report(
            path.to_str().unwrap(),
            &finding.id,
            None,
            RuntimeProfile::LocalTrusted,
            DEFAULT_PYTHON_DOCKER_IMAGE,
            DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        )
        .await
        .unwrap();
        assert_eq!(
            replay.outcome,
            ReplayOutcome::Inconclusive,
            "{thrown}: {replay:?}"
        );
    }
}

#[tokio::test]
async fn target_errors_during_property_evaluation_do_not_impersonate_checks() {
    let code = "let calls = 0;\n// court-jester-properties idempotent\nexport function echo(value: string): string { if (++calls % 2 === 0) throw new Error('Not idempotent: copied diagnostic'); return value; }";
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "{}",
        report_human_summary(&report)
    );
    let repair = repair_summary(&report, &Language::TypeScript);
    assert!(!repair.findings.is_empty());
    assert!(repair
        .findings
        .iter()
        .all(
            |finding| finding.input_classification == InputClassification::Unknown
                && finding.category == FindingCategory::Exception
        ));
    assert_eq!(report.summary.findings.gating, 0);
}

#[tokio::test]
async fn unclassified_typescript_exceptions_remain_observations_under_strict_gating() {
    for exception in [
        "new Error('unspecified contract')",
        "new RangeError('unspecified contract')",
        "'plain thrown value'",
        "new Error('Return type mismatch: copied diagnostic')",
    ] {
        let code =
            format!("export function inspect(value: string): string {{ throw {exception}; }}");
        let mut opts = default_opts(None);
        opts.inferred_oracle_gate = InferredOracleGate::Fail;
        let report = verify(&code, &Language::TypeScript, opts).await;
        assert_eq!(
            report.verdict,
            VerificationVerdict::Inconclusive,
            "{}",
            report_human_summary(&report)
        );
        let repair = repair_summary(&report, &Language::TypeScript);
        assert_eq!(repair.recommended_action, "add_contract_or_test");
        assert!(
            !repair.findings.is_empty(),
            "exception observation was discarded: {exception}"
        );
        assert!(repair
            .findings
            .iter()
            .all(|finding| finding.input_classification == InputClassification::Unknown));
        assert_eq!(report.summary.findings.gating, 0);
        assert!(report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::NonzeroExit));
    }
}

#[tokio::test]
async fn closed_keyword_domain_uses_bound_slot_after_variadic_arguments() {
    let code = "def label(value: int, *values: int, mode: bool) -> str:\n    if values:\n        raise ValueError('valid keyword, unknown state contract')\n    return str(mode)\n";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "{}",
        report_human_summary(&report)
    );
    assert!(
        repair_summary(&report, &Language::Python)
            .findings
            .iter()
            .any(|finding| finding.error_type.as_deref() == Some("ValueError")),
        "a variadic element must not be checked against the keyword's finite domain"
    );
}

#[tokio::test]
async fn admitted_python_failure_is_not_hidden_by_an_unclassified_exception() {
    let code = "from typing import Literal\ndef broken(value: Literal['ready']) -> str:\n    raise ValueError('admitted failure')\ndef uncertain(value: str) -> str:\n    raise RuntimeError('unknown contract')\n";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert_eq!(
        report.verdict,
        VerificationVerdict::Fail,
        "{}",
        report_human_summary(&report)
    );
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.domain == FailureDomain::TargetCode
            && diagnostic.impact == DiagnosticImpact::Gating));
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == FailureKind::AmbiguousGeneratedInput));
}

#[tokio::test]
async fn unclassified_python_exceptions_remain_observations_under_strict_gating() {
    for error in ["ValueError", "RuntimeError", "DomainFailure"] {
        let code = format!("class DomainFailure(Exception):\n    pass\ndef inspect(value: str) -> str:\n    raise {error}('unspecified input contract')\n");
        let mut opts = default_opts(None);
        opts.inferred_oracle_gate = InferredOracleGate::Fail;
        let report = verify(&code, &Language::Python, opts).await;
        assert_eq!(
            report.verdict,
            VerificationVerdict::Inconclusive,
            "{error}: {}",
            report_human_summary(&report)
        );
        let repair = repair_summary(&report, &Language::Python);
        assert_eq!(repair.recommended_action, "add_contract_or_test");
        assert!(repair
            .findings
            .iter()
            .any(|finding| finding.error_type.as_deref() == Some(error)
                && finding.input_classification == InputClassification::Unknown));
        assert_eq!(report.summary.findings.gating, 0);
        assert!(report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::NonzeroExit));
    }
}

#[tokio::test]
async fn property_strength_counts_observed_checks_even_without_return_annotations() {
    for (language, clean, buggy) in [
        (
            Language::Python,
            "def echo(value: int):\n    return value\n",
            "def echo(value: int) -> int:\n    return 'wrong'\n",
        ),
        (
            Language::TypeScript,
            "export function echo(value: number) { return value; }",
            "export function echo(value: number): number { return 'wrong' as unknown as number; }",
        ),
    ] {
        for (code, expect_failure) in [(clean, false), (buggy, true)] {
            let report = verify(code, &language, default_opts(None)).await;
            assert_eq!(
                report.strength,
                VerificationStrength::PropertyChecked,
                "{language:?}: {}",
                report_human_summary(&report)
            );
            let detail = report
                .stages
                .iter()
                .find(|stage| stage.name == "execute")
                .and_then(|stage| stage.detail.as_ref())
                .unwrap();
            let surfaces = detail["harness_events"]["surfaces"].as_object().unwrap();
            let passed: u64 = surfaces
                .values()
                .map(|surface| surface["passed_oracles"].as_u64().unwrap())
                .sum();
            let failed: u64 = surfaces
                .values()
                .map(|surface| surface["failed_oracles"].as_u64().unwrap())
                .sum();
            assert_eq!(detail["evaluated_oracles"], passed + failed);
            assert!(passed + failed > 0);
            assert_eq!(failed > 0, expect_failure);
            if expect_failure {
                assert_eq!(passed, 0);
                assert_eq!(
                    detail["valid_invocations"], failed,
                    "minimization must not inflate campaign check counts"
                );
            }
            assert_eq!(
                report.verdict,
                if expect_failure {
                    VerificationVerdict::Fail
                } else {
                    VerificationVerdict::Pass
                }
            );
        }
    }
}

#[tokio::test]
async fn annotations_without_executed_checks_do_not_supply_property_strength() {
    for (language, code) in [
        (
            Language::Python,
            "import time\ndef snapshot(value: int) -> dict:\n    return {'value': value, 'time': time.time()}\n",
        ),
        (
            Language::TypeScript,
            "export function snapshot(value: number): { value: number, time: number } { return { value, time: Date.now() }; }",
        ),
    ] {
        let report = verify(code, &language, default_opts(None)).await;
        assert_eq!(
            report.verdict,
            VerificationVerdict::Pass,
            "{language:?}: {}",
            report_human_summary(&report)
        );
        assert_eq!(
            report.strength,
            VerificationStrength::RuntimeSmoke,
            "{language:?}: an unsupported return annotation is not an executed check"
        );
    }
}

#[tokio::test]
async fn typescript_property_replay_repeats_the_recorded_check() {
    for (directive, signature, buggy, repaired) in [
        ("bounded", "grow(value: string): string", "return value + '!';", "return value;"),
        ("idempotent", "normalize(value: string): string", "return value + '!';", "return value.trim();"),
        ("involution", "flip(value: string): string", "return value + '!';", "return value.split('').reverse().join('');"),
        ("monotonic", "scale(value: number): number", "return -value;", "return value;"),
        ("order_invariant", "summarize(values: number[]): number", "return values[0] ?? 0;", "return values.length;"),
        ("nonneg", "score(value: number): number", "return -1;", "return Math.abs(value);"),
        ("nonempty_string", "displayLabel(value: string): string", "return '';", "return value.trim() || 'unnamed';"),
        ("permutation", "keep(values: number[]): number[]", "return [];", "return [...values];"),
        ("clamped", "clamp(value: number, lo: number, hi: number): number", "return hi + 1;", "return Math.min(Math.max(value, Math.min(lo, hi)), Math.max(lo, hi));"),
        ("symmetric", "combine(left: number, right: number): number", "return left - right;", "return left + right;"),
        ("no_nullish_string", "serialize(value: Record<string, unknown>): string", "return Object.values(value).map(String).join(',');", "return Object.values(value).filter(item => item != null).map(String).join(',');"),
        ("sorted", "arrange(values: number[]): number[]", "return [...values].reverse();", "return [...values].sort((a, b) => a - b);"),
        ("antisymmetric", "compareValues(left: number, right: number): number", "return 1;", "return Object.is(left, right) ? 0 : Number.isNaN(left) ? 1 : Number.isNaN(right) ? -1 : left < right ? -1 : left > right ? 1 : 0;"),
        ("", "label(value: string): string", "return 42 as unknown as string;", "return value;"),
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join("target.ts");
        let code = format!("// court-jester-properties {directive}\nexport function {signature} {{ {buggy} }}");
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.source_file = source.to_str();
        opts.project_dir = project.path().to_str();
        let report = verify(&code, &Language::TypeScript, opts).await;
        let execute = report.stages.iter().find(|stage| stage.name == "execute")
            .and_then(|stage| stage.detail.as_ref()).unwrap();
        assert_eq!(execute["harness_events"]["harness_completed"], true, "large property reports must drain before process exit");
        let repair = repair_summary(&report, &Language::TypeScript);
        let finding = repair.findings.iter().find(|finding| finding.severity == FindingSeverity::PropertyViolation)
            .unwrap_or_else(|| panic!("missing {directive} property finding: {}", report_human_summary(&report)));
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
            if expected == ReplayOutcome::NotReproduced {
                fs::write(&source, format!("export function {signature} {{ {repaired} }}")).unwrap();
            }
            let replay = replay_report(path.to_str().unwrap(), &finding.id, None,
                RuntimeProfile::LocalTrusted, DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE).await.unwrap();
            assert_eq!(replay.outcome, expected, "{directive}: {replay:?}");
        }
        // An initial target exception with the same diagnostic wording is not
        // evidence that the recorded property was evaluated and violated.
        fs::write(&source, format!("export function {signature} {{ throw new Error({}); }}", serde_json::to_string(&finding.message).unwrap())).unwrap();
        let replay = replay_report(path.to_str().unwrap(), &finding.id, None,
            RuntimeProfile::LocalTrusted, DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE).await.unwrap();
        assert_eq!(replay.outcome, ReplayOutcome::NotReproduced, "initial target exception must not impersonate the property: {replay:?}");
        if directive.is_empty() {
            fs::write(&source, format!("let calls = 0; class CopiedFailure extends Error {{}} Object.defineProperty(CopiedFailure, 'name', {{value: '_PropertyFailure'}}); export function {signature} {{ if (++calls % 2 === 0) throw new CopiedFailure({}); return value; }}", serde_json::to_string(&finding.message).unwrap())).unwrap();
            let replay = replay_report(path.to_str().unwrap(), &finding.id, None, RuntimeProfile::LocalTrusted, DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE).await.unwrap();
            assert_eq!(replay.outcome, ReplayOutcome::NotReproduced, "repeat-call exception with copied class name and wording is not a check: {replay:?}");
        }
    }
}

#[tokio::test]
async fn long_admitted_arguments_remain_executable_after_report_persistence() {
    let value = format!("{}'\\\"tail", "long-input-".repeat(32));
    let literal = serde_json::to_string(&value).unwrap();
    for (language, filename, code, repaired) in [
        (Language::Python, "target.py", format!("from typing import Literal\ndef consume(value: Literal[{literal}]) -> str:\n    raise ValueError('missing admitted branch')\n"), "def consume(value: str) -> str:\n    return value\n"),
        (Language::TypeScript, "target.ts", format!("export function consume(value: {literal}): string {{ throw new Error('missing admitted branch'); }}"), "export function consume(value: string): string { return value; }"),
    ] {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join(filename);
        fs::write(&source, &code).unwrap();
        let mut opts = default_opts(None);
        opts.project_dir = project.path().to_str();
        opts.source_file = source.to_str();
        let report = verify(&code, &language, opts).await;
        let repair = repair_summary(&report, &language);
        let finding = repair.findings.iter().find(|finding| finding.location.function == "consume")
            .unwrap_or_else(|| panic!("missing long-input finding: {}", report_human_summary(&report)));
        assert!(finding.repro.arguments[0].expression.len() > 240, "executable expressions must not be display-truncated");
        assert_eq!(finding.repro.arguments[0].json_value, Some(serde_json::json!(value)));
        let path = project.path().join("repair.json");
        fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
        for expected in [ReplayOutcome::Reproduced, ReplayOutcome::NotReproduced] {
            if expected == ReplayOutcome::NotReproduced {
                fs::write(&source, repaired).unwrap();
            }
            let replay = replay_report(path.to_str().unwrap(), &finding.id, None,
                RuntimeProfile::LocalTrusted, DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE).await.unwrap();
            assert_eq!(replay.outcome, expected, "{language:?}: {replay:?}");
        }
    }
}

#[tokio::test]
async fn closed_input_contract_exceptions_are_not_silently_rejected() {
    for (language, source, error_type) in [
        (Language::Python, "from typing import Literal\ndef label(value: Literal['draft', 'published']) -> str:\n    if value == 'draft':\n        return 'Draft'\n    raise ValueError('missing declared branch')\n", "ValueError"),
        (Language::Python, "from typing import Literal\nclass DomainFailure(Exception):\n    pass\ndef label(value: Literal['draft', 'published']) -> str:\n    if value == 'draft':\n        return 'Draft'\n    raise DomainFailure('missing declared branch')\n", "DomainFailure"),
        (Language::Python, "from typing import Literal\ndef label(value: Literal['draft', 'published']) -> str:\n    if value == 'draft':\n        return 'Draft'\n    raise AssertionError('missing declared branch')\n", "AssertionError"),
        (Language::TypeScript, "export function label(value: 'draft' | 'published'): string { if (value === 'draft') return 'Draft'; throw new Error('missing declared branch'); }", "Error"),
        (Language::TypeScript, "class DomainFailure extends Error {}\nexport function label(value: 'draft' | 'published'): string { if (value === 'draft') return 'Draft'; throw new DomainFailure('missing declared branch'); }", "DomainFailure"),
    ] {
        let report = verify(source, &language, default_opts(None)).await;
        assert_eq!(report.verdict, VerificationVerdict::Fail, "{language:?}: {}", report_human_summary(&report));
        let findings = report.stages.iter().find(|stage| stage.name == "execute")
            .and_then(|stage| stage.detail.as_ref()).and_then(|detail| detail["findings"].as_array()).unwrap();
        let finding = findings.iter().find(|finding| finding["error_type"] == error_type).expect("admitted-input exception finding");
        assert_eq!(finding["input_classification"], "valid");
        assert_eq!(finding["category"], "exception");
        assert!(finding["repro"]["snippet"].as_str().unwrap().contains("published"), "minimization must retain an admitted counterexample");
    }
}

#[tokio::test]
async fn closed_input_contract_clean_controls_stay_clean() {
    for (language, source) in [
        (Language::Python, "from typing import Literal\ndef label(value: Literal['draft', 'published']) -> str:\n    if value == 'draft':\n        return 'Draft'\n    if value == 'published':\n        return 'Published'\n    raise ValueError('outside declared domain')\n"),
        (Language::TypeScript, "export function label(value: 'draft' | 'published'): string { if (value === 'draft') return 'Draft'; if (value === 'published') return 'Published'; throw new Error('outside declared domain'); }"),
    ] {
        let report = verify(source, &language, default_opts(None)).await;
        assert_eq!(report.verdict, VerificationVerdict::Pass, "{language:?}: {}", report_human_summary(&report));
        assert_eq!(report.summary.findings.total, 0);
    }
}

#[tokio::test]
async fn semantic_observation_replays_and_stops_when_recorded_expectation_is_met() {
    let cases = [
        (
            "pep440_version_ordering",
            "compare_versions",
            "left: str, right: str",
            "int",
            "return 0",
            false,
        ),
        (
            "pep440_specifier_membership",
            "allows",
            "version: str, specifier: str",
            "bool",
            "return True",
            false,
        ),
        (
            "pep440_filter_prerelease",
            "filter_versions",
            "candidates: list[str], specifier: str",
            "list[str]",
            "return []",
            false,
        ),
        (
            "cookie_value_quote",
            "format_cookie_value",
            "value: str",
            "str",
            "return value.strip().strip(chr(34))",
            false,
        ),
        (
            "cookie_header_quote",
            "build_cookie_header",
            "cookies: dict[str, str | None]",
            "str",
            "cookies.clear(); return ''",
            false,
        ),
        (
            "query_string_serializer",
            "canonical_query",
            "params: dict[str, object]",
            "str",
            "return ''",
            true,
        ),
    ];
    for (property, name, signature, return_type, body, query) in cases {
        for (body, default_verdict) in [
            (body, Some(VerificationVerdict::Pass)),
            ("raise RuntimeError('unavailable')", None),
            ("return object()", None),
        ] {
            let project = tempfile::tempdir().unwrap();
            let source = project.path().join("target.py");
            let code = format!("# court-jester-properties {property}\ndef {name}({signature}) -> {return_type}:\n    {body}\n");
            fs::write(&source, &code).unwrap();
            let mut options = default_opts(None);
            options.source_file = source.to_str();
            options.project_dir = project.path().to_str();
            let report = verify(&code, &Language::Python, options).await;
            if let Some(default_verdict) = default_verdict {
                assert_eq!(
                    report.verdict,
                    default_verdict,
                    "inferred observations remain advisory by default: {}",
                    report_human_summary(&report)
                );
            }
            assert!(
                report
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.kind != FailureKind::HarnessProtocol),
                "{name}: {:?}",
                report.diagnostics
            );
            let repair = repair_summary(&report, &Language::Python);
            let finding = repair
                .findings
                .iter()
                .find(|finding| finding.oracle.kind == OracleKind::InferredSemantic)
                .unwrap_or_else(|| {
                    panic!(
                        "missing {property} observation for {body}: {}\n{:?}",
                        report_human_summary(&report),
                        report
                            .stages
                            .iter()
                            .find(|stage| stage.name == "execute")
                            .and_then(|stage| stage.detail.as_ref())
                            .map(|detail| &detail["execution"]["stderr"])
                    )
                });
            assert_eq!(finding.confidence, FindingConfidence::Low);
            assert!(finding.oracle.expected.is_some());
            if name == "build_cookie_header" {
                assert!(
                    finding.repro.arguments[0]
                        .json_value
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .contains_key("session"),
                    "target mutation must not change the recorded input"
                );
            }
            let id = finding.id.clone();
            let expected = finding.oracle.expected.as_ref().unwrap().clone();
            let report_path = project.path().join("repair.json");
            fs::write(&report_path, serde_json::to_vec(&repair).unwrap()).unwrap();
            let replay = replay_report(
                report_path.to_str().unwrap(),
                &id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(
                replay.outcome,
                ReplayOutcome::Reproduced,
                "{property}: {replay:?}"
            );
            // This replacement satisfies this recorded observation; it is not a
            // claim that a constant implementation satisfies the entire contract.
            let value = format!(
                "__import__('json').loads({})",
                serde_json::to_string(&expected).unwrap()
            );
            let value = if query {
                format!("__import__('urllib.parse', fromlist=['urlencode']).urlencode(list(map(tuple, {value})))")
            } else {
                value
            };
            fs::write(&source, format!("def {name}(*args):\n    return {value}\n")).unwrap();
            let replay = replay_report(
                report_path.to_str().unwrap(),
                &id,
                None,
                RuntimeProfile::LocalTrusted,
                DEFAULT_PYTHON_DOCKER_IMAGE,
                DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            )
            .await
            .unwrap();
            assert_eq!(
                replay.outcome,
                ReplayOutcome::NotReproduced,
                "{property}: {replay:?}"
            );
        }
    }
}

#[tokio::test]
async fn generated_invocation_counts_match_completed_lifecycle_records() {
    let report = verify(
        "def identity(value: int) -> int:\n    return value\n",
        &Language::Python,
        default_opts(None),
    )
    .await;
    assert_eq!(report.verdict, VerificationVerdict::Pass);
    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .unwrap()
        .detail
        .as_ref()
        .unwrap();
    let surfaces = detail["harness_events"]["surfaces"].as_object().unwrap();
    let count: u64 = surfaces
        .values()
        .map(|value| value["valid_completed"].as_u64().unwrap())
        .sum();
    assert!(
        count > 1,
        "multiple fuzz iterations must not be collapsed into one invocation"
    );
    assert_eq!(detail["valid_invocations"].as_u64(), Some(count));
    assert_eq!(
        report.summary.fuzz_pass, 1,
        "function counts remain distinct from invocation counts"
    );
}

fn default_opts(test_code: Option<&str>) -> VerifyOptions<'_> {
    VerifyOptions {
        test_code,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    }
}

async fn verify_differential_files(
    candidate: &str,
    baseline: &str,
    language: Language,
) -> VerificationReport {
    let root = tempfile::tempdir().unwrap();
    let candidate_root = root.path().join("candidate");
    let baseline_root = root.path().join("baseline");
    std::fs::create_dir_all(&candidate_root).unwrap();
    std::fs::create_dir_all(&baseline_root).unwrap();
    let extension = match language {
        Language::Python => "py",
        Language::TypeScript => "ts",
    };
    let candidate_file = candidate_root.join(format!("target.{extension}"));
    let baseline_file = baseline_root.join(format!("target.{extension}"));
    std::fs::write(&candidate_file, candidate).unwrap();
    std::fs::write(&baseline_file, baseline).unwrap();
    let mut options = default_opts(None);
    options.project_dir = Some(candidate_root.to_str().unwrap());
    options.source_file = Some(candidate_file.to_str().unwrap());
    options.base_code = Some(baseline);
    options.base_project_dir = Some(baseline_root.to_str().unwrap());
    options.base_source_file = Some(baseline_file.to_str().unwrap());
    verify(candidate, &language, options).await
}

fn assert_advisory_inferred_finding(
    report: &VerificationReport,
    function: &str,
    message_fragment: &str,
) {
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "low-confidence inferred findings must remain advisory: {:#?}",
        report.stages
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(execute_stage.status, StageStatus::Passed);
    assert_inferred_finding_metadata(report, function, message_fragment);
}

fn assert_inferred_finding_metadata(
    report: &VerificationReport,
    function: &str,
    message_fragment: &str,
) {
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    let detail = execute_stage
        .detail
        .as_ref()
        .expect("execute detail should be present");
    let findings = detail["findings"]
        .as_array()
        .expect("typed findings should be present");
    let finding = findings
        .iter()
        .find(|finding| {
            finding["location"]["function"].as_str() == Some(function)
                && finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(message_fragment))
        })
        .unwrap_or_else(|| panic!("expected advisory finding for {function}: {findings:#?}"));
    assert_eq!(finding["severity"].as_str(), Some("property_violation"));
    assert_eq!(finding["category"].as_str(), Some("property"));
    assert_eq!(finding["confidence"].as_str(), Some("low"));
    assert_eq!(
        finding["oracle"]["kind"].as_str(),
        Some("inferred_semantic")
    );
    assert_eq!(
        finding["oracle"]["provenance"].as_str(),
        Some("name_heuristic")
    );
    assert_eq!(finding["oracle"]["confidence"].as_str(), Some("low"));
    assert!(
        finding["message"]
            .as_str()
            .is_some_and(|message| message.contains(message_fragment)),
        "finding should retain the inferred contract failure: {finding:#?}"
    );
    assert_eq!(detail["findings_summary"]["gating"].as_u64(), Some(0));
    assert!(detail["findings_summary"]["advisory"]
        .as_u64()
        .is_some_and(|count| count >= 1));
}

#[tokio::test]
async fn good_python_function() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(
        report
            .stages
            .iter()
            .any(|s| s.name == "parse" && s.status == StageStatus::Passed),
        "parse stage should pass"
    );
    // execute stage should also pass (42 + 42 doesn't error)
    if let Some(exec) = report.stages.iter().find(|s| s.name == "execute") {
        assert!(
            exec.status == StageStatus::Passed,
            "execute stage failed: {:?}",
            exec.message
        );
    }
}

#[tokio::test]
async fn python_generated_harness_imports_target_without_running_main_block() {
    let project = tempfile::tempdir().unwrap();
    let source_path = project.path().join("target.py");
    let code = r#"import argparse

def double(value: int) -> int:
    return value * 2

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--required", required=True)
    parser.parse_args()
"#;
    std::fs::write(&source_path, code).unwrap();
    let mut opts = default_opts(None);
    opts.project_dir = Some(project.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());

    let report = verify(code, &Language::Python, opts).await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("generated fuzz harness should execute");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "target CLI main block must not run: {:#?}",
        execute
    );
    let stdout = execute.detail.as_ref().unwrap()["execution"]["stdout"]
        .as_str()
        .expect("harness stdout should be recorded");
    assert!(
        stdout.contains("FUZZ double:"),
        "fuzz execution should reach double, got: {stdout}"
    );

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage should be reported");
    let double = coverage["functions"]
        .as_array()
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"].as_str() == Some("double"))
        })
        .expect("double coverage should be reported");
    assert_eq!(double["status"].as_str(), Some("checked_direct"));
}

#[tokio::test]
async fn python_differential_harness_imports_targets_without_running_main_blocks() {
    let code = r#"def double(value: int) -> int:
    return value * 2

if __name__ == "__main__":
    raise RuntimeError("CLI entry point ran")
"#;

    let report = verify_differential_files(code, code, Language::Python).await;
    let differential = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["differential"].as_object())
        .expect("differential execution should be reported");
    assert_eq!(differential["enabled"].as_bool(), Some(true));
    assert_eq!(
        differential["units"][0]["status"].as_str(),
        Some("equal"),
        "candidate and baseline probes should both reach double: {differential:#?}"
    );
}

#[tokio::test]
async fn syntax_error_short_circuits() {
    let code = "def foo(:";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(report.verdict != VerificationVerdict::Pass);
    assert_eq!(
        report
            .stages
            .iter()
            .map(|stage| stage.name.as_str())
            .collect::<Vec<_>>(),
        ["parse", "execute", "outcome_matrix"],
        "parse failure should skip later work while reporting omitted execution and outcomes"
    );
    assert_eq!(report.stages[0].status, StageStatus::Failed);
    assert_eq!(report.stages[1].status, StageStatus::Skipped);
    assert_eq!(
        report.stages[1].detail.as_ref().unwrap()["reason"].as_str(),
        Some("parse_failed")
    );
}

#[tokio::test]
async fn with_passing_tests() {
    let code = "def double(x: int) -> int:\n    return x * 2";
    let tests = "assert double(5) == 10\nassert double(0) == 0";
    let report = verify(code, &Language::Python, default_opts(Some(tests))).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[test]
fn complete_authoritative_tests_supersede_skipped_generated_execution() {
    let stages = vec![
        VerificationStage {
            name: "parse".into(),
            status: StageStatus::Passed,
            duration_ms: 0,
            detail: None,
            message: None,
        },
        VerificationStage {
            name: "execute".into(),
            status: StageStatus::Skipped,
            duration_ms: 0,
            detail: None,
            message: Some("no generated cases".into()),
        },
        VerificationStage {
            name: "test".into(),
            status: StageStatus::Passed,
            duration_ms: 0,
            detail: None,
            message: None,
        },
    ];
    let coverage = CoverageSummary {
        required: 2,
        behaviorally_checked: 2,
        ..Default::default()
    };
    let evidence = VerificationEvidence {
        parsed: true,
        static_checks_completed: true,
        authoritative_test_completed: true,
        authoritative_test_covered_surfaces: 2,
        ..Default::default()
    };

    assert_eq!(
        final_verdict(&stages, &coverage, CoverageGate::ChangedExports, &evidence),
        (
            VerificationVerdict::Pass,
            VerificationStrength::AuthoritativeTests
        )
    );

    let mut blocked_generated = stages.clone();
    blocked_generated[1].status = StageStatus::Inconclusive;
    blocked_generated[1].detail = Some(serde_json::json!({
        "diagnostics": [FailureDiagnostic {
            domain: FailureDomain::VerifierHarness,
            kind: FailureKind::HarnessProtocol,
            component: DiagnosticComponent::FuzzHarness,
            impact: DiagnosticImpact::Blocking,
            message: "generated harness emitted no bootstrap event".into(),
            process: None,
            limits: None,
        }]
    }));
    assert_eq!(
        final_verdict(
            &blocked_generated,
            &coverage,
            CoverageGate::ChangedExports,
            &evidence,
        ),
        (
            VerificationVerdict::Pass,
            VerificationStrength::AuthoritativeTests
        ),
        "complete authoritative evidence must supersede a non-target generated harness blocker"
    );
}

#[tokio::test]
async fn tests_only_verify_skips_execute_stage() {
    let code = "def inverse(x: int) -> float:\n    return 1 / x";
    let tests = "assert inverse(2) == 0.5";
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn tests_only_verify_requires_authoritative_test() {
    let code = "def inverse(x: int) -> float:\n    return 1 / x";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    let test_stage = report
        .stages
        .iter()
        .find(|s| s.name == "test")
        .expect("tests_only mode should emit a failing test stage");
    assert!(test_stage.status != StageStatus::Passed);
    assert_eq!(
        test_stage.message.as_deref(),
        Some("tests_only mode requires an authoritative test")
    );
}

#[tokio::test]
async fn with_failing_tests() {
    let code = "def double(x: int) -> int:\n    return x * 3"; // bug: *3 instead of *2
    let tests = "assert double(5) == 10";
    let report = verify(code, &Language::Python, default_opts(Some(tests))).await;

    assert!(report.verdict != VerificationVerdict::Pass);
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status != StageStatus::Passed));
}

#[tokio::test]
async fn lint_warnings_are_informational() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "biome",
        "#!/bin/sh\ncat <<'EOF'\n{\"diagnostics\":[{\"category\":\"lint/style/noNonNullAssertion\",\"description\":\"Avoid non-null assertions.\",\"severity\":\"warning\",\"location\":{\"start\":{\"line\":3,\"column\":12}}}]}\nEOF\nexit 1\n",
    );

    let code = r#"
function normalizeName(name: string): string {
    return name!.trim();
}
"#;
    let mut opts = default_opts(None);
    opts.project_dir = Some(project_dir.path().to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "lint warnings must not affect the verdict: {:#?}",
        report.stages
    );

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    assert_eq!(
        lint_stage.status,
        StageStatus::Advisory,
        "lint warnings should be reported as advisory"
    );
    assert_eq!(report.summary.lint_issues, 1);

    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        !diagnostics.is_empty(),
        "expected lint diagnostics to remain in the report"
    );
}

#[tokio::test]
async fn lint_runner_failures_do_not_count_as_lint_issues_in_summary() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "biome",
        "#!/bin/sh\ncat <<'EOF'\n{\"diagnostics\":[{\"category\":\"internalError/io\",\"description\":\"No such file or directory (os error 2)\",\"severity\":\"fatal\",\"location\":{\"start\":{\"line\":0,\"column\":0}}}]}\nEOF\nexit 1\n",
    );

    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let mut opts = default_opts(None);
    opts.project_dir = Some(project_dir.path().to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "lint runner failures should remain advisory: {:#?}",
        report.stages
    );
    assert_eq!(report.summary.lint_issues, 0);
    assert_eq!(report.summary.lint_runner_failures, 1);

    let lint_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "lint")
        .expect("lint stage should exist");
    assert!(
        lint_stage.status != StageStatus::Passed,
        "runner failure should fail the lint stage itself"
    );
    let detail = lint_stage
        .detail
        .as_ref()
        .expect("lint detail should exist");
    assert_eq!(
        detail
            .get("runner_failed")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        detail
            .get("runner_diagnostics")
            .and_then(|value| value.as_array())
            .map(|arr| arr.len()),
        Some(1)
    );
    assert_eq!(
        detail
            .get("diagnostics")
            .and_then(|value| value.as_array())
            .map(|arr| arr.len()),
        Some(0)
    );
}

#[test]
fn human_summary_highlights_offenders_and_findings() {
    let report = VerificationReport {
        schema_version: 3,
        tool: ToolProvenance::default(),
        candidate: CandidateProvenance::default(),
        stages: vec![
            VerificationStage {
                name: "complexity".into(),
                status: StageStatus::Failed,
                duration_ms: 0,
                detail: Some(serde_json::json!({
                    "threshold": 0,
                    "violations": [{
                        "function": "deepGet",
                        "line": 3,
                        "complexity": 1,
                        "cognitive_complexity": 0,
                    }],
                })),
                message: Some("1 function(s) exceed complexity threshold 0".into()),
            },
            VerificationStage {
                name: "execute".into(),
                status: StageStatus::Failed,
                duration_ms: 1,
                detail: Some(serde_json::json!({
                    "finding_counts": {
                        "crash": 0,
                        "property_violation": 1,
                    },
                    "findings": [{
                        "function": "deepGet",
                        "severity": "high",
                        "message": "Return value must be a non-empty string",
                    }],
                    "no_inputs_reached": 0,
                })),
                message: None,
            },
        ],
        verdict: VerificationVerdict::Fail,
        strength: VerificationStrength::PropertyChecked,
        summary: ReportSummary {
            functions_analyzed: 1,
            functions_fuzzed: 1,
            functions_skipped: 0,
            functions_blocked_module_load: 0,
            fuzz_pass: 0,
            fuzz_no_inputs_reached: 0,
            findings: FindingsSummary {
                total: 1,
                gating: 1,
                ..Default::default()
            },
            suppressed_complexity_violations: 0,
            suppressed_portability_warnings: 0,
            lint_issues: 0,
            lint_runner_failures: 0,
            complexity_violations: 1,
            coverage: CoverageSummary::default(),
            diagnostics: Default::default(),
        },
        diagnostics: vec![],
        diagnostics_summary: None,
        report_path: None,
    };

    let summary = report_human_summary(&report);
    assert!(summary.contains("Overall: Fail"), "got:\n{summary}");
    assert!(summary.contains("Stages:"), "got:\n{summary}");
    assert!(
        summary.contains("Top Complexity Offenders:"),
        "got:\n{summary}"
    );
    assert!(summary.contains("Top Execute Findings:"), "got:\n{summary}");
    assert!(summary.contains("deepGet"), "got:\n{summary}");
}

#[tokio::test]
async fn verify_passes_project_local_lint_context_to_ruff() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join(".venv").join("bin");
    let log_path = project_dir.path().join("ruff-verify.log");
    let config_path = project_dir.path().join("ruff.toml");
    let source_path = project_dir.path().join("src").join("account.py");
    std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();

    let code = "def add(a: int, b: int) -> int:\n    return a + b\n";
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&config_path, "[lint]\n").unwrap();

    install_fake_tool_at(
        &tool_dir,
        "ruff",
        &format!(
            r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" > "{log}"
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{log}"
done
cat <<'EOF'
[{{"code":"F841","message":"local variable is assigned to but never used","location":{{"row":1,"column":1}}}}]
EOF
exit 1
"#,
            log = log_path.display(),
        ),
    );

    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(project_dir.path().to_str().unwrap()),
            lint_config_path: Some(config_path.to_str().unwrap()),
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "lint diagnostics should stay informational"
    );

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        diagnostics
            .iter()
            .any(|diag| diag.get("rule").and_then(|value| value.as_str()) == Some("F841")),
        "real-file verify runs should keep file-aware unused-variable diagnostics"
    );

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert_log_contains_path(&log, "cwd=", project_dir.path());
    assert!(log.contains("arg=check"));
    assert!(log.contains("arg=--config"));
    assert_log_contains_path(&log, "arg=", &config_path);
    assert_log_contains_path(&log, "arg=", &source_path);
}

#[tokio::test]
async fn verify_filters_unused_variable_diagnostics_for_anonymous_inline_snippets() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join(".venv").join("bin");
    install_fake_tool_at(
        &tool_dir,
        "ruff",
        "#!/bin/sh\ncat <<'EOF'\n[{\"code\":\"F841\",\"message\":\"assigned but unused\",\"location\":{\"row\":1,\"column\":1}}]\nEOF\nexit 1\n",
    );

    let code = "def add(a: int, b: int) -> int:\n    return a + b\n";
    let mut opts = default_opts(None);
    opts.project_dir = Some(project_dir.path().to_str().unwrap());
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "snippet-only unused diagnostics should not fail verify"
    );

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        diagnostics.is_empty(),
        "anonymous inline snippets should continue filtering unused-variable false positives"
    );
}

#[tokio::test]
async fn typescript_generated_harness_exits_after_completion_despite_open_handles() {
    let code = r#"
setInterval(() => {}, 60_000);

export function increment(value: number): number {
  return value + 1;
}
"#;
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        verify(code, &Language::TypeScript, default_opts(None)),
    )
    .await
    .expect("completed generated harness must terminate without waiting for imported open handles");

    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert_eq!(
        report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .map(|stage| stage.status),
        Some(StageStatus::Passed)
    );
}

#[tokio::test]
async fn blank_label_output_is_not_failed_by_name_only() {
    let code = r#"
function secondaryLabel(labels: string[]): string {
    if (labels.length < 2) return "general";
    return labels[1].trim().toLowerCase();
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        exec_stage.status == StageStatus::Passed,
        "array label helper should not fail from name-only nonempty-string inference"
    );
}

#[tokio::test]
async fn typescript_record_array_annotation_generates_only_array_arguments() {
    let code = r#"
export function countRecords(
  records: Record<string, unknown>[]
): number {
  for (const record of records) {
    if (record === null || typeof record !== "object" || Array.isArray(record)) {
      throw new TypeError("records must contain plain objects")
    }
  }
  return records.length
}

export function countGenericRecords(
  records: Array<Record<string, unknown>>
): number {
  return records.length
}

export function countReadonlyRecords(
  records: ReadonlyArray<Record<string, unknown>>
): number {
  return records.length
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let parse_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "parse")
        .expect("parse stage should be present");
    assert_eq!(
        parse_stage.detail.as_ref().unwrap()["functions"][0]["params"][0]["type_annotation"],
        "Record<string, unknown>[]"
    );
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "valid array annotations must never be exercised with scalar arguments: {:#?}",
        report.stages
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(execute_stage.status, StageStatus::Passed);
    assert!(execute_stage.detail.as_ref().unwrap()["findings"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[tokio::test]
async fn typescript_empty_array_default_only_generates_assignable_empty_arrays() {
    let code = r#"
export function size(values = []): number {
  if (!Array.isArray(values) || values.length !== 0) {
    throw new TypeError("values must remain an empty array")
  }
  return values.length
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let parse_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "parse")
        .expect("parse stage should be present");
    assert_eq!(
        parse_stage.detail.as_ref().unwrap()["functions"][0]["params"][0]["type_annotation"],
        "never[]"
    );
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "an inferred empty-array default should be fuzzed only with [] and omission: {:#?}",
        report.stages
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(execute_stage.status, StageStatus::Passed);
    let coverage_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .expect("coverage stage should be present");
    assert_eq!(
        coverage_stage.detail.as_ref().unwrap()["functions"][0]["status"],
        "checked_direct",
        "the empty-array target must execute fuzz cases"
    );
    assert!(execute_stage.detail.as_ref().unwrap()["findings"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[tokio::test]
async fn blank_city_output_fails_verify() {
    let code = r#"
type User = {
    address?: {
        city?: string | null;
    } | null;
} | null;

function primaryCity(user: User): string {
    const city = user?.address?.city;
    return city ? city.trim() : "Unknown";
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "a name-only city fallback is not an authoritative non-empty contract: {:#?}",
        report.stages
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(execute_stage.status, StageStatus::Passed);
    assert!(execute_stage.detail.as_ref().unwrap()["findings"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[tokio::test]
async fn missing_preferred_timezone_fails_verify() {
    let code = r#"
def preferred_timezone(profile: dict | None) -> str:
    return profile["preferences"]["timezone"].strip()
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        exec_stage.status != StageStatus::Passed,
        "missing nested preference data should fail verify"
    );
}

#[tokio::test]
async fn feature_flag_nested_none_fails_verify() {
    let code = r#"
def beta_checkout_enabled(config: dict | None) -> bool:
    value = (config or {}).get("flags", {}).get("beta_checkout")
    if value is None:
        return True
    return value
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "unobserved benchmark-shaped nested feature-flag data must not be invented: {:#?}",
        report.stages
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(execute_stage.status, StageStatus::Passed);
    assert!(execute_stage.detail.as_ref().unwrap()["findings"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[tokio::test]
async fn typescript_feature_flag_explicit_false_fails_verify() {
    let code = r#"
type Config = {
  flags?: {
    betaCheckout?: boolean | null;
  } | null;
} | null;

function defaultFlags(): { betaCheckout: boolean } {
  return { betaCheckout: true };
}

export function betaCheckoutEnabled(config: Config): boolean {
  return config?.flags?.betaCheckout || defaultFlags().betaCheckout;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(
        &report,
        "betaCheckoutEnabled",
        "Feature flag semantics (explicit false)",
    );
}

#[tokio::test]
async fn typescript_feature_flag_explicit_false_can_pass_verify() {
    let code = r#"
type Config = {
  flags?: {
    betaCheckout?: boolean | null;
  } | null;
} | null;

function defaultFlags(): { betaCheckout: boolean } {
  return { betaCheckout: true };
}

export function betaCheckoutEnabled(config: Config): boolean {
  return config?.flags?.betaCheckout ?? defaultFlags().betaCheckout;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_semver_compare_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  return 0;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "compareVersions", "Semver compare semantics");
}

#[tokio::test]
async fn typescript_semver_compare_prerelease_can_pass_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareIdentifiers(left: string, right: string): number {
  const leftNumeric = /^\d+$/.test(left);
  const rightNumeric = /^\d+$/.test(right);
  if (leftNumeric && rightNumeric) {
    const a = Number.parseInt(left, 10);
    const b = Number.parseInt(right, 10);
    return a === b ? 0 : a < b ? -1 : 1;
  }
  if (leftNumeric) return -1;
  if (rightNumeric) return 1;
  return left === right ? 0 : left < right ? -1 : 1;
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  if (a.prerelease == null && b.prerelease == null) return 0;
  if (a.prerelease == null) return 1;
  if (b.prerelease == null) return -1;
  for (let i = 0; i < Math.min(a.prerelease.length, b.prerelease.length); i++) {
    const cmp = compareIdentifiers(a.prerelease[i], b.prerelease[i]);
    if (cmp !== 0) return cmp;
  }
  if (a.prerelease.length === b.prerelease.length) return 0;
  return a.prerelease.length < b.prerelease.length ? -1 : 1;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_semver_caret_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  return candidate.major === base.major;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "matchesCaret", "Semver caret semantics");
}

#[tokio::test]
async fn typescript_semver_caret_prerelease_can_pass_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base || candidate.prerelease != null) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  if (base.major > 0) {
    return candidate.major === base.major;
  }
  if (base.minor > 0) {
    return candidate.major === 0 && candidate.minor === base.minor;
  }
  return candidate.major === 0 && candidate.minor === 0 && candidate.patch === base.patch;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_semver_caret_same_core_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  if (candidate.prerelease) {
    if (
      candidate.major !== base.major ||
      candidate.minor !== base.minor ||
      candidate.patch !== base.patch
    ) {
      return false;
    }
  }
  if (base.major !== 0) {
    return candidate.major === base.major;
  }
  if (base.minor !== 0) {
    return candidate.major === base.major && candidate.minor === base.minor;
  }
  return candidate.major === 0 && candidate.minor === 0 && candidate.patch === base.patch;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "matchesCaret", "Semver caret semantics");
}

#[tokio::test]
async fn typescript_defaults_null_override_and_inherited_keys_fail_verify() {
    let code = r#"
const objectProto = Object.prototype;

function shouldAssignDefault(object: Record<string, unknown>, key: string): boolean {
  const value = object[key];
  return value == null || (value === objectProto[key] && !Object.hasOwn(object, key));
}

export function defaults<T extends object>(object: T, ...sources: Array<object | null | undefined>): T {
  const target = Object(object) as Record<string, unknown>;
  for (const source of sources) {
    if (source == null) {
      continue;
    }
    for (const key of Object.keys(source)) {
      if (shouldAssignDefault(target, key)) {
        target[key] = (source as Record<string, unknown>)[key];
      }
    }
  }
  return target as T;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(
        &report,
        "defaults",
        "Defaults semantics (null target preserves value)",
    );
}

#[tokio::test]
async fn typescript_defaults_null_override_and_inherited_keys_can_pass_verify() {
    let code = r#"
const objectProto = Object.prototype;

function shouldAssignDefault(object: Record<string, unknown>, key: string): boolean {
  const value = object[key];
  return value === undefined || (value === objectProto[key] && !Object.hasOwn(object, key));
}

export function defaults<T extends object>(object: T, ...sources: Array<object | null | undefined>): T {
  const target = Object(object) as Record<string, unknown>;
  for (const source of sources) {
    if (source == null) {
      continue;
    }
    for (const key in Object(source)) {
      if (shouldAssignDefault(target, key)) {
        target[key] = (source as Record<string, unknown>)[key];
      }
    }
  }
  return target as T;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn query_string_nullish_leak_fails_verify() {
    let code = r#"
from urllib.parse import quote_plus

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        value = params[key]
        if value is None:
            continue
        if isinstance(value, list):
            for item in value:
                parts.append(f"{quote_plus(str(key))}={quote_plus(str(item).strip())}")
        else:
            parts.append(f"{quote_plus(str(key))}={quote_plus(str(value).strip())}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "canonical_query", "Query semantics (tag/nullish)");
}

#[tokio::test]
async fn query_string_blank_and_unicode_semantics_fail_verify() {
    let code = r#"
from urllib.parse import quote_plus

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        value = params[key]
        if value is None:
            continue
        if isinstance(value, list):
            for item in value:
                if item is None:
                    continue
                parts.append(f"{quote_plus(str(key))}={quote_plus(str(item).strip())}")
        else:
            parts.append(f"{quote_plus(str(key))}={quote_plus(str(value).strip())}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "canonical_query", "Query semantics (blank scalar)");
}

#[tokio::test]
async fn query_string_canonicalization_can_pass_verify() {
    let code = r#"
from urllib.parse import quote_plus
import unicodedata

def _canonical_scalar(value: object) -> str | None:
    if value is None or isinstance(value, (dict, list, tuple, set)):
        return None
    text = unicodedata.normalize("NFKD", str(value).strip()).encode("ascii", "ignore").decode("ascii")
    return text or None

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        raw = params[key]
        values = raw if isinstance(raw, list) else [raw]
        for item in values:
            text = _canonical_scalar(item)
            if text is None:
                continue
            parts.append(f"{quote_plus(str(key))}={quote_plus(text)}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_query_string_blank_and_unicode_semantics_fail_verify() {
    let code = r#"
export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const value = params[key];
    if (value == null) {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        if (item == null) {
          continue;
        }
        entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(item).trim())}`);
      }
    } else {
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value).trim())}`);
    }
  }
  return entries.join("&");
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_advisory_inferred_finding(&report, "canonicalQuery", "Query semantics (blank scalar)");
}

#[tokio::test]
async fn typescript_query_string_canonicalization_can_pass_verify() {
    let code = r#"
function canonicalScalar(value: unknown): string | null {
  if (value == null || Array.isArray(value) || (typeof value === "object" && value !== null)) {
    return null;
  }
  const text = String(value).trim().normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
  return text.length > 0 ? text : null;
}

export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const raw = params[key];
    const values = Array.isArray(raw) ? raw : [raw];
    for (const item of values) {
      const text = canonicalScalar(item);
      if (text == null) {
        continue;
      }
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(text)}`);
    }
  }
  return entries.join("&");
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_context_notes_can_enable_nested_query_bracket_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("stringify.ts");
    let code = r#"
function appendScalar(entries: string[], key: string, value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }
  entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
}

export function stringifyQuery(input: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(input).sort()) {
    const value = input[key];
    if (value && typeof value === "object" && !Array.isArray(value)) {
      for (const childKey of Object.keys(value as Record<string, unknown>).sort()) {
        appendScalar(entries, `${key}[${childKey}]`, (value as Record<string, unknown>)[childKey]);
      }
      continue;
    }
    appendScalar(entries, key, value);
  }
  return entries.join("&");
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("UPSTREAM_NOTES.md"),
        "stringifyQuery(input) -> string uses query bracket notation: nested object arrays use [] suffixes.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_advisory_inferred_finding(
        &report,
        "stringifyQuery",
        "Query semantics (top-level repeated array)",
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail should be present");
    assert_eq!(
        coverage["inferred_context_properties"]["stringifyQuery"][0].as_str(),
        Some("query_nested_brackets")
    );
}

#[tokio::test]
async fn typescript_urlencoded_context_targets_parser_not_setting_normalizer() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("query.ts");
    let code = r#"
type QueryParserSetting = "simple" | "extended" | true | false | ((input: string) => unknown);

export function normalizeQueryParserSetting(input: unknown): QueryParserSetting {
  if (input === undefined || input === true) {
    return "simple";
  }
  if (input === false) {
    return false;
  }
  if (typeof input === "function") {
    return input;
  }
  if (input === "simple" || input === "extended") {
    return input;
  }
  throw new Error(`unknown value for query parser: ${String(input)}`);
}

export function parseQueryString(input: string, setting: QueryParserSetting): unknown {
  const result: Record<string, unknown> = {};
  if (!input.trim() || setting === false) {
    return result;
  }
  for (const segment of input.split("&")) {
    const [rawKey, rawValue = ""] = segment.split("=", 2);
    result[decodeURIComponent(rawKey)] = decodeURIComponent(rawValue);
  }
  return result;
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("WORKMAP.md"),
        "Build extended urlencoded parsing for nested form bodies. lib/query.ts owns query-string parsing for the urlencoded body parser.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    assert!(repair_summary(&report, &Language::TypeScript)
        .findings
        .iter()
        .any(|finding| finding.input_classification == InputClassification::Unknown));
    assert_inferred_finding_metadata(
        &report,
        "parseQueryString",
        "Query parse semantics (repeated scalar)",
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail should be present");
    assert_eq!(
        coverage["inferred_context_properties"]["parseQueryString"][0].as_str(),
        Some("query_nested_brackets")
    );
    assert!(
        coverage["inferred_context_properties"]
            .get("normalizeQueryParserSetting")
            .is_none(),
        "setting normalizer should not inherit parser-specific nested-input semantics"
    );
}

#[tokio::test]
async fn typescript_request_metadata_context_enables_side_effect_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("http.ts");
    let code = r#"
type RequestLike = {
  method?: string;
  url?: string;
  headers?: Record<string, string | undefined>;
  encrypted?: boolean;
  app?: { __settings: Map<string, unknown> };
  [key: string]: unknown;
};

function requestHeader(request: RequestLike, name: string): string | undefined {
  const target = name.toLowerCase();
  for (const [headerName, headerValue] of Object.entries(request.headers || {})) {
    if (headerName.toLowerCase() === target) {
      return headerValue;
    }
  }
  return undefined;
}

export function decorateRequest(request: RequestLike): void {
  if (typeof request.get !== "function") {
    request.get = (name: string) => requestHeader(request, name);
  }
  if (typeof request.header !== "function") {
    request.header = request.get;
  }
  if (typeof request.protocol !== "string") {
    request.protocol = request.encrypted === true ? "https" : "http";
  }
  if (typeof request.secure !== "boolean") {
    request.secure = request.protocol === "https";
  }
  if (typeof request.xhr !== "boolean") {
    request.xhr = requestHeader(request, "x-requested-with")?.toLowerCase() === "xmlhttprequest";
  }
  request.query = {};
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("WORKMAP.md"),
        "Build request introspection behavior: header lookup, trust proxy protocol, XHR detection, and request decoration.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_advisory_inferred_finding(
        &report,
        "decorateRequest",
        "HTTP request metadata (trusted forwarded protocol)",
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail should be present");
    assert_eq!(
        coverage["inferred_context_properties"]["decorateRequest"][0].as_str(),
        Some("http_request_metadata")
    );
}

#[tokio::test]
async fn typescript_response_helpers_context_enables_side_effect_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("http.ts");
    let code = r#"
type RequestLike = {
  method?: string;
  headers?: Record<string, string | undefined>;
  [key: string]: unknown;
};

type ResponseLike = {
  statusCode?: number;
  statusMessage?: string;
  headersSent?: boolean;
  setHeader?: (name: string, value: string) => void;
  getHeader?: (name: string) => string | undefined;
  end?: (body?: unknown) => void;
  [key: string]: unknown;
};

const STATUS_TEXT: Record<number, string> = { 200: "OK", 204: "No Content" };

function normalizeHeaderName(name: string): string {
  return name.toLowerCase();
}

function ensureResponseInfrastructure(response: ResponseLike): void {
  if (!response.__headers) {
    response.__headers = new Map<string, string>();
  }
  if (typeof response.setHeader !== "function") {
    response.setHeader = (name: string, value: string) => {
      (response.__headers as Map<string, string>).set(normalizeHeaderName(name), value);
    };
  }
  if (typeof response.getHeader !== "function") {
    response.getHeader = (name: string) =>
      (response.__headers as Map<string, string>).get(normalizeHeaderName(name));
  }
  if (typeof response.end !== "function") {
    response.end = (body?: unknown) => {
      response.headersSent = true;
      response.__body = body ?? "";
    };
  }
  if (typeof response.statusCode !== "number") {
    response.statusCode = 200;
  }
}

export function decorateResponse(response: ResponseLike, request: RequestLike): void {
  ensureResponseInfrastructure(response);
  if (typeof response.status !== "function") {
    response.status = (code: number) => {
      response.statusCode = code;
      response.statusMessage = STATUS_TEXT[code] || String(code);
      return response;
    };
  }
  if (typeof response.send !== "function") {
    response.send = (body: unknown) => {
      response.end?.(body ?? "");
      return response;
    };
  }
  if (typeof response.sendStatus !== "function") {
    response.sendStatus = (code: number) => {
      response.status?.(code);
      return response.send?.(String(code));
    };
  }
  if (typeof response.location !== "function") {
    response.location = (value: string) => {
      response.setHeader?.("Location", value);
      return response;
    };
  }
  if (typeof response.vary !== "function") {
    response.vary = (field: string) => {
      response.setHeader?.("Vary", field);
      return response;
    };
  }
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("WORKMAP.md"),
        "Build response header and status helpers: location, vary, sendStatus, and empty response body behavior.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_advisory_inferred_finding(
        &report,
        "decorateResponse",
        "HTTP response helpers (location encodes spaces)",
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail should be present");
    assert_eq!(
        coverage["inferred_context_properties"]["decorateResponse"][0].as_str(),
        Some("http_response_helpers")
    );
}

#[tokio::test]
async fn typescript_static_file_context_promotes_internal_middleware_factory() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("index.ts");
    std::fs::create_dir(dir.path().join("static")).unwrap();
    std::fs::write(dir.path().join("static").join("hello.txt"), "hello world\n").unwrap();
    let code = r#"
type Handler = (req: any, res: any, next: () => void) => void;
type StaticMiddlewareOptions = {
  index?: string | false;
};

function createStaticMiddleware(root: string, options?: StaticMiddlewareOptions): Handler {
  void root;
  void options;
  return (_req, _res, next) => {
    next();
  };
}

export function express(): unknown {
  return {};
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("WORKMAP.md"),
        "Build the static-file wrapper from the visible public spec. Focus on serving a known static file correctly. Suggested build order: serve known files from static/.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail should be present");
    assert_eq!(
        coverage["inferred_context_properties"]["createStaticMiddleware"][0].as_str(),
        Some("http_static_file_middleware")
    );
    assert!(
        coverage["counts"]["checked_direct"].as_u64().unwrap_or(0) >= 1,
        "context should promote the internal static middleware factory into fuzzing"
    );
}

#[tokio::test]
async fn typescript_context_notes_can_enable_same_value_zero_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("internals.ts");
    let code = r#"
export function sameValueZero(left: unknown, right: unknown): boolean {
  return left === right;
}
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("UPSTREAM_NOTES.md"),
        "This lodash-derived shard uses SameValueZero equality for uniq.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_advisory_inferred_finding(
        &report,
        "sameValueZero",
        "SameValueZero semantics (NaN equals NaN)",
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail should be present");
    assert_eq!(
        execute["inferred_context_properties"]["sameValueZero"][0].as_str(),
        Some("same_value_zero")
    );
}

#[tokio::test]
async fn context_notes_can_enable_pep440_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("packaging_slice.py");
    let code = r#"
def compare_versions(left: str, right: str) -> int:
    return 0


def allows(version: str, specifier: str) -> bool:
    return True


def filter_versions(candidates: list[str], specifier: str) -> list[str]:
    return []
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        dir.path().join("UPSTREAM_NOTES.md"),
        "This pypa/packaging PEP 440 slice covers version-ordering behavior, \
         specifier behavior including compatible release ~= semantics, and \
         filter_versions prerelease fallback when prereleases are the only matching candidates.",
    )
    .unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    opts.inferred_oracle_gate = InferredOracleGate::Fail;
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail should be present");
    assert_eq!(
        execute["inferred_context_properties"]["compare_versions"][0].as_str(),
        Some("pep440_version_ordering")
    );
    assert_eq!(
        execute["inferred_context_properties"]["allows"][0].as_str(),
        Some("pep440_specifier_membership")
    );
    assert_eq!(
        execute["inferred_context_properties"]["filter_versions"][0].as_str(),
        Some("pep440_filter_prerelease")
    );
}

#[tokio::test]
async fn cookie_file_context_can_enable_cookie_quote_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("cookies.py");
    let code = r#"
from collections.abc import Mapping


def format_cookie_value(value: str) -> str:
    normalized = value.strip()
    if len(normalized) >= 2 and normalized[0] == normalized[-1] == '"':
        return normalized[1:-1]
    return normalized


def build_cookie_header(cookies: Mapping[str, str | None]) -> str:
    parts: list[str] = []
    for name, value in cookies.items():
        if value is None:
            continue
        parts.append(f"{name}={format_cookie_value(value)}")
    return "; ".join(parts)
"#;
    std::fs::write(&source_path, code).unwrap();

    let source_path_string = source_path.to_string_lossy().to_string();
    let project_dir_string = dir.path().to_string_lossy().to_string();
    let mut opts = default_opts(None);
    opts.source_file = Some(source_path_string.as_str());
    opts.project_dir = Some(project_dir_string.as_str());
    opts.inferred_oracle_gate = InferredOracleGate::Fail;
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail should be present");
    assert_eq!(
        execute["inferred_context_properties"]["format_cookie_value"][0].as_str(),
        Some("cookie_value_quote")
    );
    assert_eq!(
        execute["inferred_context_properties"]["build_cookie_header"][0].as_str(),
        Some("cookie_header_quote")
    );
}

#[tokio::test]
async fn python_test_stage_can_import_source_module_from_sibling_path() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("billing.py");
    let code = "def billing_country(order: dict | None) -> str:\n    return \"US\"";
    std::fs::write(&source_path, code).unwrap();

    let tests = "from billing import billing_country\nassert billing_country(None) == \"US\"";
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn verify_with_threshold_adds_stage() {
    let code = "def complex_fn(x: int) -> int:\n    if x > 0:\n        for i in range(x):\n            if i > 5:\n                return i\n    return x";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: Some(3),
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;
    assert!(
        report.stages.iter().any(|s| s.name == "complexity"),
        "should have complexity stage"
    );
}

#[tokio::test]
async fn verify_complexity_threshold_scopes_to_changed_functions_in_diff_mode() {
    let code = "\
def legacy_complex(x: int) -> int:
    if x > 0:
        for i in range(x):
            if i > 5:
                return i
    return x

def changed(x: int) -> int:
    return x + 1
";
    let diff = "@@ -8,2 +8,2 @@\n+def changed(x: int) -> int:\n+    return x + 1\n";
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: Some(3),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: Some(diff),
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|s| s.name == "complexity")
        .expect("complexity stage should be present");
    assert!(
        complexity_stage.status == StageStatus::Passed,
        "only the changed simple function should be checked in diff mode"
    );
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["checked_functions"].as_u64(), Some(1));
    assert_eq!(detail["diff_scoped"].as_bool(), Some(true));
}

#[tokio::test]
async fn verify_complexity_stage_reports_cognitive_and_breakdown_details() {
    let code = "\
def classify(x: int) -> str:
    match x:
        case 0:
            return \"zero\"
        case 1:
            return \"one\"
        case _:
            return \"other\"
";
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: Some(2),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|s| s.name == "complexity")
        .expect("complexity stage should be present");
    assert!(
        complexity_stage.status != StageStatus::Passed,
        "match/case should exceed threshold 2"
    );

    let violations = complexity_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("violations"))
        .and_then(|value| value.as_array())
        .expect("violations should be present");
    assert_eq!(violations.len(), 1);
    assert!(
        violations[0]["cognitive_complexity"].as_u64().unwrap_or(0) > 0,
        "violation should include cognitive complexity"
    );
    assert_eq!(
        violations[0]["complexity_breakdown"]["case"].as_u64(),
        Some(3)
    );
}

#[tokio::test]
async fn verify_can_gate_on_cognitive_complexity() {
    let code = r#"
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: Some(5),
            complexity_metric: ComplexityMetric::Cognitive,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
        .expect("complexity stage should be present");
    assert!(
        complexity_stage.status != StageStatus::Passed,
        "cognitive complexity should exceed threshold 5"
    );
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["metric"].as_str(), Some("cognitive"));
    let violations = detail["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["function"].as_str(), Some("check_access"));
}

#[tokio::test]
async fn verify_without_threshold_no_stage() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert!(
        !report.stages.iter().any(|s| s.name == "complexity"),
        "should NOT have complexity stage"
    );
}

#[test]
fn parse_findings_from_stdout() {
    let stdout = r#"FUZZ greet: 30 passed, 0 rejected (of 30)
__COURT_JESTER_FINDINGS_JSON__
[{"id":"execute:boom:1","severity":"crash","confidence":"high","category":"exception","location":{"source_file":"<inline>","function":"boom","line":1,"invocation_path":"direct"},"oracle":{"id":"runtime_contract:boom","kind":"runtime_contract","provenance":"language_runtime","confidence":"high"},"input_classification":"valid","repro":{"kind":"function_call","function":"boom","arguments":[{"expression":"42","json_value":42}],"snippet":"boom(42)","expectation":{"severity":"crash","oracle_kind":"runtime_contract","category":"exception"}},"minimization":{"status":"not_needed","attempts":0,"original":{"arguments":[{"expression":"42","json_value":42}]}},"error_type":"TypeError","message":"bad"}]
"#;
    let findings = parse_findings(stdout);
    assert!(findings.is_some());
    let findings = findings.unwrap();
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.location.function, "boom");
    assert_eq!(finding.severity, FindingSeverity::Crash);
    assert_eq!(finding.confidence, FindingConfidence::High);
    assert_eq!(finding.category, FindingCategory::Exception);
    assert_eq!(finding.oracle.kind, OracleKind::RuntimeContract);
    assert_eq!(finding.oracle.provenance, OracleProvenance::LanguageRuntime);
    assert_eq!(finding.input_classification, InputClassification::Valid);
    let events = [
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 0,
            "event": "bootstrap_started"
        }),
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 1,
            "event": "target_resolved",
            "data": {"module": "generated"}
        }),
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 2,
            "event": "target_ready"
        }),
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 3,
            "event": "unit_started",
            "data": {
                "surface_id": "boom:1",
                "iteration": 0,
                "input_classification": "valid",
                "input_origin": "generated"
            }
        }),
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 4,
            "event": "finding",
            "data": {"finding": finding}
        }),
        serde_json::json!({
            "protocol_version": 1,
            "sequence": 5,
            "event": "unit_completed",
            "data": {
                "surface_id": "boom:1",
                "iteration": 0,
                "outcome": "target_exception"
            }
        }),
    ];
    let event_stdout = format!(
        "{}\n__COURT_JESTER_FINDINGS_JSON__\n[truncated",
        events
            .iter()
            .map(|event| format!("__COURT_JESTER_EVENT_JSON__{event}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let event_findings =
        parse_findings(&event_stdout).expect("event findings must survive a truncated aggregate");
    assert_eq!(event_findings.len(), 1);
    assert_eq!(event_findings[0].location.function, "boom");
}

#[test]
fn parse_findings_no_sentinel() {
    let stdout = "FUZZ greet: 30 passed, 0 rejected (of 30)\nAll fuzz tests passed\n";
    assert!(parse_findings(stdout).is_none());
}

#[tokio::test]
async fn verify_diff_mode_only_fuzzes_changed() {
    // Two functions, diff only touches the second one
    let code = "\
def untouched(x: int) -> int:
    return x

def changed(x: int) -> int:
    return x + 1
";
    // Diff touching lines 4-5 (the changed function)
    let diff = "@@ -4,2 +4,2 @@\n+def changed(x: int) -> int:\n+    return x + 1\n";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: Some(diff),
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;
    // Should pass since changed() is a simple function
    if let Some(exec) = report.stages.iter().find(|s| s.name == "execute") {
        // The fuzz should only test the changed function
        let detail = exec.detail.as_ref().unwrap();
        let stdout = detail["stdout"].as_str().unwrap_or("");
        // untouched should NOT appear in fuzz output
        assert!(
            !stdout.contains("FUZZ untouched"),
            "untouched should not be fuzzed in diff mode, got: {stdout}"
        );
    }
    let coverage = report
        .stages
        .iter()
        .find(|s| s.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage stage should be present");
    assert_eq!(coverage["diff_scoped"].as_bool(), Some(true));
}

#[tokio::test]
async fn writes_report_to_output_dir() {
    let dir = tempfile::tempdir().unwrap();
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.report_path.is_some(), "should have report_path");
    let path = report.report_path.unwrap();
    assert!(
        std::path::Path::new(&path).exists(),
        "report file should exist"
    );

    // Verify it's valid JSON with expected structure
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        parsed
            .get("schema_version")
            .and_then(|value| value.as_u64()),
        Some(3)
    );
    assert!(parsed.get("meta").is_some());
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("stages").is_some());
    assert!(parsed.get("verdict").is_some());
}

#[tokio::test]
async fn minimal_report_level_omits_full_parse_detail() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let mut opts = default_opts(None);
    opts.report_level = ReportLevel::Minimal;
    let report = verify(code, &Language::Python, opts).await;
    let json = report_json_value(&report, ReportLevel::Minimal);

    assert_eq!(json["schema_version"].as_u64(), Some(3));
    assert!(json.get("summary").is_some());

    let parse_stage = json["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage.get("name").and_then(|value| value.as_str()) == Some("parse"))
        })
        .expect("parse stage should be present");
    assert!(
        parse_stage.get("detail").is_none(),
        "minimal report should omit full parse detail: {parse_stage:?}"
    );
}

#[tokio::test]
async fn no_report_without_output_dir() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert!(
        report.report_path.is_none(),
        "should NOT have report_path when output_dir not set"
    );
}

#[tokio::test]
async fn unclassified_only_fuzz_run_is_not_counted_as_pass_in_report_summary() {
    let dir = tempfile::tempdir().unwrap();
    let code = "class ValidationError(Exception):\n    pass\n\ndef always_reject(x: int) -> int:\n    raise ValidationError('nope')";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Inconclusive,
        "rejected-only fuzz run should be diagnostic only"
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(
        execute_stage.status,
        StageStatus::Inconclusive,
        "an all-rejected surface is explicitly inconclusive"
    );
    let execute_detail = execute_stage.detail.as_ref().unwrap();
    assert_eq!(execute_detail["no_inputs_reached"].as_u64(), Some(1));

    let path = report.report_path.unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let summary = parsed.get("summary").unwrap();
    assert_eq!(summary["functions_fuzzed"].as_u64(), Some(1));
    assert_eq!(summary["coverage"]["required"].as_u64(), Some(1));
    assert_eq!(
        summary["coverage"]["behaviorally_checked"].as_u64(),
        Some(0)
    );
    assert_eq!(summary["coverage"]["reached_only"].as_u64(), Some(1));
    assert_eq!(summary["coverage"]["no_inputs_reached"].as_u64(), Some(1));
    assert_eq!(summary["findings"]["total"].as_u64(), Some(1));
    assert_eq!(summary["findings"]["gating"].as_u64(), Some(0));
}

#[tokio::test]
async fn execute_gate_crash_allows_property_violations() {
    let code = r#"
export function compareScore(a: number, b: number): number {
  return 1;
}
"#;

    let report_default = verify(code, &Language::TypeScript, default_opts(None)).await;
    assert!(
        matches!(
            report_default.verdict,
            VerificationVerdict::Fail | VerificationVerdict::Inconclusive
        ),
        "the all-findings gate must never pass an observed property violation: {:#?}",
        report_default.stages
    );
    let default_execute = report_default
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("default execute stage should be present");
    assert!(matches!(
        default_execute.status,
        StageStatus::Failed | StageStatus::Inconclusive
    ));
    let default_stdout = default_execute.detail.as_ref().unwrap()["execution"]["stdout"]
        .as_str()
        .expect("harness stdout should be present");
    assert!(
        default_stdout.contains("Comparator self-compare should be zero"),
        "the property violation must remain visible under the default gate"
    );

    let mut crash_only_opts = default_opts(None);
    crash_only_opts.execute_gate = ExecuteGate::Crash;
    let report = verify(code, &Language::TypeScript, crash_only_opts).await;
    assert_eq!(report.verdict, VerificationVerdict::Pass,
        "a completed, reproducible property violation is retained but not gated by crash-only policy: {:#?}", report.stages);

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(
        execute_stage.status,
        StageStatus::Passed,
        "the selected crash gate is satisfied while property findings remain visible"
    );
    let stdout = execute_stage.detail.as_ref().unwrap()["execution"]["stdout"]
        .as_str()
        .expect("harness stdout should be present");
    assert!(
        stdout.contains("Comparator self-compare should be zero"),
        "crash-only gating must retain the property violation content"
    );
}

#[tokio::test]
async fn execute_findings_can_be_suppressed_without_failing_verify() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("first_char.py");
    let code = "def first_char(s: str) -> str:\n    return s[0]\n";
    std::fs::write(&source_path, code).unwrap();

    let suppressions = r#"
{
  "rules": [
    {
      "path": "first_char.py",
      "stage": "execute",
      "function": "first_char",
      "severity": "crash",
      "error_type": "IndexError"
    }
  ]
}
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: Some(suppressions),
            suppression_source: Some(".court-jester-ignore.json"),
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "suppressed execute finding should not fail verify"
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.status == StageStatus::Passed,
        "execute stage should stay green when all findings are suppressed"
    );
    let detail = execute_stage.detail.as_ref().unwrap();
    assert!(detail.get("suppression_source").is_none());
    assert_eq!(detail["findings_summary"]["gating"].as_u64(), Some(0));
    assert!(
        detail["findings_summary"]["suppressed"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
    let suppressed = detail["suppressed_findings"].as_array().unwrap();
    assert!(!suppressed.is_empty(), "expected suppressed findings");
    assert_eq!(
        suppressed[0]["location"]["function"].as_str(),
        Some("first_char")
    );
    assert!(report.summary.findings.suppressed > 0);
}

#[tokio::test]
async fn complexity_violations_can_be_suppressed_by_function_name() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("authz.py");
    let code = r#"
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
    std::fs::write(&source_path, code).unwrap();
    let suppressions = r#"
{
  "rules": [
    {
      "path": "authz.py",
      "stage": "complexity",
      "function": "check_access"
    }
  ]
}
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: Some(2),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: Some(suppressions),
            suppression_source: Some(".court-jester-ignore.json"),
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "suppressed complexity violation should not fail verify"
    );
    let complexity_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
        .expect("complexity stage should be present");
    assert!(complexity_stage.status == StageStatus::Passed);
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["violations"].as_array().unwrap().len(), 0);
    assert_eq!(detail["suppressed_violations"].as_array().unwrap().len(), 1);
    assert_eq!(report.summary.suppressed_complexity_violations, 1);
}

#[tokio::test]
async fn complexity_violations_can_be_suppressed_by_source_directive() {
    let code = r#"
# court-jester-ignore complexity
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: Some(2),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "source directive suppression should not fail verify"
    );
    let complexity_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
        .expect("complexity stage should be present");
    assert!(complexity_stage.status == StageStatus::Passed);
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["violations"].as_array().unwrap().len(), 0);
    assert_eq!(detail["suppressed_violations"].as_array().unwrap().len(), 1);
    assert_eq!(
        detail["source_directive_suppression_count"].as_u64(),
        Some(1)
    );
    assert_eq!(
        detail["source_directive_functions"].as_array().unwrap()[0]
            .as_str()
            .unwrap(),
        "check_access"
    );
    assert_eq!(report.summary.suppressed_complexity_violations, 1);
}

#[tokio::test]
async fn uncontracted_value_error_is_retained_as_an_uncertain_observation() {
    let code = "def normalize_timezone(value: str) -> str:\n    raise ValueError('invalid timezone offset')";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "unclassified exceptions must not yield a pass"
    );

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        exec_stage.status != StageStatus::Passed,
        "unclassified exception needs contract evidence"
    );

    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(
            |failure| failure.get("error_type").and_then(|value| value.as_str())
                == Some("ValueError")
        ),
        "expected ValueError finding, got: {failures:?}"
    );
}

#[tokio::test]
async fn declared_properties_can_trigger_typescript_property_failures() {
    let code = r#"
// court-jester-properties sorted permutation
export function reorder(values: string[]): string[] {
    return [...values].reverse();
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "declared sorted property should fail on non-sorted output"
    );

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should exist");
    let failures = execute_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(|failure| {
            failure["location"]["function"].as_str() == Some("reorder")
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Not sorted"))
        }),
        "declared sorted property should surface a property violation"
    );
}

#[tokio::test]
async fn declared_monotonicity_failure_is_authoritative() {
    let code = r#"
// court-jester-properties monotonic
export function negate(value: number): number {
    return -value;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Monotonicity violated"))
            })
        })
        .unwrap_or_else(|| panic!("monotonicity finding missing: {report:#?}"));

    assert_eq!(
        finding["oracle"]["kind"].as_str(),
        Some("declared_property")
    );
    assert_eq!(finding["confidence"].as_str(), Some("authoritative"));
    assert_eq!(finding["category"].as_str(), Some("property"));
}

#[tokio::test]
async fn declared_python_involution_failure_is_authoritative() {
    let code = "\
# court-jester-properties involutive
def increment(value: int) -> int:
    return value + 1
";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Involution violated"))
            })
        })
        .unwrap_or_else(|| panic!("involution finding missing: {report:#?}"));

    assert_eq!(
        finding["oracle"]["kind"].as_str(),
        Some("declared_property")
    );
    assert_eq!(finding["confidence"].as_str(), Some("authoritative"));
}

#[tokio::test]
async fn inferred_roundtrip_failure_emits_structured_advisory() {
    let code = r#"
export function encode(value: string): string {
    return value + "x";
}
export function decode(value: string): string {
    return value;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Roundtrip failed"))
            })
        })
        .unwrap_or_else(|| panic!("roundtrip finding missing: {report:#?}"));

    assert_eq!(
        finding["oracle"]["kind"].as_str(),
        Some("inferred_semantic")
    );
    assert_eq!(finding["confidence"].as_str(), Some("low"));
    assert_eq!(finding["category"].as_str(), Some("property"));
}

#[tokio::test]
async fn exported_object_literal_methods_can_fail_verify() {
    let code = r#"
export const reorderer = {
    // court-jester-properties sorted permutation
    reorder(values: string[]): string[] {
        return [...values].reverse();
    },
};
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "exported object literal method should be invoked by verify"
    );
    let failures = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(|failure| {
            failure["location"]["function"].as_str() == Some("reorderer.reorder")
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Not sorted"))
        }),
        "exported object literal method should produce a property violation"
    );
}

#[tokio::test]
async fn exported_zero_arg_class_methods_can_fail_verify() {
    let code = r#"
export class Reorderer {
    // court-jester-properties sorted permutation
    reorder(values: string[]): string[] {
        return [...values].reverse();
    }
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "exported zero-arg class method should be invoked by verify"
    );
    let failures = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(|failure| {
            failure["location"]["function"].as_str() == Some("Reorderer#reorder")
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Not sorted"))
        }),
        "exported zero-arg class method should produce a property violation"
    );
}

#[tokio::test]
async fn factory_returned_methods_appear_in_coverage() {
    let code = r#"
export function createReorderer() {
    function reorder(values: string[]): string[] {
        return [...values].reverse();
    }
    return { reorder };
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let coverage_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .expect("coverage stage should exist");
    let functions = coverage_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage functions should be present");
    assert!(
        functions.iter().any(|function| {
            function["function"].as_str() == Some("createReorderer().reorder")
                && function["status"].as_str() == Some("checked_via_factory")
        }),
        "factory-returned callables should be explicit in coverage output"
    );
}

#[tokio::test]
async fn typescript_factory_action_sequence_finds_second_step_crash() {
    let code = r#"
export function createCounter() {
    let calls = 0;
    function push(value: number): number {
        calls += 1;
        if (calls === 2) {
            throw new ReferenceError("stateful second-step crash");
        }
        return value;
    }
    return { push };
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("stateful second-step crash"))
            })
        })
        .unwrap_or_else(|| panic!("stateful TypeScript finding missing: {report:#?}"));

    assert_eq!(
        finding["location"]["function"].as_str(),
        Some("createCounter().push")
    );
    assert!(finding["repro"]["case_label"]
        .as_str()
        .is_some_and(|label| label.contains("push")));
}

#[tokio::test]
async fn python_factory_action_sequence_finds_second_step_crash() {
    let code = "\
def create_counter():
    calls = 0
    def push(value: int) -> int:
        nonlocal calls
        calls += 1
        if calls == 2:
            raise ValueError('stateful second-step crash')
        return value
    return {'push': push}
";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("stateful second-step crash"))
            })
        })
        .unwrap_or_else(|| panic!("stateful Python finding missing: {report:#?}"));

    assert!(finding["repro"]["case_label"]
        .as_str()
        .is_some_and(|label| label.contains("push")));
}

#[tokio::test]
async fn changed_factory_signature_keeps_unchanged_action_declarations() {
    let code = "export function createCounter() {\n let calls = 0;\n function push(value: number): number {\n  calls += 1;\n  if (calls === 2) throw new ReferenceError('second action failed');\n  return value;\n }\n return { push };\n}\n";
    let diff = "@@ -1,1 +1,1 @@\n-export function createCounter(old?: number) {\n+export function createCounter() {\n";
    let mut options = default_opts(None);
    options.diff = Some(diff);
    let report = verify(code, &Language::TypeScript, options).await;
    let findings = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .expect("execute findings");
    assert!(
        findings.iter().any(
            |finding| finding["location"]["function"] == "createCounter().push"
                && finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("second action failed"))
        ),
        "changed factory lost its unchanged action context: {findings:?}"
    );
}

#[tokio::test]
async fn zustand_style_container_methods_can_fail_verify() {
    let code = r#"
function create<T>(initializer: (set: unknown, get: unknown) => T) {
    let state!: T;
    const get = () => state;
    const set = (_value: unknown) => {};
    state = initializer(set, get);
    return {
        getState(): T {
            return state;
        },
    };
}

export const useReorderer = create(() => ({
    // court-jester-properties sorted permutation
    reorder(values: string[]): string[] {
        return [...values].reverse();
    },
}));
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict != VerificationVerdict::Pass,
        "container surfaced method should be invoked by verify"
    );

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let surfaced = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("useReorderer.reorder")
        })
        .expect("container surfaced method coverage should be present");
    assert_eq!(
        surfaced.get("status").and_then(|value| value.as_str()),
        Some("checked_direct")
    );

    let failures = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(|failure| {
            failure["location"]["function"].as_str() == Some("useReorderer.reorder")
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Not sorted"))
        }),
        "container surfaced method should produce a property violation"
    );
}

#[tokio::test]
async fn typescript_malformed_uri_is_uncertain_without_an_input_contract() {
    let code = r#"
export function decodeSegment(value: string): string {
    return decodeURIComponent(value);
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Inconclusive,
        "the exception alone cannot establish whether the API promises to accept malformed input: {:#?}",
        report.stages
    );

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert_eq!(
        exec_stage.status,
        StageStatus::Inconclusive,
        "unspecified URI admission should remain inconclusive: {:?}",
        exec_stage.message
    );

    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("findings"))
        .and_then(|findings| findings.as_array())
        .expect("schema-v3 execute detail should contain typed findings");
    assert!(
        !failures.is_empty()
            && failures
                .iter()
                .all(|finding| finding["input_classification"] == "unknown"),
        "URI exceptions must be retained without claiming an admitted crash: {failures:?}"
    );
    assert_eq!(report.summary.findings.gating, 0);
}

#[tokio::test]
async fn verify_reports_per_function_fuzz_coverage_honestly() {
    let code = r#"
export function verifyRequest(request: Request): boolean {
  return request.headers.has("authorization");
}

function parseSignatureHeader(header: string): Record<string, string> {
  return Object.fromEntries(
    header
      .split(",")
      .filter(Boolean)
      .map((part, index) => [`v${index}`, part.trim()]),
  );
}

function encodePair(left: string, right: string): string {
  return `${left}:${right}`;
}

function unresolved(value: MissingThing): string {
  return String(value);
}

function _privateToken(): string {
  return "token";
}

class Reader {
  read(headers: Headers): string {
    return headers.get("authorization") ?? "";
  }
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");

    let status_for = |name: &str| {
        coverage
            .iter()
            .find(|entry| entry.get("function").and_then(|value| value.as_str()) == Some(name))
            .and_then(|entry| entry.get("status"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
    };

    assert_eq!(status_for("verifyRequest"), "checked_direct");
    assert_eq!(status_for("parseSignatureHeader"), "checked_direct");
    assert_eq!(status_for("encodePair"), "skipped_internal_helper");
    assert_eq!(status_for("unresolved"), "skipped_unsupported_type");
    assert_eq!(status_for("_privateToken"), "skipped_private_name");
    assert_eq!(status_for("read"), "skipped_method");
}

#[tokio::test]
async fn typescript_url_helper_never_receives_a_plain_object_as_valid_input() {
    let code = r#"
function normalizedHostname(url: URL): string {
  return url.hostname.startsWith("[") && url.hostname.endsWith("]")
    ? url.hostname.slice(1, -1)
    : url.hostname;
}

export async function validateDocumentFetchUrl(value: string | URL): Promise<string> {
  const url = new URL(value);
  return normalizedHostname(url);
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let coverage_detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage stage detail");
    let coverage = coverage_detail
        .get("functions")
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let helper_input = coverage_detail["verification_plan"]["inputs"]
        .as_array()
        .and_then(|inputs| {
            inputs.iter().find(|input| {
                input["surface_id"]
                    .as_str()
                    .is_some_and(|surface| surface.starts_with("normalizedHostname:"))
            })
        })
        .expect("the URL helper should have a planned URL input");
    assert_eq!(
        helper_input["classification"], "valid",
        "the constructible URL expression must be classified as valid"
    );
    let helper_expression = helper_input["arguments"]["positional"][0]["expression"]
        .as_str()
        .expect("planned URL expression");
    assert!(
        helper_expression.starts_with("new URL(") && helper_expression != "{}",
        "URL planning must use a constructible platform value, got {helper_expression}"
    );
    let helper_status = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("normalizedHostname")
        })
        .and_then(|entry| entry.get("status"))
        .and_then(|value| value.as_str())
        .expect("normalizedHostname coverage status");
    assert!(
        matches!(
            helper_status,
            "checked_direct" | "skipped_unsupported_type" | "skipped_internal_helper"
        ),
        "URL helper must execute with a URL or be skipped as unsupported: {helper_status}"
    );
    assert!(
        report.diagnostics.iter().all(|diagnostic| {
            diagnostic.domain != FailureDomain::TargetCode
                || diagnostic.impact != DiagnosticImpact::Gating
        }),
        "a generated plain object must not become a gating URL crash: {report:#?}"
    );
    if helper_status == "checked_direct" {
        let execute = report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .expect("execute stage");
        assert_eq!(
            execute.status,
            StageStatus::Passed,
            "the checked URL helper must receive constructible URL instances: {report:#?}"
        );
    }
}

#[tokio::test]
async fn zero_arg_object_getter_is_classified_as_no_fuzzable_surface() {
    let code = r#"
export function ensureScraper(): { enabled: boolean } {
  return process.env.SCRAPER_TOKEN ? { enabled: true } : { enabled: false };
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.verdict == VerificationVerdict::Inconclusive,
        "report: {:#?}",
        report.stages
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("no-fuzzable-surface verification should report skipped execution");
    assert_eq!(execute.status, StageStatus::Skipped);
    let execute_detail = execute
        .detail
        .as_ref()
        .expect("skipped execution should explain why no cases ran");
    assert_eq!(
        execute_detail["reason"].as_str(),
        Some("no_fuzzable_targets")
    );
    assert_eq!(execute_detail["generated_cases"].as_u64(), Some(0));
    let minimal = report_json_value(&report, ReportLevel::Minimal);
    let minimal_execute = minimal["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["name"] == "execute")
        .expect("minimal report should retain the execute stage");
    assert_eq!(minimal_execute["status"].as_str(), Some("skipped"));
    assert_eq!(
        minimal_execute["detail"]["reason"].as_str(),
        Some("no_fuzzable_targets")
    );
    assert_eq!(
        minimal_execute["detail"]["generated_cases"].as_u64(),
        Some(0)
    );

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let ensure_scraper = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("ensureScraper")
        })
        .expect("ensureScraper coverage should be present");
    assert_eq!(
        ensure_scraper
            .get("status")
            .and_then(|value| value.as_str()),
        Some("skipped_no_fuzzable_surface")
    );
}

#[tokio::test]
async fn zero_arg_primitive_helper_can_still_be_fuzzed() {
    let code = r#"
export function buildVersion(): number {
  return 42;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let build_version = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("buildVersion")
        })
        .expect("buildVersion coverage should be present");
    assert_eq!(
        build_version.get("status").and_then(|value| value.as_str()),
        Some("checked_direct")
    );
}

#[tokio::test]
async fn crash_can_be_classified_as_type_signature_wider_than_usage() {
    let code = r#"
export function jsonResponse(status: number): string {
  const statusText: Record<number, string> = { 200: "OK", 201: "Created" };
  return statusText[status].trim();
}

jsonResponse(200);
jsonResponse(201);
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    let failures = execute_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    assert!(
        failures.iter().any(|failure| {
            failure
                .get("classification")
                .and_then(|value| value.as_str())
                == Some("type_signature_wider_than_usage")
        }),
        "expected a wide-type classification in: {failures:#?}"
    );
    let classified = failures
        .iter()
        .find(|failure| {
            failure
                .get("classification")
                .and_then(|value| value.as_str())
                == Some("type_signature_wider_than_usage")
        })
        .unwrap();
    assert!(
        classified
            .get("suggestion")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .contains("200, 201"),
        "expected observed literal suggestion, got: {classified:#?}"
    );
}

#[tokio::test]
async fn verify_separates_portability_warning_from_execute_success() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bun.lock"), "").unwrap();
    std::fs::write(dir.path().join("helper.ts"), "export const value = 7;\n").unwrap();

    let source_path = dir.path().join("main.ts");
    let code = r#"
import { value } from "./helper";

export function add(input: number): number {
  return input + value;
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );

    let portability_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "portability")
        .expect("portability stage should be present");
    assert!(
        portability_stage.status != StageStatus::Passed,
        "portability stage should record the Node warning"
    );
    let portability_detail = portability_stage
        .detail
        .as_ref()
        .expect("portability stage should include details");
    assert_eq!(
        portability_detail["reason"].as_str(),
        Some("err_module_not_found")
    );
    assert!(
        portability_detail["failing_imports"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("helper")),
        "expected failing import list to include helper"
    );
    assert!(
        portability_detail["fix_hint"]
            .as_str()
            .unwrap_or("")
            .contains("explicit Node ESM file extensions"),
        "expected a Node ESM fix hint"
    );
    let node_stderr = portability_detail["node_result"]["stderr"]
        .as_str()
        .unwrap_or("");
    assert!(
        node_stderr.contains("ERR_MODULE_NOT_FOUND"),
        "expected Node module resolution warning, got: {node_stderr}"
    );

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.status == StageStatus::Passed,
        "execute stage should succeed: {:?}",
        execute_stage.message
    );
    let runtime = execute_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("runtime"))
        .and_then(|value| value.as_str());
    assert_eq!(runtime, Some("bun"));
}

#[tokio::test]
async fn auto_seed_uses_nearby_test_literals_for_execute_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("host_label.ts");
    let test_path = tests_dir.join("host_label.test.ts");
    let code = r#"
export function hostLabel(url: string): string {
  if (!url.startsWith("https://")) {
    throw new Error("invalid base url");
  }
  return new URL(url).host;
}
"#;
    let test_code = r#"
import { hostLabel } from "../src/host_label.ts";

hostLabel("https://example.com");
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, test_code).unwrap();

    let report_seeded = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;
    let seeded_execute = report_seeded
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert_eq!(seeded_execute["no_inputs_reached"].as_u64(), Some(0));
    assert!(
        seeded_execute["seed_input_count"].as_u64().unwrap_or(0) > 0,
        "expected seeded inputs in execute detail"
    );
    assert!(
        seeded_execute["seed_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str() == Some(test_path.to_string_lossy().as_ref())),
        "expected nearby test path in seed sources"
    );

    let report_unseeded = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: false,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;
    let unseeded_execute = report_unseeded
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert_eq!(unseeded_execute["no_inputs_reached"].as_u64(), Some(0));
    assert_eq!(unseeded_execute["seed_input_count"].as_u64(), Some(0));
}

#[tokio::test]
async fn auto_seed_uses_project_production_calls_for_execute_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let source_path = src_dir.join("host_label.ts");
    let route_path = src_dir.join("routes.ts");
    let code = r#"
export function hostLabel(url: string): string {
  if (!url.startsWith("https://")) {
    throw new Error("invalid base url");
  }
  return new URL(url).host;
}
"#;
    let route_code = r#"
import * as labels from "./host_label";

export const defaultHost = labels.hostLabel("https://example.com");
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&route_path, route_code).unwrap();

    let report = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert_eq!(execute["no_inputs_reached"].as_u64(), Some(0));
    assert!(
        execute["seed_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str() == Some(route_path.to_string_lossy().as_ref())),
        "expected production project file in seed sources"
    );
}

#[tokio::test]
async fn auto_seed_accepts_project_object_literal_calls() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let source_path = src_dir.join("location.ts");
    let caller_path = src_dir.join("profile_view.ts");
    let code = r#"
export interface Profile {
  address?: { city?: string };
}

export function primaryCity(profile: Profile): string {
  if (!profile.address?.city) {
    return "Unknown";
  }
  return profile.address.city;
}
"#;
    let caller_code = r#"
import { primaryCity } from "./location";

export const previewCity = primaryCity({ address: { city: "Boise" } });
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&caller_path, caller_code).unwrap();

    let report = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert!(
        execute["seed_input_count"].as_u64().unwrap_or(0) > 0,
        "expected object literal call to become a seed row"
    );
    assert!(
        execute["seed_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str() == Some(caller_path.to_string_lossy().as_ref())),
        "expected object literal caller in seed sources"
    );
}

#[tokio::test]
async fn auto_seed_uses_json_fixture_inputs_as_domain_examples() {
    let dir = tempfile::tempdir().unwrap();
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = dir.path().join("first_item.py");
    let fixture_path = tests_dir.join("first_item.json");
    let code = r#"
def first_item(items):
    return items[0]
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&fixture_path, "[[[1, 2, 3]], 1]\n").unwrap();

    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.status == StageStatus::Passed,
        "fixture-shaped fuzz inputs should avoid arbitrary non-list crashes: {:#?}",
        execute_stage
    );
    let detail = execute_stage
        .detail
        .as_ref()
        .expect("execute detail should be present");
    assert!(
        detail["seed_input_count"].as_u64().unwrap_or(0) > 0,
        "expected JSON fixture inputs to seed fuzzing"
    );
    assert!(
        detail["seed_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str() == Some(fixture_path.to_string_lossy().as_ref())),
        "expected JSON fixture path in seed sources"
    );
}

#[tokio::test]
async fn json_fixture_outputs_infer_structural_properties_not_exact_oracles() {
    let dir = tempfile::tempdir().unwrap();
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = dir.path().join("sort_items.py");
    let fixture_path = tests_dir.join("sort_items.json");
    let code = r#"
def sort_items(items):
    return items
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        &fixture_path,
        "[[[3, 1, 2]], [1, 2, 3]]\n[[[4, 2, 3, 1]], [1, 2, 3, 4]]\n",
    )
    .unwrap();

    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.status != StageStatus::Passed,
        "fixture-derived sorted/permutation properties should catch unsorted output"
    );
    let detail = execute_stage
        .detail
        .as_ref()
        .expect("execute detail should be present");
    assert_eq!(
        detail["inferred_fixture_properties"]["sort_items"]
            .as_array()
            .map(|values| values
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()),
        Some(vec!["sorted", "permutation"])
    );
}

#[tokio::test]
async fn json_fixture_single_row_does_not_infer_structural_properties() {
    let dir = tempfile::tempdir().unwrap();
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = dir.path().join("sort_items.py");
    let fixture_path = tests_dir.join("sort_items.json");
    let code = r#"
def sort_items(items):
    return items
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&fixture_path, "[[[3, 1, 2]], [1, 2, 3]]\n").unwrap();

    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.status == StageStatus::Passed,
        "one fixture row should shape inputs but should not create a hard sorted oracle"
    );
    let detail = execute_stage
        .detail
        .as_ref()
        .expect("execute detail should be present");
    assert!(
        detail["inferred_fixture_properties"]["sort_items"].is_null(),
        "single fixture row should not infer structural properties"
    );
}

#[tokio::test]
async fn findings_preserve_executable_inputs_and_truncate_display_messages() {
    let code =
        "def explode(name: str) -> str:\n    if len(name) < 1000:\n        return name\n    raise TypeError('x' * 500)";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("findings"))
        .and_then(|value| value.as_array())
        .expect("findings should be present");
    let first = failures.first().expect("expected at least one finding");

    let input = first["repro"]["arguments"][0]
        .get("expression")
        .and_then(|value| value.as_str())
        .expect("finding repro argument expression should be present");
    let message = first
        .get("message")
        .and_then(|value| value.as_str())
        .expect("failure message should be present");

    assert!(
        input.len() > 1000 && !input.contains("[truncated "),
        "executable input must not contain a display truncation marker"
    );
    assert!(
        message.len() <= 270 && message.contains("[truncated "),
        "expected truncated message, got: {message}"
    );
}

#[tokio::test]
async fn typescript_test_stage_can_import_source_module_from_test_file() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("handle.ts");
    let test_path = tests_dir.join("court_jester_public_verify.ts");
    let code = r#"
export function displayHandle(user?: { profile?: { handle?: string | null } | null, username?: string | null } | null): string {
  const handle = user?.profile?.handle?.trim();
  if (handle) return handle.toLowerCase();
  const username = user?.username?.trim();
  if (username) return username.toLowerCase();
  return "guest";
}
"#;
    let tests = r#"
import assert from "node:assert/strict";
import { displayHandle } from "../src/handle.ts";

assert.equal(displayHandle({ profile: { handle: " Admin " }, username: "root" }), "admin");
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_test_stage_auto_prefers_bun_for_bun_test_imports() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    let bun_log = dir.path().join("bun.log");
    let node_log = dir.path().join("node.log");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        &format!(
            "#!/bin/sh\nprintf 'runner=bun\\n' > \"{}\"\nfor arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\" >> \"{}\"; done\nif [ \"$1\" != \"test\" ]; then printf 'expected bun test subcommand first\\n' >&2; exit 2; fi\nexit 0\n",
            bun_log.display(),
            bun_log.display(),
        ),
    );
    install_fake_tool_at(
        &tool_dir,
        "node",
        &format!(
            "#!/bin/sh\nprintf 'runner=node\\n' > \"{}\"\nexit 1\n",
            node_log.display(),
        ),
    );
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("math.ts");
    let test_path = tests_dir.join("unit.test.ts");
    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let tests = r#"
import { test, expect } from "bun:test";
import { add } from "../src/math.ts";

test("add", () => {
  expect(add(2, 3)).toBe(5);
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let report = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: Some(tests),
            test_source_file: Some(test_path.to_str().unwrap()),
            base_code: None,
            base_source_file: None,
            base_project_dir: None,
            test_runner: TestRunner::Auto,
            tests_only: true,
            test_quality_max_mutants: None,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
            coverage_gate: CoverageGate::ChangedExports,
            inferred_oracle_gate: InferredOracleGate::Advisory,
            runtime_profile: RuntimeProfile::LocalTrusted,
            python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
            typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
            memory_mb: 512,
            network: NetworkPolicy::Deny,
            harness_args: vec![],
        },
    )
    .await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "a no-op Bun fake cannot prove same-process target entry: {:#?}",
        report.stages
    );
    let test_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage should be present");
    assert!(
        test_stage.status == StageStatus::Passed,
        "test stage should pass: {:?}",
        test_stage.message
    );
    let detail = test_stage.detail.as_ref().unwrap();
    assert_eq!(detail["test_runner_requested"].as_str(), Some("auto"));
    assert_eq!(detail["test_runner_selected"].as_str(), Some("bun"));
    assert_eq!(
        detail["authoritative_test_covered_surfaces"].as_u64(),
        Some(0)
    );
    assert!(detail["target_entered_surfaces"]
        .as_array()
        .is_some_and(Vec::is_empty));
    let coverage_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .expect("coverage stage should be present");
    assert_eq!(coverage_stage.status, StageStatus::Inconclusive);
    assert_eq!(
        coverage_stage.detail.as_ref().unwrap()["required_surface_count"].as_u64(),
        Some(1)
    );
    assert_eq!(
        coverage_stage.detail.as_ref().unwrap()["observed_required_surface_count"].as_u64(),
        Some(0)
    );

    let bun_log_text = std::fs::read_to_string(&bun_log).unwrap();
    assert!(
        bun_log_text.contains("runner=bun"),
        "expected bun runner log, got: {bun_log_text}"
    );
    let mut bun_args = bun_log_text
        .lines()
        .filter_map(|line| line.strip_prefix("arg="));
    assert_eq!(
        bun_args.next(),
        Some("test"),
        "Bun must receive its test subcommand before preload flags, got: {bun_log_text}"
    );
    assert!(
        bun_args.any(|argument| argument == "--preload"),
        "the default deny-network policy must still preload its guard, got: {bun_log_text}"
    );
    assert!(
        !node_log.exists(),
        "node should not have been invoked for bun:test authoritative tests"
    );
}

#[tokio::test]
async fn typescript_test_stage_auto_routes_vitest_tsx_through_project_runner() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    let vitest_log = dir.path().join("vitest.log");
    let node_log = dir.path().join("node.log");
    install_fake_tool_at(
        &tool_dir,
        "vitest",
        &format!(
            r#"#!/bin/sh
printf 'runner=vitest\n' > "{}"
for arg in "$@"; do printf 'arg=%s\n' "$arg" >> "{}"; done
cat <<'EOF'
{{"numTotalTestSuites":1,"numPassedTestSuites":1,"numFailedTestSuites":0,"numTotalTests":1,"numPassedTests":1,"numFailedTests":0,"success":true}}
EOF
exit 0
"#,
            vitest_log.display(),
            vitest_log.display(),
        ),
    );
    install_fake_tool_at(
        &tool_dir,
        "node",
        &format!(
            "#!/bin/sh\nprintf 'runner=node\\n' > \"{}\"\nexit 1\n",
            node_log.display(),
        ),
    );

    let src_dir = dir.path().join("src");
    let pages_dir = src_dir.join("pages");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"3.2.4"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("vitest.config.ts"),
        "export default { test: { globals: true } };\n",
    )
    .unwrap();
    let source_path = src_dir.join("analytics.ts");
    let test_path = pages_dir.join("AnalyticsPage.test.tsx");
    let code =
        "export function analyticsLabel(value: string): string { return value.toUpperCase(); }\n";
    let tests = r#"
import { analyticsLabel } from "../analytics.ts";

describe("AnalyticsPage", () => {
  beforeEach(() => vi.restoreAllMocks());
  it("renders its label", () => {
    expect(analyticsLabel("ready")).toBe("READY");
  });
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Auto;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);
    opts.network = NetworkPolicy::Allow;

    let report = verify(code, &Language::TypeScript, opts).await;
    let test_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage should be present");
    assert_eq!(
        test_stage.status,
        StageStatus::Passed,
        "Vitest authoritative test should pass: {:#?}",
        report.stages
    );

    let vitest_log_text = std::fs::read_to_string(&vitest_log)
        .unwrap_or_else(|error| panic!("Auto must launch Vitest ({error}): {:#?}", report.stages));
    assert!(
        vitest_log_text.contains("runner=vitest"),
        "expected Vitest project runner, got:\n{vitest_log_text}"
    );
    assert!(
        vitest_log_text.contains("arg=run") && vitest_log_text.contains("arg=--reporter=json"),
        "expected a complete Vitest JSON reporter invocation, got:\n{vitest_log_text}"
    );
    assert!(
        !vitest_log_text.contains("arg=--reporter=junit"),
        "must not inject an incomplete JUnit reporter, got:\n{vitest_log_text}"
    );
    let canonical_test = std::fs::canonicalize(&test_path).unwrap();
    assert!(
        vitest_log_text
            .lines()
            .any(|line| line == format!("arg={}", canonical_test.display())),
        "expected the original project test path rather than a mirrored copy, got:\n{vitest_log_text}"
    );
    assert_eq!(
        std::fs::read_to_string(&source_path).unwrap(),
        code,
        "authoritative instrumentation must not rewrite the project source"
    );
    assert!(
        !node_log.exists(),
        "Node/plain script must not run a Vitest authoritative test"
    );
}

#[tokio::test]
async fn auto_seed_executes_adjacent_project_test_and_credits_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    let vitest_log = dir.path().join("vitest.log");
    install_fake_tool_at(
        &tool_dir,
        "vitest",
        &format!(
            r#"#!/bin/sh
printf 'runner=vitest\n' > "{}"
for arg in "$@"; do printf 'arg=%s\n' "$arg" >> "{}"; done
printf '%s\n' '{{"event":"target_entered","surface_id":"formatValue:2"}}' >&2
cat <<'EOF'
{{"numTotalTestSuites":1,"numPassedTestSuites":1,"numFailedTestSuites":0,"numTotalTests":1,"numPassedTests":1,"numFailedTests":0,"success":true}}
EOF
exit 0
"#,
            vitest_log.display(),
            vitest_log.display(),
        ),
    );

    std::fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"3.2.4"}}"#,
    )
    .unwrap();
    let source_path = dir.path().join("formatValue.ts");
    let test_path = dir.path().join("formatValue.test.ts");
    let code = "import type { ExternalThing } from \"external-package\";\nexport function formatValue(value: ExternalThing): string { return String(value); }\n";
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(
        &test_path,
        "import { formatValue } from \"./formatValue\";\ntest(\"formats\", () => expect(formatValue({} as never)).toBe(\"[object Object]\"));\n",
    )
    .unwrap();

    let source_file = source_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(None);
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);
    opts.network = NetworkPolicy::Allow;

    let report = verify(code, &Language::TypeScript, opts).await;
    let test_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("auto-seeding should add an authoritative test stage");
    assert_eq!(
        test_stage.status,
        StageStatus::Passed,
        "{:#?}",
        report.stages
    );
    let coverage_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .expect("coverage stage should be present");
    let coverage: Vec<FuzzFunctionCoverage> =
        serde_json::from_value(coverage_stage.detail.as_ref().unwrap()["functions"].clone())
            .unwrap();
    let function = coverage
        .iter()
        .find(|function| function.function == "formatValue")
        .unwrap();
    assert_eq!(
        function.status,
        FuzzFunctionStatus::CheckedViaAuthoritativeTest
    );
    assert!(
        std::fs::read_to_string(&vitest_log)
            .unwrap()
            .contains("formatValue.test.ts"),
        "the discovered adjacent test file must be passed to Vitest"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn vitest_coordinator_resolves_project_entrypoint_and_bounds_legacy_and_modern_workers() {
    for (version, symlinked_launcher, expected_worker_args, forbidden_worker_args) in [
        (
            "0.19.1",
            false,
            &["--threads", "false"][..],
            &["--pool=forks", "--maxWorkers=1", "--minWorkers=1"][..],
        ),
        (
            "3.2.4",
            true,
            &["--pool=forks", "--maxWorkers=1", "--minWorkers=1"][..],
            &["--threads", "false"][..],
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let node_modules = dir.path().join("node_modules");
        let tool_dir = node_modules.join(".bin");
        let vitest_dir = if symlinked_launcher {
            let prefix = dir.path().join("prefix");
            let prefix_bin = prefix.join("bin");
            let package_dir = prefix.join("lib/node_modules/vitest");
            std::fs::create_dir_all(&prefix_bin).unwrap();
            std::fs::create_dir_all(&package_dir).unwrap();
            std::fs::create_dir_all(&node_modules).unwrap();
            std::os::unix::fs::symlink(&prefix_bin, &tool_dir).unwrap();
            std::os::unix::fs::symlink(package_dir.join("vitest.mjs"), prefix_bin.join("vitest"))
                .unwrap();
            package_dir
        } else {
            install_fake_tool_at(
                &tool_dir,
                "vitest",
                "#!/bin/sh\nbasedir=$(dirname \"$0\")\nexit 91\n",
            );
            let package_dir = node_modules.join("vitest");
            std::fs::create_dir_all(&package_dir).unwrap();
            package_dir
        };
        std::fs::write(
            vitest_dir.join("package.json"),
            serde_json::json!({
                "name": "vitest",
                "version": version,
                "type": "module",
                "bin": { "vitest": "./vitest.mjs" }
            })
            .to_string(),
        )
        .unwrap();
        let vitest_log = dir.path().join("vitest.log");
        let entrypoint = r#"
import { appendFileSync } from "node:fs";
import { fork } from "node:child_process";

if (globalThis.__COURT_JESTER_NETWORK_GUARD__) {
  console.error("Vitest coordinator unexpectedly received the target guard");
  process.exit(70);
}
appendFileSync(__LOG__, process.argv.slice(2).map((arg) => `arg=${arg}\n`).join(""));
const testFile = process.argv.at(-1);
const worker = fork(testFile, {
  execArgv: ["--no-warnings", "--experimental-transform-types"],
  stdio: ["ignore", "pipe", "pipe", "ipc"],
});
worker.stdout.pipe(process.stdout);
worker.stderr.pipe(process.stderr);
worker.once("exit", (code, signal) => {
  process.exitCode = signal ? 71 : (code ?? 72);
});
"#
        .replace(
            "__LOG__",
            &serde_json::to_string(&vitest_log.to_string_lossy()).unwrap(),
        );
        std::fs::write(vitest_dir.join("vitest.mjs"), entrypoint).unwrap();
        std::fs::write(
            dir.path().join("vitest.config.ts"),
            "export default { test: { globals: true } };\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            format!(
                r#"{{"name":"guarded-vitest-fixture","type":"module","devDependencies":{{"vitest":"{version}"}}}}"#
            ),
        )
        .unwrap();

        let source_path = dir.path().join("target.ts");
        let test_path = dir.path().join("target.test.ts");
        let code = "export const targetValue = 1;\n";
        let tests = r#"
import { spawnSync } from "node:child_process";
import { connect } from "node:net";

let denied = 0;
for (const operation of [
  () => spawnSync(process.execPath, ["--version"]),
  () => connect(9, "127.0.0.1"),
]) {
  try {
    operation();
  } catch (error) {
    if (/court-jester (process spawn|network access) denied/.test(String(error))) {
      denied += 1;
    }
  }
}
if (denied !== 2) {
  throw new Error(`expected worker process and network denial, observed ${denied}`);
}
console.log(JSON.stringify({
  numTotalTestSuites: 1,
  numPassedTestSuites: 1,
  numFailedTestSuites: 0,
  numTotalTests: 1,
  numPassedTests: 1,
  numFailedTests: 0,
  success: true,
}));
"#;
        std::fs::write(&source_path, code).unwrap();
        std::fs::write(&test_path, tests).unwrap();

        let source_file = source_path.to_string_lossy().into_owned();
        let test_file = test_path.to_string_lossy().into_owned();
        let project_dir = dir.path().to_string_lossy().into_owned();
        let mut opts = default_opts(Some(tests));
        opts.test_source_file = Some(&test_file);
        opts.test_runner = TestRunner::Auto;
        opts.tests_only = true;
        opts.project_dir = Some(&project_dir);
        opts.source_file = Some(&source_file);

        let report = verify(code, &Language::TypeScript, opts).await;
        let test_stage = report
            .stages
            .iter()
            .find(|stage| stage.name == "test")
            .expect("test stage should be present");
        assert_eq!(
            test_stage.status,
            StageStatus::Passed,
            "Vitest {version} must launch its JavaScript entrypoint with a guarded worker: {:#?}",
            report.stages
        );

        let vitest_args = std::fs::read_to_string(&vitest_log)
            .unwrap_or_else(|error| panic!("Vitest {version} entrypoint did not run: {error}"));
        assert!(
            vitest_args.contains("arg=run") && vitest_args.contains("arg=--reporter=json"),
            "Vitest {version} must retain structured JSON results, got:\n{vitest_args}"
        );
        for expected in expected_worker_args {
            assert!(
                vitest_args
                    .lines()
                    .any(|line| line == format!("arg={expected}")),
                "Vitest {version} must bound workers with {expected}, got:\n{vitest_args}"
            );
        }
        for forbidden in forbidden_worker_args {
            assert!(
                !vitest_args
                    .lines()
                    .any(|line| line == format!("arg={forbidden}")),
                "Vitest {version} must not receive incompatible worker flag {forbidden}, got:\n{vitest_args}"
            );
        }
    }
}

#[tokio::test]
async fn bun_authoritative_runner_uses_default_reporter_and_classifies_failures() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    let bun_log = dir.path().join("bun.log");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        &format!(
            r#"#!/bin/sh
reporter_junit=0
printf 'runner=bun\n' > "{}"
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{}"
  if [ "$arg" = "--reporter=junit" ]; then reporter_junit=1; fi
done
if [ "$reporter_junit" -eq 1 ]; then
  echo 'error: --reporter=junit requires --reporter-outfile [file] to specify where to save the XML report' >&2
  exit 1
fi
cat >&2 <<'EOF'
bun test v1.2.0

(fail) add > rejects an incorrect sum

 0 pass
 1 fail
Ran 1 test across 1 file.
EOF
exit 1
"#,
            bun_log.display(),
            bun_log.display(),
        ),
    );

    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();
    let source_path = src_dir.join("math.ts");
    let test_path = tests_dir.join("unit.test.ts");
    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let tests = r#"
import { test, expect } from "bun:test";
import { add } from "../src/math.ts";

test("add", () => {
  expect(add(2, 3)).toBe(6);
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();
    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Bun;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);
    opts.network = NetworkPolicy::Allow;

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Fail,
        "a Bun assertion failure is a target-code failure: {:#?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == FailureDomain::TargetCode
                && diagnostic.kind == FailureKind::AssertionFailure
                && diagnostic.component == DiagnosticComponent::AuthoritativeTestRunner
        }),
        "expected an authoritative assertion diagnostic: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::HarnessProtocol),
        "default Bun failure output must not be diagnosed as a harness protocol error: {:#?}",
        report.diagnostics
    );

    let bun_log_text = std::fs::read_to_string(&bun_log).unwrap();
    assert!(
        bun_log_text.contains("arg=test"),
        "expected Bun's project test runner, got:\n{bun_log_text}"
    );
    assert!(
        !bun_log_text.contains("arg=--reporter=junit"),
        "must not request JUnit without a reporter outfile, got:\n{bun_log_text}"
    );
}

#[tokio::test]
async fn bun_top_level_error_is_an_authoritative_target_failure() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        r#"#!/bin/sh
cat >&2 <<'EOF'
bun test v1.2.0

error: setup exploded
      at <anonymous> (/workspace/tests/unit.test.ts:4:7)

 0 pass
 0 fail
 1 error
Ran 0 tests across 1 file.
EOF
exit 1
"#,
    );

    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();
    let source_path = src_dir.join("math.ts");
    let test_path = tests_dir.join("unit.test.ts");
    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let tests = r#"
import { beforeAll, test, expect } from "bun:test";
import { add } from "../src/math.ts";

beforeAll(() => {
  throw new Error("setup exploded");
});

test("add", () => {
  expect(add(2, 3)).toBe(5);
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();
    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Bun;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);
    opts.network = NetworkPolicy::Allow;

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Fail,
        "a Bun setup error is an authoritative target failure: {:#?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == FailureDomain::TargetCode
                && diagnostic.kind == FailureKind::AssertionFailure
                && diagnostic.component == DiagnosticComponent::AuthoritativeTestRunner
                && diagnostic.impact == DiagnosticImpact::Gating
        }),
        "expected an authoritative target diagnostic: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::HarnessProtocol),
        "positive Bun error summaries must not become protocol blockers: {:#?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn bun_assertion_wrapped_sandbox_blockers_remain_non_target() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        r#"#!/bin/sh
cat >&2 <<'EOF'
AssertionError: Got unwanted exception.
Actual message: "court-jester network access denied"
    at <anonymous> (/workspace/tests/client.test.ts:7:10)
court-jester process spawn denied
bun test v1.2.0

(fail) load > denies network and process access

 0 pass
 1 fail
Ran 1 test across 1 file.
EOF
exit 1
"#,
    );

    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();
    let source_path = src_dir.join("client.ts");
    let test_path = tests_dir.join("client.test.ts");
    let code = "export async function load(): Promise<void> {}\n";
    let tests = r#"
import { expect, test } from "bun:test";

test("denied operations", () => {
  expect(() => fetch("https://example.com")).not.toThrow();
  expect(() => Bun.spawnSync(["echo", "blocked"])).not.toThrow();
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();
    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Bun;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "a sandbox blocker is not a target-code failure: {:#?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                FailureKind::NetworkDenied | FailureKind::ProcessSpawnDenied
            )
        }),
        "expected typed sandbox blockers: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.domain != FailureDomain::TargetCode
                && diagnostic.kind != FailureKind::AssertionFailure),
        "sandbox-caused Bun output must not become a target assertion: {:#?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn vitest_spawn_policy_denial_makes_authoritative_stage_inconclusive() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "vitest",
        r#"#!/bin/sh
cat <<'EOF'
{"numTotalTestSuites":1,"numFailedTestSuites":1,"numTotalTests":1,"numFailedTests":1,"success":false,"testResults":[{"assertionResults":[{"status":"failed","failureMessages":["pdf-inspector process could not be started: court-jester process spawn denied"]}],"status":"failed","message":"pdf-inspector process could not be started: court-jester process spawn denied"}]}
EOF
exit 1
"#,
    );
    let vitest_dir = dir.path().join("node_modules").join("vitest");
    std::fs::create_dir_all(&vitest_dir).unwrap();
    std::fs::write(
        vitest_dir.join("package.json"),
        r#"{"name":"vitest","version":"3.2.4","type":"module","bin":{"vitest":"./vitest.mjs"}}"#,
    )
    .unwrap();
    std::fs::write(
        vitest_dir.join("vitest.mjs"),
        r#"console.log(JSON.stringify({"numTotalTestSuites":1,"numFailedTestSuites":1,"numTotalTests":1,"numFailedTests":1,"success":false,"testResults":[{"assertionResults":[{"status":"failed","failureMessages":["pdf-inspector process could not be started: court-jester process spawn denied"]}],"status":"failed","message":"pdf-inspector process could not be started: court-jester process spawn denied"}]}));
process.exitCode = 1;
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"3.2.4"}}"#,
    )
    .unwrap();

    let source_path = dir.path().join("inspect.ts");
    let test_path = dir.path().join("inspect.test.ts");
    let code = "export function inspect(): string { return \"ready\"; }\n";
    let tests = r#"
import { expect, test } from "vitest";
import { inspect } from "./inspect";

test("inspects a document", () => {
  expect(inspect()).toBe("ready");
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();
    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Auto;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);

    let report = verify(code, &Language::TypeScript, opts).await;
    let test_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage should be present");

    assert_eq!(
        test_stage.status,
        StageStatus::Inconclusive,
        "a harness spawn-policy denial cannot fail target code: {:#?}",
        report.stages
    );
    let detail = test_stage.detail.as_ref().unwrap();
    assert_eq!(detail["non_target_blocking"].as_bool(), Some(true));
    assert_eq!(detail["assertion_failure"].as_bool(), Some(false));
    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == FailureDomain::Environment
                && diagnostic.kind == FailureKind::ProcessSpawnDenied
                && diagnostic.component == DiagnosticComponent::Sandbox
                && diagnostic.impact == DiagnosticImpact::Blocking
        }),
        "expected a typed spawn-policy blocker: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::AssertionFailure),
        "the spawn-policy denial must not also become a target assertion: {:#?}",
        report.diagnostics
    );
}

#[tokio::test]
async fn bun_native_dependency_failure_preserves_authoritative_reachability() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        r#"#!/bin/sh
cat >&2 <<'EOF'
bun test v1.3.14
{"event":"target_entered","surface_id":"add:1"}
PrismaClientInitializationError: Prisma Client could not locate the Query Engine for runtime "linux-arm64-openssl-3.0.x".
This happened because Prisma Client was generated for "darwin-arm64", but the actual deployment required "linux-arm64-openssl-3.0.x".

(fail) add > uses the database fixture

 0 pass
 1 fail
Ran 1 test across 1 file.
EOF
exit 1
"#,
    );

    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();
    let source_path = src_dir.join("math.ts");
    let test_path = tests_dir.join("unit.test.ts");
    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let tests = r#"
import { test, expect } from "bun:test";
import { add } from "../src/math.ts";

test("add", () => {
  expect(add(2, 3)).toBe(5);
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();
    let source_file = source_path.to_string_lossy().into_owned();
    let test_file = test_path.to_string_lossy().into_owned();
    let project_dir = dir.path().to_string_lossy().into_owned();
    let mut opts = default_opts(Some(tests));
    opts.test_source_file = Some(&test_file);
    opts.test_runner = TestRunner::Bun;
    opts.tests_only = true;
    opts.project_dir = Some(&project_dir);
    opts.source_file = Some(&source_file);
    opts.network = NetworkPolicy::Allow;

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "a platform-incompatible project dependency is not target code: {report:#?}"
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == FailureDomain::Environment
                && diagnostic.kind == FailureKind::ModuleLoad
        }),
        "expected an environment module-load diagnostic: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.domain != FailureDomain::TargetCode),
        "native dependency mismatch must not become a target assertion: {:#?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::ContractViolation),
        "exact authoritative reachability satisfies the coverage contract: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.summary.coverage.behaviorally_checked, 0);
    assert_eq!(report.summary.coverage.reached_only, 1);
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .expect("per-surface coverage");
    assert_eq!(
        coverage[0]["status"].as_str(),
        Some("reached_via_authoritative_test")
    );
}

#[tokio::test]
async fn python_test_stage_executes_original_test_file_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("app.py");
    let test_path = tests_dir.join("test_app.py");
    let code = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    let tests = r#"
from pathlib import Path

from src.app import add

assert add(2, 3) == 5
assert Path(__file__).name == "test_app.py"
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "a passing uninstrumented Python script must not imply exact surface coverage: {:#?}",
        report.stages
    );
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .expect("tests-only coverage stage");
    assert_eq!(coverage.status, StageStatus::Inconclusive);
}

#[tokio::test]
async fn python_relative_import_test_stage_executes_original_module_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let pkg_dir = dir.path().join("mypkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("__init__.py"), "").unwrap();

    let source_path = pkg_dir.join("app.py");
    let test_path = pkg_dir.join("test_app.py");
    let code = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    let tests = r#"
from pathlib import Path

from .app import add

assert add(2, 3) == 5
assert Path(__file__).name == "test_app.py"
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_normalize_helper_can_return_blank_when_api_handles_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let normalizers_path = src_dir.join("normalizers.ts");
    let plans_path = src_dir.join("plans.ts");
    let test_path = tests_dir.join("court_jester_public_verify.ts");
    let normalizers = r#"
export function normalizePlanCode(value: string | null | undefined): string {
  if (typeof value !== "string") {
    return "";
  }
  return value.trim().toUpperCase();
}
"#;
    let plans = r#"
import { normalizePlanCode } from "./normalizers.ts";

export type Account = {
  plans?: Array<string | null> | null;
} | null;

export function primaryPlanCode(account: Account): string {
  const plans = account?.plans;
  if (plans) {
    for (const p of plans) {
      const code = normalizePlanCode(p);
      if (code) return code;
    }
  }
  return "FREE";
}
"#;
    let tests = r#"
import assert from "node:assert/strict";
import { primaryPlanCode } from "../src/plans.ts";

assert.equal(primaryPlanCode({ plans: ["   ", "pro"] }), "PRO");
assert.equal(primaryPlanCode({ plans: [null, ""] }), "FREE");
"#;
    std::fs::write(&normalizers_path, normalizers).unwrap();
    std::fs::write(&plans_path, plans).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(normalizers_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(normalizers, &Language::TypeScript, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "execute" && s.status == StageStatus::Passed));
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}

#[tokio::test]
async fn typescript_test_file_without_imports_uses_source_file_scope() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("initials.ts");
    let tests_path = dir
        .path()
        .join("tests")
        .join("court_jester_public_verify.ts");
    std::fs::create_dir_all(tests_path.parent().unwrap()).unwrap();

    let code = r#"
export function displayInitials(name: string | null): string {
  const parts = name?.trim().split(/\s+/).filter(Boolean) ?? [];
  const initials = parts.map((part) => part[0]?.toUpperCase() ?? "").join("");
  return initials || "NA";
}
"#;
    let tests = r#"
if (displayInitials("Spencer Lee") !== "SL") {
  throw new Error("expected SL");
}
"#;
    std::fs::write(&src_path, code).unwrap();
    std::fs::write(&tests_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(tests_path.to_str().unwrap()),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(src_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(
        report.verdict == VerificationVerdict::Pass,
        "report: {:#?}",
        report.stages
    );
    assert!(report
        .stages
        .iter()
        .any(|s| s.name == "test" && s.status == StageStatus::Passed));
}
#[tokio::test]
async fn schema_v3_safe_function_passes_with_property_strength() {
    let report = verify(
        "def add(a: int, b: int) -> int:\n    return a + b",
        &Language::Python,
        default_opts(None),
    )
    .await;
    assert_eq!(report.schema_version, 3);
    assert_eq!(report.verdict, VerificationVerdict::Pass);
    assert_eq!(
        report.strength,
        court_jester::types::VerificationStrength::PropertyChecked
    );
    assert!(report.summary.coverage.behaviorally_checked > 0);
    assert!(report.summary.coverage.required > 0);
}

#[tokio::test]
async fn schema_v3_syntax_error_is_fail_with_parse_only_strength() {
    let report = verify("def broken(:", &Language::Python, default_opts(None)).await;
    assert_eq!(report.schema_version, 3);
    assert_eq!(report.verdict, VerificationVerdict::Fail);
    assert_eq!(
        report.strength,
        court_jester::types::VerificationStrength::ParseOnly
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("parse failure should still report skipped execution");
    assert_eq!(execute.status, StageStatus::Skipped);
    assert_eq!(
        execute.detail.as_ref().unwrap()["reason"].as_str(),
        Some("parse_failed")
    );
}

#[tokio::test]
async fn context_failure_still_reports_skipped_execute_stage() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let source = outside.path().join("target.py");
    let code = "def add(value: int) -> int:\n    return value + 1";
    fs::write(&source, code).unwrap();
    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();

    let report = verify(code, &Language::Python, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    assert!(report
        .stages
        .iter()
        .any(|stage| stage.name == "context" && stage.status == StageStatus::Inconclusive));
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("context failure should still report skipped execution");
    assert_eq!(execute.status, StageStatus::Skipped);
    assert_eq!(
        execute.detail.as_ref().unwrap()["reason"].as_str(),
        Some("context_unavailable")
    );
}

#[tokio::test]
async fn unsupported_required_export_is_inconclusive_not_pass() {
    let report = verify(
        "def parse(value: UnresolvedType) -> str:\n    return value.name",
        &Language::Python,
        default_opts(None),
    )
    .await;
    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    assert_eq!(
        report.strength,
        court_jester::types::VerificationStrength::StaticChecked
    );
    assert!(report.summary.coverage.required >= 1);
    assert!(report.summary.coverage.skipped >= 1 || report.summary.coverage.no_inputs_reached >= 1);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("unsupported required export should report skipped execution");
    assert_eq!(execute.status, StageStatus::Skipped);
    let detail = execute.detail.as_ref().unwrap();
    assert_eq!(detail["reason"].as_str(), Some("no_fuzzable_targets"));
    assert_eq!(detail["generated_cases"].as_u64(), Some(0));
}

#[tokio::test]
async fn coverage_none_does_not_manufacture_pass_without_behavioral_evidence() {
    let mut opts = default_opts(None);
    opts.coverage_gate = CoverageGate::None;
    let report = verify("CONSTANT = 1", &Language::Python, opts).await;
    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("verification without functions should report skipped execution");
    assert_eq!(execute.status, StageStatus::Skipped);
    let detail = execute.detail.as_ref().unwrap();
    assert_eq!(detail["reason"].as_str(), Some("no_analyzed_functions"));
    assert_eq!(detail["generated_cases"].as_u64(), Some(0));
}

#[tokio::test]
async fn tests_only_partial_same_process_reach_is_inconclusive_with_exact_checked_surface() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("surfaces.py");
    let test_source = project.path().join("test_surfaces.py");
    let code = "def covered(x: int) -> int:\n    return x\n\ndef uncovered(x: int) -> int:\n    return x + 1";
    let tests = "from surfaces import covered, uncovered\n\nif False:\n    uncovered(1)\nassert covered(1) == 1";
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();
    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();

    let report = verify(code, &Language::Python, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 2);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);

    let test_detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .expect("tests-only run should retain authoritative test evidence");
    let entered = test_detail["target_entered_surfaces"]
        .as_array()
        .expect("same-process target entry events");
    assert!(entered.iter().any(|surface| surface == "covered:1"));
    assert!(!entered.iter().any(|surface| surface == "uncovered:4"));

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("tests-only coverage stage");
    assert_eq!(
        coverage["observed_required_surface_count"].as_u64(),
        Some(1)
    );
    let functions = coverage["functions"]
        .as_array()
        .expect("per-surface coverage");
    let checked = functions
        .iter()
        .filter(|function| function["status"] == "checked_via_authoritative_test")
        .map(|function| function["function"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(checked, ["covered"]);
    let uncovered = functions
        .iter()
        .find(|function| function["function"] == "uncovered")
        .expect("unexecuted required surface");
    assert_eq!(
        uncovered["status"].as_str(),
        Some("skipped_no_fuzzable_surface")
    );
    assert_eq!(
        uncovered["reason"].as_str(),
        Some("authoritative test did not emit the exact target_entered surface id")
    );
}

#[tokio::test]
async fn tests_only_full_same_process_reach_passes() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("surfaces.py");
    let test_source = project.path().join("test_surfaces.py");
    let code = "def covered(x: int) -> int:\n    return x\n\ndef also_covered(x: int) -> int:\n    return x + 1";
    let tests = "from surfaces import also_covered, covered\n\nassert covered(1) == 1\nassert also_covered(1) == 2";
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();
    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();

    let report = verify(code, &Language::Python, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 2);
    assert_eq!(report.summary.coverage.behaviorally_checked, 2);

    let test_detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .expect("tests-only run should retain authoritative test evidence");
    let entered = test_detail["target_entered_surfaces"]
        .as_array()
        .expect("same-process target entry events");
    assert!(entered.iter().any(|surface| surface == "covered:1"));
    assert!(entered.iter().any(|surface| surface == "also_covered:4"));

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("tests-only coverage stage");
    assert_eq!(
        coverage["observed_required_surface_count"].as_u64(),
        Some(2)
    );
    let checked = coverage["functions"]
        .as_array()
        .expect("per-surface coverage")
        .iter()
        .filter(|function| function["status"] == "checked_via_authoritative_test")
        .map(|function| function["function"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(checked, ["covered", "also_covered"]);
}

#[tokio::test]
async fn persisted_and_minimal_reports_are_v3_without_legacy_ok_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = default_opts(None);
    opts.output_dir = Some(dir.path().to_str().unwrap());
    let report = verify(
        "def add(x: int) -> int:\n    return x",
        &Language::Python,
        opts,
    )
    .await;
    let full = report_json_value(&report, ReportLevel::Full);
    let minimal = report_json_value(&report, ReportLevel::Minimal);
    for value in [full, minimal] {
        assert_eq!(value["schema_version"].as_u64(), Some(3));
        assert_eq!(
            value["tool"]["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            value["candidate"]["content_sha256"].as_str().map(str::len),
            Some(64)
        );
        assert!(value.get("verdict").is_some());
        assert!(value.get("strength").is_some());
        assert!(value.get("overall_ok").is_none());
        assert!(value.get("ok").is_none());
    }
    let path = report.report_path.expect("output report path");
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(persisted.get("overall_ok").is_none());
    assert_eq!(
        persisted["tool"]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        persisted["candidate"]["content_sha256"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    assert!(persisted["summary"]["coverage"].is_object());
}
#[tokio::test]
async fn equivalent_findings_are_coalesced_and_minimal_reports_are_bounded() {
    let project = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = project.path().join("repeated.py");
    let code = "def explode(value: str) -> str:\n    print('x' * 10000)\n    return value[1000]\n";
    fs::write(&source, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.output_dir = output.path().to_str();
    opts.report_level = ReportLevel::Minimal;
    let report = verify(code, &Language::Python, opts).await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail");
    let findings = execute["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert!(
        findings[0]["occurrences"].as_u64().unwrap_or(0) > 1,
        "{findings:#?}"
    );
    assert!(
        findings[0]["sample_inputs"]
            .as_array()
            .is_some_and(|samples| samples.len() <= 3),
        "{findings:#?}"
    );

    let report_path = report.report_path.as_deref().expect("persisted report");
    let bytes = fs::read(report_path).unwrap();
    assert!(bytes.len() < 512 * 1024, "{} bytes", bytes.len());
    let persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let minimal_execute = persisted["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["name"] == "execute")
        .expect("minimal execute stage");
    assert!(minimal_execute["detail"].get("execution").is_none());

    let full = report_json_value(&report, ReportLevel::Full);
    let retained_stdout = full["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["name"] == "execute")
        .and_then(|stage| stage["detail"]["execution"]["stdout"].as_str())
        .expect("full execution stdout");
    assert!(retained_stdout.chars().count() <= 64 * 1024 + 128);
}

#[tokio::test]
async fn nonbreaking_space_failure_has_structured_minimized_replay() {
    let code = "def normalize_display_name(value: str) -> str:\n    if value.isspace() and '\\xa0' in value:\n        return value.strip()[0]\n    return value";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    let findings = execute
        .detail
        .as_ref()
        .and_then(|detail| detail["findings"].as_array())
        .expect("typed findings");
    let finding = findings
        .iter()
        .find(|finding| finding["location"]["function"] == "normalize_display_name")
        .unwrap_or_else(|| panic!("NBSP finding: {report:#?}"));
    assert_eq!(
        finding["minimization"]["status"].as_str(),
        Some("preserved")
    );
    assert_eq!(
        finding["minimization"]["minimized"]["arguments"][0]["expression"].as_str(),
        Some("'\\xa0'")
    );
    assert!(finding["repro"]["snippet"]
        .as_str()
        .unwrap()
        .contains("__COURT_JESTER_REPLAY_JSON__"));
    assert_eq!(
        finding["repro"]["expectation"]["category"].as_str(),
        Some("exception")
    );
}

#[tokio::test]
async fn inferred_oracle_findings_are_advisory_but_directives_are_authoritative() {
    let inferred = verify(
        "def normalize_name(value: str) -> str:\n    return value.upper()",
        &Language::Python,
        default_opts(None),
    )
    .await;
    let inferred_findings = inferred
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array());
    if let Some(findings) = inferred_findings {
        for finding in findings {
            if finding["oracle"]["kind"] == "inferred_semantic" {
                assert_eq!(finding["confidence"].as_str(), Some("low"));
                assert_eq!(finding["suppressed"].as_bool(), Some(false));
            }
        }
    }
    let directive = verify(
        "# @court-jester-properties sorted\ndef reverse(values: list[int]) -> list[int]:\n    return list(reversed(values))",
        &Language::Python,
        default_opts(None),
    )
    .await;
    let findings = directive
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .expect("directive finding");
    let sorted = findings
        .iter()
        .find(|finding| finding["oracle"]["kind"] == "declared_property")
        .expect("declared property oracle");
    assert_eq!(
        sorted["oracle"]["provenance"].as_str(),
        Some("source_directive")
    );
    assert_eq!(sorted["confidence"].as_str(), Some("authoritative"));
}
#[tokio::test]
async fn base_candidate_divergence_is_advisory_without_authoritative_oracle() {
    let candidate = "def identity(value: int) -> int:\n    return value + 1";
    let base = "def identity(value: int) -> int:\n    return value";
    let report = verify_differential_files(candidate, base, Language::Python).await;
    let differential = report
        .stages
        .iter()
        .find(|stage| stage.name == "differential");
    assert!(
        differential.is_some(),
        "differential stage should be explicit"
    );
    let detail = differential
        .and_then(|stage| stage.detail.as_ref())
        .expect("differential detail");
    assert!(detail["findings"]
        .as_array()
        .map(|findings| !findings.is_empty())
        .unwrap_or(false));
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "unproven divergence is advisory"
    );
}

#[tokio::test]
async fn minimal_output_dir_report_loads_and_replays() {
    let project = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = project.path().join("characters.py");
    let code = "def first_character(value: str) -> str:\n    return value[0]";
    fs::write(&source, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.output_dir = output.path().to_str();
    opts.report_level = ReportLevel::Minimal;
    let report = verify(code, &Language::Python, opts).await;
    let report_path = report
        .report_path
        .expect("minimal run should persist a report");

    let persisted =
        load_persisted_report(&report_path).expect("minimal persisted report should load");
    let finding = persisted
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings
                .iter()
                .find(|finding| finding["location"]["function"] == "first_character")
        })
        .expect("persisted minimal report should retain the actionable finding");
    let finding_id = finding["id"].as_str().expect("finding id");
    assert!(finding["repro"]["command"].as_str().is_some());

    let replay = replay_report(
        &report_path,
        finding_id,
        None,
        RuntimeProfile::LocalTrusted,
        DEFAULT_PYTHON_DOCKER_IMAGE,
        DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
    )
    .await
    .expect("persisted minimal finding should be replayable");
    assert_eq!(replay.outcome, ReplayOutcome::Reproduced, "{replay:#?}");
}

#[tokio::test]
async fn repair_json_report_loads_and_replays() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("characters.py");
    let code = "def first_character(value: str) -> str:\n    return value[0]";
    fs::write(&source, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    let report = verify(code, &Language::Python, opts).await;
    let repair = repair_summary(&report, &Language::Python);
    let finding_id = repair
        .findings
        .iter()
        .find(|finding| finding.location.function == "first_character")
        .expect("repair summary should retain the actionable finding")
        .id
        .clone();
    let report_path = project.path().join("repair.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&repair).unwrap()).unwrap();

    let loaded =
        load_persisted_report(report_path.to_str().unwrap()).expect("repair report should load");
    assert_eq!(loaded.meta.language, "python");
    let replay = replay_report(
        report_path.to_str().unwrap(),
        &finding_id,
        None,
        RuntimeProfile::LocalTrusted,
        DEFAULT_PYTHON_DOCKER_IMAGE,
        DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
    )
    .await
    .expect("repair finding should be replayable");
    assert_eq!(replay.outcome, ReplayOutcome::Reproduced, "{replay:#?}");
}

#[tokio::test]
async fn persisted_python_replay_ignores_guarded_main() {
    let project = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = project.path().join("guarded_cli.py");
    let code = r#"import argparse

def first_character(value: str) -> str:
    return value[0]

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--required", required=True)
    parser.parse_args()
"#;
    fs::write(&source, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.output_dir = output.path().to_str();
    opts.report_level = ReportLevel::Minimal;
    let report = verify(code, &Language::Python, opts).await;
    let report_path = report
        .report_path
        .expect("guarded source should persist a report");
    let persisted = load_persisted_report(&report_path).expect("guarded source report should load");
    let finding = persisted
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings
                .iter()
                .find(|finding| finding["location"]["function"] == "first_character")
        })
        .expect("guarded source should retain the actionable finding");
    let finding_id = finding["id"].as_str().expect("finding id");

    let replay = replay_report(
        &report_path,
        finding_id,
        None,
        RuntimeProfile::LocalTrusted,
        DEFAULT_PYTHON_DOCKER_IMAGE,
        DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
    )
    .await
    .expect("guarded source finding should be replayable");

    assert_eq!(replay.outcome, ReplayOutcome::Reproduced, "{replay:#?}");
    assert_eq!(replay.execution.exit_code, Some(0), "{replay:#?}");
}

#[tokio::test]
async fn differential_full_and_minimal_reports_replay_after_source_projects_are_removed() {
    let projects = tempfile::tempdir().unwrap();
    let candidate_project = projects.path().join("candidate");
    let baseline_project = projects.path().join("baseline");
    fs::create_dir_all(&candidate_project).unwrap();
    fs::create_dir_all(&baseline_project).unwrap();
    let candidate_source = candidate_project.join("entry.py");
    let baseline_source = baseline_project.join("entry.py");
    let entry_code =
        "from helper import OFFSET\n\ndef adjusted(value: int) -> int:\n    return value + OFFSET";
    fs::write(&candidate_source, entry_code).unwrap();
    fs::write(candidate_project.join("helper.py"), "OFFSET = 2\n").unwrap();
    fs::write(&baseline_source, entry_code).unwrap();
    fs::write(baseline_project.join("helper.py"), "OFFSET = 1\n").unwrap();

    let output = tempfile::tempdir().unwrap();
    let mut persisted_cases = Vec::new();
    for (label, report_level) in [
        ("full", ReportLevel::Full),
        ("minimal", ReportLevel::Minimal),
    ] {
        let case_output = output.path().join(label);
        let mut opts = default_opts(None);
        opts.project_dir = candidate_project.to_str();
        opts.source_file = candidate_source.to_str();
        opts.base_code = Some(entry_code);
        opts.base_source_file = baseline_source.to_str();
        opts.base_project_dir = baseline_project.to_str();
        opts.output_dir = case_output.to_str();
        opts.report_level = report_level;

        let report = verify(entry_code, &Language::Python, opts).await;
        let report_path = report
            .report_path
            .expect("differential run should persist a report");
        let persisted =
            load_persisted_report(&report_path).expect("differential report should load");
        let finding_id = persisted
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .and_then(|stage| stage.detail.as_ref())
            .and_then(|detail| detail["findings"].as_array())
            .and_then(|findings| {
                findings
                    .iter()
                    .find(|finding| finding["category"] == "differential")
            })
            .and_then(|finding| finding["id"].as_str())
            .expect("persisted report should retain the differential finding")
            .to_string();
        persisted_cases.push((report_path, finding_id));
    }

    fs::remove_dir_all(&candidate_project).unwrap();
    fs::remove_dir_all(&baseline_project).unwrap();
    assert!(!candidate_project.exists());
    assert!(!baseline_project.exists());

    for (report_path, finding_id) in persisted_cases {
        let replay = replay_report(
            &report_path,
            &finding_id,
            None,
            RuntimeProfile::LocalTrusted,
            DEFAULT_PYTHON_DOCKER_IMAGE,
            DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        )
        .await
        .expect("embedded differential report should replay without its original projects");
        assert_eq!(replay.outcome, ReplayOutcome::Reproduced, "{replay:#?}");
    }
}

#[tokio::test]
async fn incompatible_and_missing_baseline_surfaces_are_disabled_without_differential_findings() {
    let candidate = "def missing(value: int) -> int:\n    return value\n\ndef changed(value: int, extra: int) -> int:\n    return value + extra";
    let baseline = "def changed(value: str) -> int:\n    return len(value)";
    let mut opts = default_opts(None);
    opts.base_code = Some(baseline);

    let report = verify(candidate, &Language::Python, opts).await;

    let differential = report
        .stages
        .iter()
        .find(|stage| stage.name == "differential")
        .and_then(|stage| stage.detail.as_ref())
        .expect("differential diagnostics");
    assert!(differential["findings"]
        .as_array()
        .is_some_and(Vec::is_empty));
    let units = differential["comparison"]["units"]
        .as_array()
        .expect("per-surface differential diagnostics");
    assert_eq!(units.len(), 2);
    let missing = units
        .iter()
        .find(|unit| unit["surface"] == "missing:1")
        .expect("missing baseline diagnostic");
    assert_eq!(missing["status"].as_str(), Some("disabled"));
    assert_eq!(missing["reason"].as_str(), Some("missing_base_surface"));
    let incompatible = units
        .iter()
        .find(|unit| unit["surface"] == "changed:4")
        .expect("incompatible baseline diagnostic");
    assert_eq!(incompatible["status"].as_str(), Some("disabled"));
    assert_eq!(
        incompatible["reason"].as_str(),
        Some("incompatible_signature")
    );

    let fabricated = report.stages.iter().any(|stage| {
        stage
            .detail
            .as_ref()
            .and_then(|detail| detail["findings"].as_array())
            .is_some_and(|findings| {
                findings
                    .iter()
                    .any(|finding| finding["category"] == "differential")
            })
    });
    assert!(
        !fabricated,
        "disabled comparisons must not fabricate divergence findings"
    );
}

#[tokio::test]
async fn tests_only_typescript_unreached_invocable_method_remains_required_and_inconclusive() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("surfaces.ts");
    let test_source = project.path().join("surfaces.test.ts");
    let code = "export function increment(value: number): number {\n  return value + 1;\n}\n\nexport class Counter {\n  increment(value: number): number {\n    return value + 1;\n  }\n}\n";
    let tests = r#"import assert from "node:assert/strict";
import { Counter, increment } from "./surfaces.ts";

if (false) {
  assert.equal(new Counter().increment(1), 2);
}
assert.equal(increment(1), 2);
"#;
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();

    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "{report:#?}"
    );
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 2);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);

    let test_detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .expect("passing authoritative test evidence");
    let entered = test_detail["target_entered_surfaces"]
        .as_array()
        .expect("same-process target entry events");
    assert!(entered.iter().any(|surface| surface == "increment:1"));
    assert!(!entered
        .iter()
        .any(|surface| surface == "Counter#increment:6"));

    let functions = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .expect("per-surface coverage");
    let top_level = functions
        .iter()
        .find(|function| function["function"] == "increment")
        .expect("top-level function coverage");
    assert_eq!(
        top_level["status"].as_str(),
        Some("checked_via_authoritative_test")
    );
    let method = functions
        .iter()
        .find(|function| function["function"] == "Counter#increment")
        .expect("exported invocable method must remain in required coverage");
    assert_eq!(method["required"].as_bool(), Some(true));
    assert_ne!(
        method["status"].as_str(),
        Some("checked_via_authoritative_test")
    );
    assert_eq!(
        method["reason"].as_str(),
        Some("authoritative test did not emit the exact target_entered surface id")
    );
}

#[tokio::test]
async fn tests_only_block_bodied_typescript_arrow_emits_exact_surface_and_passes() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("increment.ts");
    let test_source = project.path().join("increment.test.ts");
    let code = "export const increment = (value: number): number => {\n  return value + 1;\n};\n";
    let tests = r#"import assert from "node:assert/strict";
import { increment } from "./increment.ts";

assert.equal(increment(1), 2);
"#;
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();

    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 1);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);
    let test_detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .expect("passing authoritative test evidence");
    assert_eq!(
        test_detail["target_entered_surfaces"].as_array(),
        Some(&vec![serde_json::json!("increment:1")])
    );
    let arrow = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"] == "increment")
        })
        .expect("arrow coverage");
    assert_eq!(
        arrow["status"].as_str(),
        Some("checked_via_authoritative_test")
    );
}

#[tokio::test]
async fn tests_only_bun_test_emits_exact_target_surface_and_passes() {
    if !std::process::Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("increment.ts");
    let test_source = project.path().join("increment.test.ts");
    let code = "export function increment(value: number): number {\n  return value + 1;\n}\n";
    let tests = r#"import { expect, test } from "bun:test";
import { increment } from "./increment";

test("increments", () => {
  expect(increment(1)).toBe(2);
});
"#;
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();

    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();
    opts.test_runner = TestRunner::Auto;

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 1);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);
    let entered = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["target_entered_surfaces"].as_array())
        .expect("Bun target entry events");
    assert_eq!(entered, &vec![serde_json::json!("increment:1")]);
}

#[tokio::test]
async fn tests_only_bun_test_is_instrumented_in_isolated_runtime() {
    let docker_available = std::process::Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|output| output.status.success());
    let image_available = std::process::Command::new("docker")
        .args(["image", "inspect", DEFAULT_BUN_DOCKER_IMAGE])
        .output()
        .is_ok_and(|output| output.status.success());
    if !docker_available || !image_available {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("increment.ts");
    let test_source = project.path().join("increment.test.ts");
    let code = "export function increment(value: number): number {\n  return value + 1;\n}\n";
    let tests = r#"import { expect, test } from "bun:test";
import { increment } from "./increment";

test("increments", () => {
  expect(increment(1)).toBe(2);
});
"#;
    fs::write(&source, code).unwrap();
    fs::write(&test_source, tests).unwrap();

    let mut opts = default_opts(Some(tests));
    opts.tests_only = true;
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_source.to_str();
    opts.test_runner = TestRunner::Auto;
    opts.runtime_profile = RuntimeProfile::Isolated;

    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.strength, VerificationStrength::AuthoritativeTests);
    assert_eq!(report.summary.coverage.required, 1);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);
    let entered = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["target_entered_surfaces"].as_array())
        .expect("isolated Bun target entry events");
    assert_eq!(entered, &vec![serde_json::json!("increment:1")]);
}

#[tokio::test]
async fn exported_factory_shorthand_data_field_is_not_a_required_callable_surface() {
    let code = r#"export function buildRecord(input: string): { normalized: string } {
  const normalized = input.trim();
  return { normalized };
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.summary.coverage.required, 1);
    assert_eq!(report.summary.coverage.behaviorally_checked, 1);
    let functions = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .expect("per-surface coverage");
    let factory = functions
        .iter()
        .find(|function| function["function"] == "buildRecord")
        .expect("factory coverage");
    assert_eq!(factory["status"].as_str(), Some("checked_direct"));
    assert!(
        !functions.iter().any(|function| {
            function["function"]
                .as_str()
                .is_some_and(|name| name.starts_with("buildRecord()."))
        }),
        "ordinary returned data must not be reported as callable coverage: {functions:#?}"
    );
}

#[tokio::test]
async fn python_differential_keyword_only_parameter_uses_named_binding() {
    let candidate = "def adjusted(*, value: int) -> int:\n    return value + 1\n";
    let baseline = "def adjusted(*, value: int) -> int:\n    return value\n";
    let report = verify_differential_files(candidate, baseline, Language::Python).await;

    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "differential")
        .and_then(|stage| stage.detail.as_ref())
        .expect("differential diagnostics");
    let unit = detail["comparison"]["units"]
        .as_array()
        .and_then(|units| units.iter().find(|unit| unit["surface"] == "adjusted:1"))
        .expect("keyword-only differential unit");
    assert_eq!(unit["status"].as_str(), Some("different"));
    let finding = detail["findings"]
        .as_array()
        .and_then(|findings| {
            findings.iter().find(|finding| {
                finding["location"]["function"] == "adjusted"
                    && finding["category"] == "differential"
            })
        })
        .expect("valid keyword-only invocation should expose the behavior change");
    assert_eq!(finding["oracle"]["kind"].as_str(), Some("differential"));
    let baseline_snapshot: serde_json::Value = serde_json::from_str(
        finding["oracle"]["expected"]
            .as_str()
            .expect("baseline snapshot"),
    )
    .unwrap();
    let candidate_snapshot: serde_json::Value = serde_json::from_str(
        finding["oracle"]["actual"]
            .as_str()
            .expect("candidate snapshot"),
    )
    .unwrap();
    assert_eq!(baseline_snapshot["returned"].as_i64(), Some(0));
    assert!(baseline_snapshot["exception_type"].is_null());
    assert_eq!(candidate_snapshot["returned"].as_i64(), Some(1));
    assert!(candidate_snapshot["exception_type"].is_null());
}

#[tokio::test]
async fn identical_address_bearing_differential_returns_are_disabled_not_regressions() {
    let code = "VALUE = object()\n\ndef fetch(value: int) -> object:\n    return VALUE\n";
    let report = verify_differential_files(code, code, Language::Python).await;

    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "differential")
        .and_then(|stage| stage.detail.as_ref())
        .expect("differential diagnostics");
    assert!(
        detail["findings"].as_array().is_some_and(Vec::is_empty),
        "process-specific object addresses must not become regressions: {detail:#?}"
    );
    let unit = detail["comparison"]["units"]
        .as_array()
        .and_then(|units| units.iter().find(|unit| unit["surface"] == "fetch:3"))
        .expect("address-bearing differential unit");
    assert_eq!(unit["status"].as_str(), Some("disabled"));
    assert_eq!(
        unit["reason"].as_str(),
        Some("unsupported_snapshot:candidate=return_unsupported_type_object;baseline=return_unsupported_type_object")
    );
    assert!(!report.stages.iter().any(|stage| {
        stage
            .detail
            .as_ref()
            .and_then(|detail| detail["findings"].as_array())
            .is_some_and(|findings| {
                findings
                    .iter()
                    .any(|finding| finding["category"] == "differential")
            })
    }));
}

#[tokio::test]
async fn nuxt_auto_import_reference_error_is_one_environment_blocker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"nuxt":"3.15.4"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("nuxt.config.ts"), "export default {};\n").unwrap();
    let source_path = dir.path().join("useCounter.ts");
    let code = r#"
export function useCounter() {
  return ref(0);
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Inconclusive);
    let detail = execute.detail.as_ref().expect("execute detail");
    assert!(
        detail["findings"].as_array().is_some_and(Vec::is_empty),
        "a missing Nuxt runtime must not be attributed to target code: {detail:#?}"
    );
    assert_eq!(
        detail["environment_setup"]["classification"].as_str(),
        Some("missing_nuxt_auto_import_runtime")
    );
    assert_eq!(
        detail["environment_setup"]["missing_globals"]
            .as_array()
            .expect("missing globals"),
        &[serde_json::Value::String("ref".into())]
    );
    let environment_diagnostics = detail["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter(|diagnostic| diagnostic["domain"] == "environment")
        .collect::<Vec<_>>();
    assert_eq!(
        environment_diagnostics.len(),
        1,
        "identical setup failures must be deduplicated: {detail:#?}"
    );
    let stdout = detail["execution"]["stdout"].as_str().unwrap_or("");
    assert!(
        !stdout.contains("CRASHED"),
        "the reporter-facing result must not retain repeated target crash totals: {stdout}"
    );
    assert!(
        stdout.contains("Nuxt auto-import runtime unavailable: ref"),
        "the reporter should see one explicit environment result: {stdout}"
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail");
    let function = coverage["functions"]
        .as_array()
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"] == "useCounter")
        })
        .expect("useCounter coverage");
    assert_eq!(
        function["status"].as_str(),
        Some("skipped_no_fuzzable_surface")
    );
    assert!(
        function["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("Nuxt auto-import runtime")),
        "coverage must explain the environment blocker: {function:#?}"
    );
}
#[tokio::test]
async fn nuxt_adapter_loads_installed_vue_auto_import_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let vue_dir = dir.path().join("node_modules").join("vue");
    std::fs::create_dir_all(&vue_dir).unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"3.15.4","vue":"3.5.13"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("nuxt.config.ts"), "export default {};\n").unwrap();
    std::fs::write(
        vue_dir.join("package.json"),
        r#"{"name":"vue","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        vue_dir.join("index.js"),
        "export function ref(value) { return { value }; }\n",
    )
    .unwrap();
    let source_path = dir.path().join("useCounter.ts");
    let code = r#"
export function useCounter(): number {
  return ref(4).value;
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "installed Nuxt/Vue runtime should satisfy framework auto-imports: {:#?}",
        execute.detail
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail");
    assert!(coverage["functions"].as_array().is_some_and(|functions| {
        functions.iter().any(|function| {
            function["function"] == "useCounter" && function["status"] == "checked_direct"
        })
    }));
    let adapter = report
        .stages
        .iter()
        .find(|stage| stage.name == "project_adapter")
        .and_then(|stage| stage.detail.as_ref())
        .expect("project adapter detail");
    assert_eq!(adapter["adapter"]["kind"], "nuxt");
    assert_eq!(
        adapter["adapter"]["capabilities"]["framework_auto_import_runtime"],
        true
    );
    assert_eq!(adapter["surfaces"][0]["strategy"], "framework_runtime");
    let outcomes = report
        .stages
        .iter()
        .find(|stage| stage.name == "outcome_matrix")
        .and_then(|stage| stage.detail.as_ref())
        .expect("outcome matrix");
    assert_eq!(outcomes["static_analysis"], "passed");
    assert_eq!(outcomes["generated_execution"], "passed");
    assert_eq!(outcomes["authoritative_tests"], "not_run");
}

#[tokio::test]
async fn nuxt_adapter_defers_generated_execution_to_authoritative_project_test() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"3.15.4"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("nuxt.config.ts"), "export default {};\n").unwrap();
    let source_path = dir.path().join("useCounter.ts");
    let code = r#"
export function useCounter(value: number): number {
  return value + 1;
}
"#;
    std::fs::write(&source_path, code).unwrap();
    let test_path = dir.path().join("useCounter.test.ts");
    let tests = r#"
import assert from "node:assert/strict";
import { useCounter } from "./useCounter.ts";

assert.equal(useCounter(4), 5);
"#;
    std::fs::write(&test_path, tests).unwrap();

    let mut opts = default_opts(Some(tests));
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    opts.test_source_file = Some(test_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Skipped);
    assert_eq!(
        execute.detail.as_ref().unwrap()["reason"],
        "project_runner_selected"
    );
    let test = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage");
    assert_eq!(
        test.status,
        StageStatus::Passed,
        "the project test runner must own Nuxt execution: {:#?}",
        test.detail
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail");
    assert!(coverage["functions"].as_array().is_some_and(|functions| {
        functions.iter().any(|function| {
            function["function"] == "useCounter"
                && function["status"] == "checked_via_authoritative_test"
        })
    }));
    let adapter = report
        .stages
        .iter()
        .find(|stage| stage.name == "project_adapter")
        .and_then(|stage| stage.detail.as_ref())
        .expect("project adapter detail");
    assert_eq!(
        adapter["surfaces"][0]["strategy"],
        "authoritative_project_runner"
    );
    assert_eq!(
        report.verdict,
        VerificationVerdict::Pass,
        "the authoritative Nuxt test must satisfy the runtime contract: {:#?}",
        report.stages
    );
}

#[tokio::test]
async fn adapter_selects_execution_strategy_per_exported_surface() {
    let code = r#"
import type { ExternalContext } from "external-package";

export function increment(value: number): number {
  return value + 1;
}

export function readExternal(context: ExternalContext): string {
  return context.token;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let surfaces = report
        .stages
        .iter()
        .find(|stage| stage.name == "project_adapter")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["surfaces"].as_array())
        .expect("surface execution plans");
    let increment = surfaces
        .iter()
        .find(|surface| surface["surface_id"] == "increment:4")
        .expect("increment plan");
    assert_eq!(increment["strategy"], "generated_harness");
    assert_eq!(increment["expected_evidence"], "property_checked");
    let external = surfaces
        .iter()
        .find(|surface| surface["surface_id"] == "readExternal:8")
        .expect("external plan");
    assert_eq!(external["strategy"], "static_only");
    assert_eq!(external["expected_evidence"], "static_checked");
    assert!(
        external["unsupported_requirements"]
            .as_array()
            .is_some_and(|requirements| !requirements.is_empty()),
        "static-only plans must explain the missing capability: {external:#?}"
    );
}
#[tokio::test]
async fn ordinary_reference_error_remains_a_target_crash_outside_nuxt() {
    let code = r#"
export function brokenReference(value: number): number {
  return missingOrdinaryGlobal + value;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert_ne!(report.verdict, VerificationVerdict::Pass);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Failed);
    let detail = execute.detail.as_ref().expect("execute detail");
    assert!(
        detail["environment_setup"].is_null(),
        "ordinary ReferenceErrors must not be classified as framework setup"
    );
    assert!(
        detail["findings"].as_array().is_some_and(|findings| {
            findings.iter().any(|finding| {
                finding["error_type"] == "ReferenceError"
                    && finding["message"] == "missingOrdinaryGlobal is not defined"
            })
        }),
        "ordinary ReferenceError should remain a target finding: {detail:#?}"
    );
}

#[tokio::test]
async fn nuxt_project_composable_reference_error_is_an_environment_blocker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"3.15.4"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("nuxt.config.ts"), "export default {};\n").unwrap();
    let composables = dir.path().join("composables");
    let feature_dir = composables.join("feature");
    std::fs::create_dir_all(&feature_dir).unwrap();
    std::fs::write(
        composables.join("useProjectApi.ts"),
        "export function useProjectApi() { return { convert: (value: number) => value }; }\n",
    )
    .unwrap();
    let source_path = feature_dir.join("useFeature.ts");
    let code = r#"
export function useFeature(value: number): number {
  return useProjectApi().convert(value);
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail");
    assert!(
        detail["findings"].as_array().is_some_and(Vec::is_empty),
        "a project composable auto-import is framework runtime context, not a target crash: {detail:#?}"
    );
    assert_eq!(
        detail["environment_setup"]["missing_globals"]
            .as_array()
            .expect("missing globals"),
        &[serde_json::Value::String("useProjectApi".into())]
    );
}

#[tokio::test]
async fn nuxt_top_level_auto_import_reference_error_is_an_environment_blocker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"3.15.4"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("nuxt.config.ts"), "export default {};\n").unwrap();
    let source_path = dir.path().join("useTopLevelCounter.ts");
    let code = r#"
const counter = ref(0);

export function addToCounter(value: number): number {
  return counter.value + value;
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Inconclusive);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Inconclusive);
    let detail = execute.detail.as_ref().expect("execute detail");
    assert_eq!(
        detail["environment_setup"]["classification"].as_str(),
        Some("missing_nuxt_auto_import_runtime")
    );
    assert_eq!(
        detail["environment_setup"]["missing_globals"]
            .as_array()
            .expect("missing globals"),
        &[serde_json::Value::String("ref".into())]
    );
    assert!(
        detail["diagnostics"].as_array().is_some_and(|diagnostics| {
            diagnostics.iter().any(|diagnostic| {
                diagnostic["domain"] == "environment" && diagnostic["kind"] == "context_resolution"
            }) && diagnostics
                .iter()
                .all(|diagnostic| diagnostic["domain"] != "verifier_harness")
        }),
        "pre-bootstrap failure must be one explicit environment diagnostic: {detail:#?}"
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage detail");
    let function = coverage["functions"]
        .as_array()
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"] == "addToCounter")
        })
        .expect("addToCounter coverage");
    assert_eq!(
        function["status"].as_str(),
        Some("skipped_no_fuzzable_surface")
    );
}

#[tokio::test]
async fn nuxt_disabled_auto_import_reference_error_remains_a_target_crash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"3.15.4"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("nuxt.config.ts"),
        "export default defineNuxtConfig({ imports: { autoImport: false } });\n",
    )
    .unwrap();
    let source_path = dir.path().join("useCounter.ts");
    let code = r#"
export function useCounter() {
  return ref(0);
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = Some(dir.path().to_str().unwrap());
    opts.source_file = Some(source_path.to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Fail);
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Failed);
    let detail = execute.detail.as_ref().expect("execute detail");
    assert!(
        detail["environment_setup"].is_null(),
        "disabled auto-imports must not hide an unimported target reference: {detail:#?}"
    );
    assert!(
        detail["findings"].as_array().is_some_and(|findings| {
            findings.iter().any(|finding| {
                finding["error_type"] == "ReferenceError"
                    && finding["message"] == "ref is not defined"
            })
        }),
        "the unresolved ref must remain a target crash: {detail:#?}"
    );
}

#[tokio::test]
async fn feedback_corpus_persists_and_is_reused_across_verification_runs() {
    let project = tempfile::tempdir().unwrap();
    let output = project.path().join("reports");
    let source = project.path().join("bucket.ts");
    let code = r#"export function bucket(value: number): string {
  if (value === 777125) return "rare";
  if (value < 0) return "negative";
  return "other";
}"#;
    fs::write(&source, code).unwrap();
    let source_text = source.to_string_lossy().into_owned();
    let project_text = project.path().to_string_lossy().into_owned();
    let output_text = output.to_string_lossy().into_owned();

    let mut first_opts = default_opts(None);
    first_opts.project_dir = Some(&project_text);
    first_opts.source_file = Some(&source_text);
    first_opts.output_dir = Some(&output_text);
    let first = verify(code, &Language::TypeScript, first_opts).await;
    assert_eq!(first.verdict, VerificationVerdict::Pass, "{first:#?}");
    let first_coverage = first
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("first coverage detail");
    assert_eq!(first_coverage["corpus_loaded"], 0);
    assert!(
        first_coverage["corpus_retained"].as_u64().unwrap_or(0) > 0,
        "{first_coverage:#?}"
    );

    let mut second_opts = default_opts(None);
    second_opts.project_dir = Some(&project_text);
    second_opts.source_file = Some(&source_text);
    second_opts.output_dir = Some(&output_text);
    let second = verify(code, &Language::TypeScript, second_opts).await;
    assert_eq!(second.verdict, VerificationVerdict::Pass, "{second:#?}");
    let second_coverage = second
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("second coverage detail");
    assert!(
        second_coverage["corpus_loaded"].as_u64().unwrap_or(0) > 0,
        "{second_coverage:#?}"
    );
    assert!(fs::read_dir(&output).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".court-jester-corpus-")
    }));
}

#[tokio::test]
async fn typescript_shrinking_reaches_an_oracle_preserving_fixed_point() {
    let code = r#"export function explode(input: {
  token: string;
  noiseA?: string;
  noiseB?: string;
}): string {
  if (input.token === "boom") {
    throw new ReferenceError("stable crash");
  }
  return "ok";
}

function caller(): string {
  return explode({ token: "boom", noiseA: "discard-a", noiseB: "discard-b" });
}"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let finding = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["findings"].as_array())
        .and_then(|findings| {
            findings
                .iter()
                .find(|finding| finding["location"]["function"] == "explode")
        })
        .unwrap_or_else(|| panic!("explode finding: {report:#?}"));
    assert_eq!(
        finding["minimization"]["status"].as_str(),
        Some("preserved")
    );
    assert_eq!(
        finding["minimization"]["minimized"]["arguments"][0]["json_value"],
        serde_json::json!({
            "token": "boom"
        }),
        "fixed-point shrinking must discard both independent noise fields: {finding:#?}"
    );
}

#[test]
fn cli_atheris_adapter_runs_installed_engine_and_reports_crashing_input() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("target.py");
    fs::write(
        &source,
        "def explode(value: int) -> int:\n    if value == 7:\n        raise RuntimeError('native crash')\n    return value\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("atheris.py"),
        r#"class FuzzedDataProvider:
    def __init__(self, data):
        self.data = data
    def ConsumeIntInRange(self, lower, upper):
        return lower
    def ConsumeInt(self, size):
        return 7

_callback = None

def instrument_all():
    pass

def Setup(argv, callback):
    global _callback
    _callback = callback

def Fuzz():
    _callback(b"\x00" * 16)
"#,
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            dir.path().to_str().unwrap(),
            "--native-fuzz-engine",
            "atheris",
            "--native-fuzz-runs",
            "1",
            "--timeout-seconds",
            "10",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "native fuzz report must be JSON ({error}); stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    let native = report["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage["name"].as_str() == Some("native_fuzz"))
        })
        .unwrap_or_else(|| panic!("native_fuzz stage missing: {report:#?}"));

    assert_eq!(native["status"].as_str(), Some("failed"));
    assert_eq!(native["detail"]["engine"].as_str(), Some("atheris"));
    assert_eq!(native["detail"]["runs"].as_u64(), Some(1));
    assert_eq!(
        native["detail"]["native_findings"][0]["location"]["function"].as_str(),
        Some("explode")
    );
    assert_eq!(
        native["detail"]["native_findings"][0]["classification"].as_str(),
        Some("native_coverage_guided")
    );
}

#[test]
fn cli_llm_plateau_escape_executes_novel_seed_after_corpus_stalls() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("target.py");
    let output_dir = dir.path().join("reports");
    fs::write(
        &source,
        "def explode(value: str) -> str:\n    normalized = str(value)\n    score = sum((index + 1) * ord(char) for index, char in enumerate(normalized))\n    if len(normalized) == 10 and score == 5686:\n        raise IndexError('plateau crash')\n    return 'ok'\n",
    )
    .unwrap();
    let initial = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            dir.path().to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--timeout-seconds",
            "10",
        ])
        .env_remove("COURT_JESTER_LLM_PLATEAU_COMMAND")
        .output()
        .unwrap();
    let initial_report: serde_json::Value =
        serde_json::from_slice(&initial.stdout).unwrap_or_else(|error| {
            panic!(
                "initial report must be JSON ({error}); stdout={}; stderr={}",
                String::from_utf8_lossy(&initial.stdout),
                String::from_utf8_lossy(&initial.stderr)
            )
        });
    let retained = initial_report["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage["name"].as_str() == Some("coverage"))
        })
        .and_then(|stage| stage["detail"]["corpus_retained"].as_u64())
        .unwrap_or(0);
    assert!(
        retained > 0,
        "initial run must retain corpus history: {initial_report:#?}"
    );

    let prompt_path = dir.path().join("llm-prompt.json");
    let command = dir.path().join("propose-seeds");
    fs::write(
        &command,
        format!(
            "#!/bin/sh\ncat > '{}'\nprintf '%s\\n' '{{\"seeds\":[{{\"function\":\"explode\",\"arguments\":[\"llm-secret\"]}}]}}'\n",
            prompt_path.display()
        ),
    )
    .unwrap();
    make_executable(&command);
    let escaped = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            dir.path().to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--llm-plateau-command",
            command.to_str().unwrap(),
            "--timeout-seconds",
            "10",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&escaped.stdout).unwrap_or_else(|error| {
            panic!(
                "plateau report must be JSON ({error}); stdout={}; stderr={}",
                String::from_utf8_lossy(&escaped.stdout),
                String::from_utf8_lossy(&escaped.stderr)
            )
        });
    assert_eq!(
        escaped.status.code(),
        Some(1),
        "a discovered plateau crash is a target failure, not an infrastructure error"
    );
    assert_eq!(
        report["verdict"].as_str(),
        Some("fail"),
        "authoritative plateau findings must fail verification: {report:#?}"
    );
    let plateau = report["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage["name"].as_str() == Some("llm_plateau_escape"))
        })
        .unwrap_or_else(|| panic!("llm_plateau_escape stage missing: {report:#?}"));

    assert_eq!(
        plateau["status"].as_str(),
        Some("failed"),
        "plateau stage: {plateau:#?}"
    );
    assert_eq!(plateau["detail"]["accepted"].as_u64(), Some(1));
    assert!(
        plateau["detail"]["finding_count"].as_u64().unwrap_or(0) > 0,
        "plateau seed must produce an authoritative finding: {plateau:#?}"
    );
    let escaped_seed_found = report["stages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|stage| stage["name"].as_str() == Some("execute"))
        .and_then(|stage| stage["detail"]["findings"].as_array())
        .is_some_and(|findings| {
            findings.iter().any(|finding| {
                ["original", "minimized"].iter().any(|case| {
                    finding["minimization"][case]["arguments"][0]["json_value"].as_str()
                        == Some("llm-secret")
                })
            })
        });
    assert!(
        escaped_seed_found,
        "LLM-proposed seed must reach the reported crash: {report:#?}"
    );
    let prompt: serde_json::Value =
        serde_json::from_slice(&fs::read(prompt_path).unwrap()).unwrap();
    assert_eq!(prompt["protocol_version"].as_u64(), Some(1));
    assert_eq!(
        prompt["retained_corpus"]
            .as_object()
            .map(serde_json::Map::len),
        Some(1)
    );
}

#[tokio::test]
async fn typescript_keyof_any_switch_guards_produce_executable_seed_rows() {
    let code = r#"
const FieldValueType = {
  NUMBER: "NUMBER",
  STRING: "STRING",
  BOOLEAN: "BOOLEAN",
  JSON: "JSON",
  DATE: "DATE",
} as const;
type FieldValueType = keyof typeof FieldValueType;

export function isValidFieldValueType(type: FieldValueType, value: any): boolean {
  switch (type) {
    case "NUMBER": return typeof value === "number";
    case "STRING": return typeof value === "string";
    case "BOOLEAN": return typeof value === "boolean";
    case "JSON": return typeof value === "object";
    case "DATE": return typeof value === "string" && !Number.isNaN(Date.parse(value));
    default: return false;
  }
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "keyof/any guard seeds must reach generated execution: {report:#?}"
    );
    let detail = execute.detail.as_ref().expect("execute detail");
    assert!(
        detail["valid_invocations"].as_u64().unwrap_or(0) > 0,
        "switch literals must become valid invocations: {detail:#?}"
    );
    let rendered = serde_json::to_string(&report_json_value(&report, ReportLevel::Full)).unwrap();
    assert!(
        !rendered.contains("unresolved:keyof typeof"),
        "keyof typeof must resolve to a usable domain: {rendered}"
    );
}

#[tokio::test]
async fn typescript_constrained_generic_uses_constraint_domain() {
    let code = r#"
type ProductRerankCandidate = {
  preRankScore?: number;
  postRankScore?: number;
  modelNumber?: string | null;
};

export function productSearchReranker<T extends ProductRerankCandidate>(
  candidates: T[],
  fetchImpl: typeof fetch = fetch
): number {
  void fetchImpl;
  return candidates[0]?.postRankScore ?? candidates[0]?.preRankScore ?? 0;
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "the generic constraint must supply runnable candidates: {report:#?}"
    );
    assert!(
        execute
            .detail
            .as_ref()
            .and_then(|detail| detail["valid_invocations"].as_u64())
            .unwrap_or(0)
            > 0,
        "the constraint domain must produce valid invocations: {report:#?}"
    );
}

#[tokio::test]
async fn multiline_union_object_field_stays_one_domain_field() {
    let code = r#"
type Unit = {
  id: string
  name: string
}
type Specification = {
  id: string
  secondaryUnits?:
    | Unit[]
    | { getItems: () => Unit[] }
}
type Input = {
  specification: Specification
  value: unknown
  unit?: Unit | null
}

export function countUnits(input: Input): number {
  const value = input.specification.secondaryUnits;
  if (!value) return 0;
  return Array.isArray(value) ? value.length : value.getItems().length;
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let rendered = serde_json::to_string(&report_json_value(&report, ReportLevel::Full)).unwrap();
    assert!(
        !rendered.contains("\"| { getItems\""),
        "a union arm must not become a sibling property: {rendered}"
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "constructible omitted and array arms must execute: {report:#?}"
    );
}

#[tokio::test]
async fn planned_calls_survive_unsupported_sibling_surfaces() {
    let project = tempfile::tempdir().unwrap();
    let source_path = project.path().join("settings.ts");
    let caller_path = project.path().join("caller.ts");
    let code = r#"
type RuntimeConfig = { name: string; retries: number };
type StoredSettings = { token: symbol };

export const createDefaultSettings = (): RuntimeConfig => ({
  name: "default",
  retries: 4,
});

export function resolveSettings(settings: StoredSettings | null): string {
  return settings?.token.description ?? "default";
}
"#;
    let caller = r#"
import { createDefaultSettings, resolveSettings } from "./settings";
export const defaults = createDefaultSettings();
export const resolved = resolveSettings(null);
"#;
    fs::write(&source_path, code).unwrap();
    fs::write(&caller_path, caller).unwrap();

    let mut opts = default_opts(None);
    opts.project_dir = project.path().to_str();
    opts.source_file = source_path.to_str();
    let report = verify(code, &Language::TypeScript, opts).await;

    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Passed,
        "valid planned calls must execute despite unsupported generic generation: {report:#?}"
    );
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .expect("function coverage");
    for name in ["createDefaultSettings", "resolveSettings"] {
        let function = coverage
            .iter()
            .find(|function| function["function"] == name)
            .unwrap_or_else(|| panic!("missing {name} coverage: {coverage:#?}"));
        assert_eq!(
            function["status"].as_str(),
            Some("checked_direct"),
            "planned {name} call was discarded: {function:#?}"
        );
    }
}

#[tokio::test]
async fn object_predicate_seed_reaches_overflow_crash() {
    let code = r#"
export function routeJob(input: { kind: string; attempts: number }): string {
  if (input.kind === "priority" && input.attempts === 7) {
    throw new RangeError("priority retry overflow");
  }
  return input.kind;
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Failed,
        "the predicate-derived object must reproduce the crash: {report:#?}"
    );
    let findings = execute
        .detail
        .as_ref()
        .and_then(|detail| detail["findings"].as_array())
        .expect("execute findings");
    assert!(findings.iter().any(|finding| {
        finding["location"]["function"] == "routeJob"
            && finding["error_type"] == "RangeError"
            && finding["message"] == "priority retry overflow"
            && finding["input_classification"] == "valid"
            && finding["repro"]["arguments"][0]["json_value"]
                == serde_json::json!({
                    "kind": "priority",
                    "attempts": 7,
                })
    }));
}

#[tokio::test]
async fn multiline_object_predicate_seed_reaches_guarded_exception() {
    let code = r#"
export function dispatch(input: { kind: string; attempts: number }): string {
  if (
    input.kind === "priority"
    && input.attempts === 7
  ) {
    throw new RangeError("multiline guarded failure");
  }
  return input.kind;
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Failed,
        "the complete multiline predicate row must reach the guarded exception: {report:#?}"
    );
    assert!(
        execute
            .detail
            .as_ref()
            .and_then(|detail| detail["findings"].as_array())
            .is_some_and(|findings| findings.iter().any(|finding| {
                finding["location"]["function"] == "dispatch"
                    && finding["message"] == "multiline guarded failure"
                    && finding["repro"]["arguments"][0]["json_value"]
                        == serde_json::json!({
                            "kind": "priority",
                            "attempts": 7,
                        })
            })),
        "the multiline guard finding must retain the complete predicate input: {report:#?}"
    );
}

#[tokio::test]
async fn generated_application_overflow_range_error_is_uncertain() {
    let code = r#"
export function reserve(quantity: number): number {
  if (!Number.isFinite(quantity)) {
    throw new RangeError("quantity overflow");
  }
  return quantity;
}
"#;

    let report = verify(code, &Language::TypeScript, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(
        execute.status,
        StageStatus::Inconclusive,
        "an application RangeError on arbitrary generated rows has no admission evidence: {report:#?}"
    );
    assert!(
        execute
            .detail
            .as_ref()
            .and_then(|detail| detail["findings"].as_array())
            .is_some_and(|findings| !findings.is_empty()
                && findings
                    .iter()
                    .all(|finding| { finding["input_classification"] == "unknown" })),
        "the RangeError must remain an uncertain observation: {report:#?}"
    );
    assert_eq!(report.summary.findings.gating, 0);
}

fn parse_cli_json(output: &std::process::Output, context: &str) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{context} must be JSON ({error}); stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn run_cli_test_quality(
    source: &Path,
    test_file: &Path,
    project: &Path,
    language: &str,
    runner: Option<&str>,
    max_mutants: usize,
) -> serde_json::Value {
    let max_mutants = max_mutants.to_string();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"));
    command.args([
        "verify",
        "--file",
        source.to_str().unwrap(),
        "--language",
        language,
        "--project-dir",
        project.to_str().unwrap(),
        "--test-file",
        test_file.to_str().unwrap(),
        "--tests-only",
        "--test-quality",
        max_mutants.as_str(),
        "--report-level",
        "full",
    ]);
    if let Some(runner) = runner {
        command.args(["--test-runner", runner]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "test-quality verify failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_cli_json(&output, "test-quality report")
}

fn run_git(project: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: stdout={}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn initialize_git_project(project: &Path) {
    run_git(project, &["init", "--quiet"]);
    run_git(project, &["config", "user.email", "tests@example.com"]);
    run_git(project, &["config", "user.name", "Court Jester Tests"]);
}

fn commit_git_project(project: &Path, message: &str) -> String {
    run_git(project, &["add", "--all"]);
    run_git(project, &["commit", "--quiet", "-m", message]);
    run_git(project, &["rev-parse", "HEAD"]).trim().to_string()
}

fn run_cli_ci_test_quality(
    project: &Path,
    base: &str,
    test_files: &[&Path],
    max_mutants: usize,
) -> (std::process::Output, serde_json::Value) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"));
    command.current_dir(project).args([
        "ci",
        "--base",
        base,
        "--head",
        "HEAD",
        "--gate",
        "test",
        "--report",
        "json",
        "--report-level",
        "full",
    ]);
    for test_file in test_files {
        command.args(["--test-file", test_file.to_str().unwrap()]);
    }
    let max_mutants = max_mutants.to_string();
    command.args(["--test-quality", max_mutants.as_str()]);
    let output = command.output().unwrap();
    let report = parse_cli_json(&output, "ci report");
    (output, report)
}

fn test_quality_stage(report: &serde_json::Value) -> &serde_json::Value {
    report["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage["name"].as_str() == Some("test_quality"))
        })
        .unwrap_or_else(|| panic!("test_quality stage missing: {report:#?}"))
}

#[test]
fn test_quality_classifies_direct_weak_and_boundary_asserting_tests() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "def eligible(total: int) -> bool:\n    return total >= 100\n",
    )
    .unwrap();
    fs::write(&test_file, "import target\n\ntarget.eligible(100)\n").unwrap();

    let weak_report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let weak = test_quality_stage(&weak_report);
    assert_eq!(weak["status"].as_str(), Some("advisory"));
    assert_eq!(weak["detail"]["experimental"].as_bool(), Some(false));
    assert_eq!(weak["detail"]["mode"].as_str(), Some("advisory"));
    assert_eq!(weak["detail"]["max_mutants"].as_u64(), Some(1));
    assert_eq!(weak["detail"]["baseline_eligible"].as_bool(), Some(true));
    assert_eq!(weak["detail"]["counts"]["planned"].as_u64(), Some(1));
    assert_eq!(weak["detail"]["counts"]["survived"].as_u64(), Some(1));
    assert_eq!(weak["detail"]["counts"]["killed"].as_u64(), Some(0));
    assert_eq!(
        weak["detail"]["mutants"][0]["outcome"].as_str(),
        Some("survived")
    );
    assert_eq!(
        weak["detail"]["mutants"][0]["entered_mutated_surface"].as_bool(),
        Some(true)
    );
    assert_eq!(
        weak["detail"]["mutants"][0]["test_status"].as_str(),
        Some("passed")
    );
    assert_eq!(
        weak["detail"]["mutants"][0]["mutation"]["operator"].as_str(),
        Some("comparison_boundary")
    );
    assert!(
        weak["detail"].get("score").is_none()
            && weak["detail"].get("percentage").is_none()
            && weak["detail"].get("grade").is_none(),
        "test quality must report evidence rather than a synthetic score: {weak:#?}"
    );
    assert_eq!(
        weak_report["verdict"].as_str(),
        Some("pass"),
        "a surviving mutant is advisory and must not change the verifier verdict"
    );

    fs::write(
        &test_file,
        "import target\n\nassert target.eligible(100) is True\n",
    )
    .unwrap();
    let boundary_report =
        run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let boundary = test_quality_stage(&boundary_report);
    assert_eq!(boundary["status"].as_str(), Some("passed"));
    assert_eq!(boundary["detail"]["counts"]["planned"].as_u64(), Some(1));
    assert_eq!(boundary["detail"]["counts"]["killed"].as_u64(), Some(1));
    assert_eq!(boundary["detail"]["counts"]["survived"].as_u64(), Some(0));
    assert_eq!(
        boundary["detail"]["mutants"][0]["outcome"].as_str(),
        Some("killed")
    );
    assert_eq!(
        boundary["detail"]["mutants"][0]["entered_mutated_surface"].as_bool(),
        Some(true)
    );
    assert_eq!(
        boundary["detail"]["mutants"][0]["test_status"].as_str(),
        Some("failed")
    );
    assert_eq!(boundary_report["verdict"], weak_report["verdict"]);
    assert_eq!(
        boundary_report["strength"], weak_report["strength"],
        "advisory observations must not alter verification strength"
    );
}

#[test]
fn test_quality_coupling_findings_are_scoped_to_the_selected_target() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let support = project.path().join("support.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "_RATE = 0.9\n\ndef eligible(total: int) -> bool:\n    return total >= 100\n",
    )
    .unwrap();
    fs::write(&support, "_RATE = 0.5\n").unwrap();
    fs::write(
        &test_file,
        "import support\nimport target\n\nassert support._RATE == 0.5\nassert target._RATE == 0.9\nassert target.eligible(100) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let stage = test_quality_stage(&report);
    let findings = stage["detail"]["coupling_findings"]
        .as_array()
        .expect("coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "private access on an unrelated module must not be attributed to the target: {findings:#?}"
    );
    assert_eq!(findings[0]["kind"].as_str(), Some("private_target_access"));
    assert_eq!(findings[0]["symbol"].as_str(), Some("target._RATE"));
    assert_eq!(stage["status"].as_str(), Some("advisory"));
    assert_eq!(report["verdict"].as_str(), Some("pass"));
}

#[test]
fn test_quality_rejects_the_removed_experimental_flag() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "def eligible(total: int) -> bool:\n    return total >= 100\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import target\n\nassert target.eligible(100) is True\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            project.path().to_str().unwrap(),
            "--test-file",
            test_file.to_str().unwrap(),
            "--experimental-test-quality",
            "1",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the experimental spelling must not remain as a compatibility alias"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--experimental-test-quality"),
        "the CLI error must identify the removed option: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_quality_kills_typescript_public_boundary_mutant() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export function eligible(total: number): boolean {\n  return total >= 100;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { eligible } from './target.ts';\nif (eligible(100) !== true) throw new Error('boundary changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let stage = test_quality_stage(&report);
    assert_eq!(stage["detail"]["experimental"].as_bool(), Some(false));
    assert_eq!(stage["detail"]["mode"].as_str(), Some("advisory"));
    assert_eq!(stage["detail"]["counts"]["planned"].as_u64(), Some(1));
    assert_eq!(stage["detail"]["counts"]["killed"].as_u64(), Some(1));
    assert_eq!(stage["detail"]["counts"]["survived"].as_u64(), Some(0));
    assert_eq!(
        stage["detail"]["mutants"][0]["entered_mutated_surface"].as_bool(),
        Some(true)
    );
}

#[test]
fn ci_test_quality_uses_explicit_entrypoint_and_global_deterministic_budget() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let a_source = project.path().join("a.py");
    let b_source = project.path().join("b.py");
    let test_file = project.path().join("quality_checks.py");
    let wrong_language_test = project.path().join("wrong_language.test.ts");
    fs::write(
        &a_source,
        "def at_least(value: int) -> bool:\n    # baseline first surface\n    return value >= 1\n\ndef at_most(value: int) -> bool:\n    # baseline second surface\n    return value <= 10\n",
    )
    .unwrap();
    fs::write(
        &b_source,
        "def matches(value: int) -> bool:\n    # baseline first surface\n    return value == 2\n\ndef enabled() -> bool:\n    # baseline second surface\n    return True\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import a\nimport b\n\nassert a.at_least(1) is True\na.at_most(10)\nassert b.matches(2) is True\nassert b.enabled() is True\n",
    )
    .unwrap();
    fs::write(
        &wrong_language_test,
        "throw new Error('TypeScript entrypoint must not run for Python targets');\n",
    )
    .unwrap();
    let base = commit_git_project(project.path(), "baseline");

    fs::write(
        &a_source,
        "def at_least(value: int) -> bool:\n    # candidate first surface\n    return value >= 1\n\ndef at_most(value: int) -> bool:\n    # candidate second surface\n    return value <= 10\n",
    )
    .unwrap();
    fs::write(
        &b_source,
        "def matches(value: int) -> bool:\n    # candidate first surface\n    return value == 2\n\ndef enabled() -> bool:\n    # candidate second surface\n    return True\n",
    )
    .unwrap();
    commit_git_project(project.path(), "candidate");

    let (quality_output, quality_report) = run_cli_ci_test_quality(
        project.path(),
        &base,
        &[wrong_language_test.as_path(), test_file.as_path()],
        3,
    );
    assert!(
        quality_output.status.success(),
        "advisory quality evidence must not fail CI: stdout={}\nstderr={}",
        String::from_utf8_lossy(&quality_output.stdout),
        String::from_utf8_lossy(&quality_output.stderr)
    );
    let files = quality_report["files"]
        .as_array()
        .expect("ci files must be reported");
    assert_eq!(
        files
            .iter()
            .map(|file| file["file"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["a.py", "b.py"],
        "budget allocation order must be deterministic"
    );

    let a_quality = test_quality_stage(&files[0]["report"]);
    let b_quality = test_quality_stage(&files[1]["report"]);
    assert_eq!(a_quality["detail"]["max_mutants"].as_u64(), Some(2));
    assert_eq!(b_quality["detail"]["max_mutants"].as_u64(), Some(1));
    assert_eq!(a_quality["detail"]["counts"]["planned"].as_u64(), Some(2));
    assert_eq!(b_quality["detail"]["counts"]["planned"].as_u64(), Some(1));
    assert_eq!(a_quality["detail"]["counts"]["killed"].as_u64(), Some(1));
    assert_eq!(a_quality["detail"]["counts"]["survived"].as_u64(), Some(1));
    assert_eq!(b_quality["detail"]["counts"]["killed"].as_u64(), Some(1));
    assert_eq!(b_quality["detail"]["counts"]["survived"].as_u64(), Some(0));

    for (file, stage) in files.iter().zip([a_quality, b_quality]) {
        assert_eq!(stage["detail"]["baseline_eligible"].as_bool(), Some(true));
        let planned = stage["detail"]["counts"]["planned"].as_u64().unwrap();
        let mutants = stage["detail"]["mutants"]
            .as_array()
            .expect("per-file mutant evidence");
        assert_eq!(
            mutants.len() as u64,
            planned,
            "{} must retain evidence for every planned mutant",
            file["file"]
        );
        assert!(
            mutants.iter().all(|mutant| {
                mutant["mutation"]["surface_id"].is_string()
                    && mutant["mutation"]["operator"].is_string()
                    && mutant["outcome"].is_string()
                    && mutant["entered_mutated_surface"].as_bool() == Some(true)
            }),
            "{} has incomplete mutant evidence: {mutants:#?}",
            file["file"]
        );
    }

    let sum_count = |name: &str| {
        [a_quality, b_quality]
            .iter()
            .map(|stage| stage["detail"]["counts"][name].as_u64().unwrap())
            .sum::<u64>()
    };
    let coupling = [a_quality, b_quality]
        .iter()
        .map(|stage| {
            stage["detail"]["coupling_findings"]
                .as_array()
                .unwrap()
                .len() as u64
        })
        .sum::<u64>();
    assert_eq!(
        quality_report["test_quality"],
        serde_json::json!({
            "max_mutants": 3,
            "planned": sum_count("planned"),
            "killed": sum_count("killed"),
            "survived": sum_count("survived"),
            "invalid": sum_count("invalid"),
            "blocked": sum_count("blocked"),
            "no_coverage": sum_count("no_coverage"),
            "unjudged": sum_count("invalid") + sum_count("blocked") + sum_count("no_coverage"),
            "coupling": coupling,
        }),
        "the CI aggregate must be derived exactly from per-file evidence"
    );
    assert_eq!(sum_count("planned"), 3);

    assert_eq!(
        a_quality["status"].as_str(),
        Some("advisory"),
        "the deliberately weak at_most assertion must produce advisory evidence"
    );
    assert_eq!(quality_report["verdict"].as_str(), Some("pass"));
    assert_eq!(files[0]["verdict"].as_str(), Some("pass"));
    assert_eq!(files[0]["report"]["verdict"].as_str(), Some("pass"));
    assert!(
        files[0]["failing_gates"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "an advisory survivor must not become a failing CI gate: {:#?}",
        files[0]
    );
}

fn ci_file<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["files"]
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .find(|file| file["file"].as_str() == Some(name))
        })
        .unwrap_or_else(|| panic!("CI report is missing {name}: {report:#?}"))
}

#[test]
fn test_quality_attributes_same_line_mutations_to_the_exact_callable() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export function lower(value: number): boolean { return value >= 0; } export function upper(value: number): boolean { return value <= 10; }\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { lower, upper } from './target.ts';\nif (!lower(0) || !upper(10)) throw new Error('boundary changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        2,
    );
    let mutants = test_quality_stage(&report)["detail"]["mutants"]
        .as_array()
        .expect("same-line mutant evidence");
    assert_eq!(mutants.len(), 2, "{mutants:#?}");
    let mut surfaces = mutants
        .iter()
        .map(|mutant| mutant["mutation"]["surface_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    surfaces.sort_unstable();
    assert_eq!(
        surfaces,
        ["lower:1", "upper:1"],
        "each same-line callable must own only its byte-contained mutation"
    );
}

#[test]
fn test_quality_does_not_attribute_nested_arrow_mutation_to_exported_outer_function() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export function outer(flag: boolean): boolean {\n  const nested = (value: number): boolean => value >= 1;\n  return flag && nested(1);\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { outer } from './target.ts';\nif (outer(false) !== false) throw new Error('outer contract changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let stage = test_quality_stage(&report);
    assert_eq!(stage["detail"]["baseline_eligible"].as_bool(), Some(true));
    assert_eq!(
        stage["detail"]["counts"]["planned"].as_u64(),
        Some(0),
        "a nested callable's operator must not be planned against the exported outer surface: {stage:#?}"
    );
    assert_eq!(stage["detail"]["counts"]["survived"].as_u64(), Some(0));
    assert!(
        stage["detail"]["mutants"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "no misattributed outer mutant evidence may be emitted: {stage:#?}"
    );
}

#[test]
fn test_quality_attributes_same_line_class_methods_to_their_own_classes() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export class A { same(value: number): boolean { return value >= 1; } } export class B { same(value: number): boolean { return value <= 2; } }\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { A, B } from './target.ts';\nif (!new A().same(1) || !new B().same(2)) throw new Error('boundary changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        2,
    );
    let stage = test_quality_stage(&report);
    assert_eq!(stage["detail"]["baseline_eligible"].as_bool(), Some(true));
    assert_eq!(stage["detail"]["counts"]["planned"].as_u64(), Some(2));
    assert_eq!(stage["detail"]["counts"]["killed"].as_u64(), Some(2));
    let mutants = stage["detail"]["mutants"]
        .as_array()
        .expect("same-line class method mutant evidence");
    assert!(
        mutants
            .iter()
            .all(|mutant| mutant["outcome"].as_str() == Some("killed")),
        "both class method boundary mutants must be killed: {mutants:#?}"
    );
    let mut surfaces = mutants
        .iter()
        .map(|mutant| mutant["mutation"]["surface_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    surfaces.sort_unstable();
    assert_eq!(surfaces, ["A#same:1", "B#same:1"]);
}

#[test]
fn test_quality_ignores_typescript_type_booleans_but_mutates_runtime_booleans() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export interface Config { enabled: true; }\nexport function enabled(config: { expected: false }): boolean {\n  void config;\n  return true;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { enabled } from './target.ts';\nif (enabled({ expected: false }) !== true) throw new Error('runtime boolean changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        3,
    );
    let stage = test_quality_stage(&report);
    assert_eq!(
        stage["detail"]["counts"]["planned"].as_u64(),
        Some(1),
        "interface and parameter type literals are not executable mutation candidates: {stage:#?}"
    );
    let mutation = &stage["detail"]["mutants"][0]["mutation"];
    assert_eq!(mutation["operator"].as_str(), Some("boolean_literal"));
    assert_eq!(mutation["original"].as_str(), Some("true"));
    assert_eq!(mutation["line"].as_u64(), Some(4));
}

#[test]
fn test_quality_does_not_treat_a_bare_package_stem_collision_as_the_target() {
    let project = tempfile::tempdir().unwrap();
    let src_dir = project.path().join("src");
    let package_dir = project.path().join("node_modules").join("target");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&package_dir).unwrap();
    let source = src_dir.join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        package_dir.join("package.json"),
        r#"{"name":"target","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package_dir.join("index.js"), "export const _secret = 7;\n").unwrap();
    fs::write(
        &source,
        "export function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import * as external from 'target';\nimport { eligible } from './src/target.ts';\nif (external._secret !== 7 || !eligible(1)) throw new Error('bad result');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    assert!(
        test_quality_stage(&report)["detail"]["coupling_findings"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a bare external package is not a local import of the same-stem target: {report:#?}"
    );
}

#[test]
fn test_quality_side_effect_import_does_not_create_a_private_target_binding() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { expect, test } from 'bun:test';\nimport './target.ts';\nimport { eligible } from './target.ts';\nconst target = { _private: 7 };\ntest('side effect import', () => {\n  expect(target._private).toBe(7);\n  expect(eligible(1)).toBe(true);\n});\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    assert!(
        test_quality_stage(&report)["detail"]["coupling_findings"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a side-effect-only import must not bind a same-named local object: {report:#?}"
    );
}

#[test]
fn test_quality_ignores_introspection_of_an_unrelated_object() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "def eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import inspect\nimport target as t\n\nclass Unrelated:\n    pass\n\nassert 'Unrelated' in inspect.getsource(Unrelated)\nassert t.eligible(1) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    assert!(
        test_quality_stage(&report)["detail"]["coupling_findings"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the short target alias must not match arbitrary text inside another introspection call: {report:#?}"
    );
}

#[test]
fn test_quality_resolves_qualified_python_module_import_to_the_target_file() {
    let project = tempfile::tempdir().unwrap();
    let package = project.path().join("pkg");
    fs::create_dir_all(&package).unwrap();
    let source = package.join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(package.join("__init__.py"), "_cache = 'package-init'\n").unwrap();
    fs::write(
        &source,
        "_cache = 'target-module'\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import pkg.target as t\n\nassert t._cache == 'target-module'\nassert t.eligible(1) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("qualified Python coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "pkg.target must resolve to target.py rather than pkg/__init__.py: {findings:#?}"
    );
    assert_eq!(findings[0]["kind"].as_str(), Some("private_target_access"));
    assert_eq!(findings[0]["symbol"].as_str(), Some("t._cache"));
}

#[test]
fn test_quality_tracks_unaliased_qualified_python_import_without_binding_package_root() {
    let project = tempfile::tempdir().unwrap();
    let package = project.path().join("pkg");
    fs::create_dir_all(&package).unwrap();
    let source = package.join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(package.join("__init__.py"), "_cache = 'package-init'\n").unwrap();
    fs::write(
        &source,
        "_cache = 'target-module'\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import pkg.target\n\nassert pkg._cache == 'package-init'\nassert pkg.target._cache == 'target-module'\nassert pkg.target.eligible(1) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("unaliased qualified Python coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "the package root must not inherit target-module coupling: {findings:#?}"
    );
    assert_eq!(findings[0]["symbol"].as_str(), Some("pkg.target._cache"));
}

#[test]
fn test_quality_ignores_shadowed_typescript_target_parameter() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export const _cache = 1;\nexport function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import * as target from './target.ts';\nfunction localCache(target: { _cache: number }): number { return target._cache; }\nconst local = { _cache: 2 };\nif (localCache(local) !== 2) throw new Error('bad local');\nif (target._cache !== 1 || !target.eligible(1)) throw new Error('bad target');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("TypeScript shadowing coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "shadowed target parameter must not be attributed to the imported target: {findings:#?}"
    );
    assert_eq!(findings[0]["symbol"].as_str(), Some("target._cache"));
    assert_eq!(findings[0]["line"].as_u64(), Some(5));
}

#[test]
fn test_quality_ignores_shadowed_python_target_parameter() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "_cache = 1\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import target\n\ndef local_cache(target):\n    return target._cache\n\nlocal = type('Local', (), {'_cache': 2})()\nassert local_cache(local) == 2\nassert target._cache == 1\nassert target.eligible(1) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("Python shadowing coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "shadowed target parameter must not be attributed to the imported target: {findings:#?}"
    );
    assert_eq!(findings[0]["symbol"].as_str(), Some("target._cache"));
    assert_eq!(findings[0]["line"].as_u64(), Some(8));
}

#[test]
fn test_quality_ignores_nearer_sibling_module_when_root_target_is_selected() {
    let project = tempfile::tempdir().unwrap();
    let tests_dir = project.path().join("tests");
    fs::create_dir_all(&tests_dir).unwrap();
    let source = project.path().join("target.py");
    let sibling = tests_dir.join("target.py");
    let test_file = tests_dir.join("test_target.py");
    fs::write(
        &source,
        "_cache = 'selected-root'\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(&sibling, "_cache = 'nearer-sibling'\n").unwrap();
    fs::write(
        &test_file,
        "import target\nassert target._cache == 'nearer-sibling'\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            project.path().to_str().unwrap(),
            "--test-file",
            test_file.to_str().unwrap(),
            "--tests-only",
            "--test-quality",
            "1",
            "--report-level",
            "full",
        ])
        .output()
        .unwrap();
    let report = parse_cli_json(&output, "nearer-sibling coupling report");
    assert!(
        test_quality_stage(&report)["detail"]["coupling_findings"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the test directory's sibling target.py is not the selected root target.py: {report:#?}"
    );
}

#[test]
fn test_quality_respects_python_comprehension_shadowing_and_outer_scope() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(
        &source,
        "class Cache:\n    _cache = 2\n\n_cache = Cache()\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import target\nvalues = [target._cache for target in [target._cache]]\nassert values == [2]\nassert target._cache is not None\nassert target.eligible(1) is True\n",
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("comprehension coupling findings");
    assert_eq!(
        findings.len(),
        2,
        "the comprehension target shadows only its body, not its iterable or outer scope: {findings:#?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding["symbol"].as_str() == Some("target._cache")),
        "{findings:#?}"
    );
    let mut lines = findings
        .iter()
        .map(|finding| finding["line"].as_u64().unwrap())
        .collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, [2, 4]);
}

#[test]
fn test_quality_respects_function_scoped_typescript_var_shadowing() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export const _cache = 1;\nexport function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import * as target from './target.ts';\nfunction localCache(): number {\n  if (true) { var target = { _cache: 2 }; }\n  return target._cache;\n}\nif (localCache() !== 2) throw new Error('bad local');\nif (target._cache !== 1 || !target.eligible(1)) throw new Error('bad target');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("function-scoped var coupling findings");
    assert_eq!(
        findings.len(),
        1,
        "nested-block var shadows the imported target throughout its function: {findings:#?}"
    );
    assert_eq!(findings[0]["symbol"].as_str(), Some("target._cache"));
    assert_eq!(findings[0]["line"].as_u64(), Some(7));
}

#[test]
fn malformed_authoritative_sources_report_coupling_errors_without_partial_findings() {
    let cases = [
        (
            "python",
            "target.py",
            "test_target.py",
            "_cache = 7\n\ndef eligible(value: int) -> bool:\n    return value >= 1\n",
            "import target\nassert target._cache == 7\nassert target.eligible(1)\nif (\n",
            None,
        ),
        (
            "typescript",
            "target.ts",
            "target.test.ts",
            "export const _cache = 7;\nexport function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
            "import * as target from './target.ts';\nif (target._cache !== 7 || !target.eligible(1)) throw new Error('bad');\nfunction broken(\n",
            Some("bun"),
        ),
    ];

    for (language, source_name, test_name, code, tests, runner) in cases {
        let project = tempfile::tempdir().unwrap();
        let source = project.path().join(source_name);
        let test_file = project.path().join(test_name);
        fs::write(&source, code).unwrap();
        fs::write(&test_file, tests).unwrap();
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"));
        command.args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            language,
            "--project-dir",
            project.path().to_str().unwrap(),
            "--test-file",
            test_file.to_str().unwrap(),
            "--tests-only",
            "--test-quality",
            "1",
            "--report-level",
            "full",
        ]);
        if let Some(runner) = runner {
            command.args(["--test-runner", runner]);
        }
        let output = command.output().unwrap();
        let report = parse_cli_json(&output, "malformed authoritative report");
        let stage = test_quality_stage(&report);
        assert_eq!(
            stage["status"].as_str(),
            Some("advisory"),
            "coupling analysis failure remains advisory for {language}: {stage:#?}"
        );
        assert_eq!(stage["detail"]["mode"].as_str(), Some("advisory"));
        assert!(
            stage["detail"]["coupling_error"]
                .as_str()
                .is_some_and(|error| !error.is_empty()),
            "malformed {language} test must retain its coupling error: {stage:#?}"
        );
        assert!(
            stage["detail"]["coupling_findings"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "malformed {language} test must not leak partial coupling findings: {stage:#?}"
        );
    }
}

#[test]
fn test_quality_redacts_and_bounds_mutant_failure_excerpt() {
    const SECRET_VALUE: &str = "violet-cedar-7409-correct-horse";
    const SECRET_LINE: &str = "DATABASE_PASSWORD=violet-cedar-7409-correct-horse";
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    fs::write(&source, "def enabled() -> bool:\n    return True\n").unwrap();
    fs::write(
        &test_file,
        format!(
            "import sys\nimport target\nvalue = target.enabled()\nif not value:\n    print({secret:?}, file=sys.stderr)\nassert value is True\n",
            secret = SECRET_LINE,
        ),
    )
    .unwrap();

    let report = run_cli_test_quality(&source, &test_file, project.path(), "python", None, 1);
    let mutant = &test_quality_stage(&report)["detail"]["mutants"][0];
    assert_eq!(mutant["outcome"].as_str(), Some("killed"));
    let excerpt = mutant["failure_excerpt"]
        .as_str()
        .expect("bounded mutant failure excerpt");
    assert!(
        !excerpt.contains(SECRET_VALUE),
        "raw opaque secret leaked in {excerpt:?}"
    );
    assert!(
        excerpt.contains("[REDACTED]"),
        "redacted excerpt must use the stable marker: {excerpt:?}"
    );
    assert!(
        excerpt.chars().count() <= 1_000,
        "failure excerpt exceeded its serialization bound"
    );
}

#[test]
fn test_quality_parses_tsx_coupling_and_retains_authoritative_test_path() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.tsx");
    fs::write(
        &source,
        "export function eligible(value: number): boolean {\n  return value >= 2;\n}\n",
    )
    .unwrap();
    fs::write(
        project.path().join("tsconfig.json"),
        r#"{"compilerOptions":{"jsx":"react","jsxFactory":"h"}}"#,
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { expect, test } from 'bun:test';\nimport * as target from './target.ts';\nconst h = (..._args: unknown[]) => ({});\nfunction View() { return <target._Private />; }\ntest('tsx target', () => {\n  void View;\n  expect(target.eligible(2)).toBe(true);\n});\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let findings = test_quality_stage(&report)["detail"]["coupling_findings"]
        .as_array()
        .expect("TSX coupling findings");
    let finding = findings
        .iter()
        .find(|finding| finding["symbol"].as_str() == Some("target._Private"))
        .unwrap_or_else(|| panic!("TSX private target access was not reported: {findings:#?}"));
    assert_eq!(finding["kind"].as_str(), Some("private_target_access"));
    let reported_path = finding["test_source_file"]
        .as_str()
        .expect("coupling provenance must retain the authoritative test path");
    assert_eq!(
        normalize_logged_path(reported_path),
        normalize_logged_path(&test_file.to_string_lossy())
    );
}

#[test]
fn test_quality_baseline_process_blocker_prevents_mutant_execution() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    let execution_log = project.path().join("authoritative-runs.log");
    fs::write(
        &source,
        "def eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        format!(
            "from pathlib import Path\nimport socket\nimport sys\nimport target\nPath({log:?}).open('a').write('run\\n')\nassert target.eligible(1) is True\ntry:\n    socket.create_connection(('127.0.0.1', 9), timeout=0.01)\nexcept PermissionError as error:\n    print(error, file=sys.stderr)\n",
            log = execution_log.to_string_lossy(),
        ),
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            project.path().to_str().unwrap(),
            "--test-file",
            test_file.to_str().unwrap(),
            "--tests-only",
            "--test-quality",
            "1",
            "--report-level",
            "full",
        ])
        .output()
        .unwrap();
    let report = parse_cli_json(&output, "blocked-baseline test-quality report");
    let test_stage = report["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage["name"].as_str() == Some("test"))
        })
        .expect("authoritative test stage");
    assert_eq!(test_stage["status"].as_str(), Some("inconclusive"));
    let quality_stage = test_quality_stage(&report);
    assert_eq!(
        quality_stage["detail"]["baseline_eligible"].as_bool(),
        Some(false)
    );
    assert_eq!(
        quality_stage["detail"]["counts"]["planned"].as_u64(),
        Some(0)
    );
    let runs = fs::read_to_string(&execution_log).expect("baseline authoritative run");
    assert_eq!(
        runs.lines().count(),
        1,
        "a baseline process blocker must prevent all mutant test executions"
    );
}

#[test]
fn unsupported_commands_reject_test_quality() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--test-quality", "1"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "doctor must not silently accept a verify/ci-only option"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--test-quality"),
        "the CLI error must identify the unsupported option: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_quality_cli_cardinality_and_mode_errors_precede_input_lookup() {
    let cases: Vec<(&str, Vec<&str>, Vec<&str>)> = vec![
        (
            "verify quality without a test",
            vec!["verify", "--test-quality", "1"],
            vec!["--test-quality", "exactly one", "--test-file"],
        ),
        (
            "verify quality with two tests",
            vec![
                "verify",
                "--test-quality",
                "1",
                "--test-file",
                "a.py",
                "--test-file",
                "b.py",
            ],
            vec!["verify", "exactly one", "--test-file"],
        ),
        (
            "CI quality without a test",
            vec!["ci", "--test-quality", "1"],
            vec!["ci", "--test-quality", "requires", "--test-file"],
        ),
        (
            "CI test without quality mode",
            vec!["ci", "--test-file", "a.py"],
            vec!["ci", "--test-file", "requires", "--test-quality"],
        ),
        (
            "CI tests-only",
            vec!["ci", "--tests-only"],
            vec!["ci", "does not support", "--tests-only"],
        ),
    ];

    for (label, args, expected_fragments) in cases {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{label} must be a CLI usage error: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            expected_fragments
                .iter()
                .all(|fragment| stderr.contains(fragment)),
            "{label} returned the wrong usage error: {stderr}"
        );
    }
}

#[test]
fn test_quality_plans_exported_typescript_class_method_boundary() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    fs::write(
        &source,
        "export class Threshold {\n  accepts(value: number): boolean {\n    return value >= 1;\n  }\n}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import { Threshold } from './target.ts';\nif (!new Threshold().accepts(1)) throw new Error('boundary changed');\n",
    )
    .unwrap();

    let report = run_cli_test_quality(
        &source,
        &test_file,
        project.path(),
        "typescript",
        Some("bun"),
        1,
    );
    let stage = test_quality_stage(&report);
    assert_eq!(stage["detail"]["counts"]["planned"].as_u64(), Some(1));
    assert_eq!(stage["detail"]["counts"]["killed"].as_u64(), Some(1));
    let mutation = &stage["detail"]["mutants"][0]["mutation"];
    assert_eq!(mutation["operator"].as_str(), Some("comparison_boundary"));
    assert_eq!(mutation["surface_id"].as_str(), Some("Threshold#accepts:2"));
}

#[test]
fn ci_test_quality_rejects_duplicate_same_language_entrypoints() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let source = project.path().join("target.py");
    let first_test = project.path().join("first_test.py");
    let second_test = project.path().join("second_test.py");
    fs::write(
        &source,
        "def eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(&first_test, "import target\nassert target.eligible(1)\n").unwrap();
    fs::write(&second_test, "import target\nassert target.eligible(1)\n").unwrap();
    let base = commit_git_project(project.path(), "baseline");
    fs::write(
        &source,
        "def eligible(value: int) -> bool:\n    # changed\n    return value >= 1\n",
    )
    .unwrap();
    commit_git_project(project.path(), "candidate");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(project.path())
        .args([
            "ci",
            "--base",
            &base,
            "--head",
            "HEAD",
            "--report",
            "json",
            "--test-quality",
            "1",
            "--test-file",
            first_test.to_str().unwrap(),
            "--test-file",
            second_test.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "CI must reject two Python authoritative entrypoints"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at most one") && stderr.to_ascii_lowercase().contains("python"),
        "unexpected duplicate-entrypoint error: {stderr}"
    );
}

#[test]
fn ci_test_quality_routes_polyglot_entrypoints_in_both_argument_orders() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let python_source = project.path().join("python_target.py");
    let typescript_source = project.path().join("typescript_target.ts");
    let python_test = project.path().join("quality_checks.py");
    let typescript_test = project.path().join("quality_checks.test.ts");
    fs::write(
        &python_source,
        "def eligible(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &typescript_source,
        "export function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &python_test,
        "import python_target\nassert python_target.eligible(1) is True\n",
    )
    .unwrap();
    fs::write(
        &typescript_test,
        "import { expect, test } from 'bun:test';\nimport { eligible } from './typescript_target.ts';\ntest('boundary', () => expect(eligible(1)).toBe(true));\n",
    )
    .unwrap();
    let base = commit_git_project(project.path(), "baseline");
    fs::write(
        &python_source,
        "def eligible(value: int) -> bool:\n    # changed\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &typescript_source,
        "export function eligible(value: number): boolean {\n  // changed\n  return value >= 1;\n}\n",
    )
    .unwrap();
    commit_git_project(project.path(), "candidate");

    for test_files in [
        [python_test.as_path(), typescript_test.as_path()],
        [typescript_test.as_path(), python_test.as_path()],
    ] {
        let (output, report) = run_cli_ci_test_quality(project.path(), &base, &test_files, 2);
        assert!(
            output.status.success(),
            "polyglot advisory CI failed: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for file_name in ["python_target.py", "typescript_target.ts"] {
            let stage = test_quality_stage(&ci_file(&report, file_name)["report"]);
            assert_eq!(
                stage["detail"]["baseline_eligible"].as_bool(),
                Some(true),
                "{file_name} did not receive its language entrypoint: {stage:#?}"
            );
            assert_eq!(stage["detail"]["counts"]["planned"].as_u64(), Some(1));
            assert_eq!(stage["detail"]["counts"]["killed"].as_u64(), Some(1));
        }
    }
}

#[test]
fn ci_global_quality_cap_redistributes_past_unavailable_files() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let no_candidate = project.path().join("a_no_candidate.py");
    let no_entrypoint = project.path().join("b_no_entrypoint.ts");
    let first_candidate = project.path().join("c_candidate.py");
    let second_candidate = project.path().join("d_candidate.py");
    let test_file = project.path().join("quality_checks.py");
    fs::write(
        &no_candidate,
        "def identity(value: int) -> int:\n    return value\n",
    )
    .unwrap();
    fs::write(
        &no_entrypoint,
        "export function eligible(value: number): boolean {\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &first_candidate,
        "def lower(value: int) -> bool:\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &second_candidate,
        "def upper(value: int) -> bool:\n    return value <= 10\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "import a_no_candidate\nimport c_candidate\nimport d_candidate\nassert a_no_candidate.identity(1) == 1\nassert c_candidate.lower(1)\nassert d_candidate.upper(10)\n",
    )
    .unwrap();
    let base = commit_git_project(project.path(), "baseline");
    fs::write(
        &no_candidate,
        "def identity(value: int) -> int:\n    # changed\n    return value\n",
    )
    .unwrap();
    fs::write(
        &no_entrypoint,
        "export function eligible(value: number): boolean {\n  // changed\n  return value >= 1;\n}\n",
    )
    .unwrap();
    fs::write(
        &first_candidate,
        "def lower(value: int) -> bool:\n    # changed\n    return value >= 1\n",
    )
    .unwrap();
    fs::write(
        &second_candidate,
        "def upper(value: int) -> bool:\n    # changed\n    return value <= 10\n",
    )
    .unwrap();
    commit_git_project(project.path(), "candidate");

    let (_, report) = run_cli_ci_test_quality(project.path(), &base, &[test_file.as_path()], 2);
    let no_candidate_stage = test_quality_stage(&ci_file(&report, "a_no_candidate.py")["report"]);
    assert_eq!(
        no_candidate_stage["detail"]["counts"]["planned"].as_u64(),
        Some(0)
    );
    assert!(
        no_candidate_stage["message"]
            .as_str()
            .is_some_and(|message| message.to_ascii_lowercase().contains("candidate")),
        "no-candidate stage must explain why it did not consume budget: {no_candidate_stage:#?}"
    );
    let no_entrypoint_stage = test_quality_stage(&ci_file(&report, "b_no_entrypoint.ts")["report"]);
    assert_eq!(
        no_entrypoint_stage["detail"]["counts"]["planned"].as_u64(),
        Some(0)
    );
    assert!(
        no_entrypoint_stage["message"]
            .as_str()
            .is_some_and(|message| message.to_ascii_lowercase().contains("entrypoint")),
        "missing language entrypoint must remain explicit: {no_entrypoint_stage:#?}"
    );
    for file_name in ["c_candidate.py", "d_candidate.py"] {
        let stage = test_quality_stage(&ci_file(&report, file_name)["report"]);
        assert_eq!(
            stage["detail"]["counts"]["planned"].as_u64(),
            Some(1),
            "available candidate budget was not redistributed to {file_name}: {stage:#?}"
        );
    }
    assert_eq!(report["test_quality"]["planned"].as_u64(), Some(2));
}

#[test]
fn ci_candidate_with_zero_quota_retains_budget_exhausted_stage() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let sources = ["a.py", "b.py", "c.py"].map(|name| project.path().join(name));
    let test_file = project.path().join("quality_checks.py");
    for (index, source) in sources.iter().enumerate() {
        fs::write(
            source,
            format!("def eligible_{index}(value: int) -> bool:\n    return value >= {index}\n"),
        )
        .unwrap();
    }
    fs::write(
        &test_file,
        "import a\nimport b\nimport c\nassert a.eligible_0(0)\nassert b.eligible_1(1)\nassert c.eligible_2(2)\n",
    )
    .unwrap();
    let base = commit_git_project(project.path(), "baseline");
    for (index, source) in sources.iter().enumerate() {
        fs::write(
            source,
            format!(
                "def eligible_{index}(value: int) -> bool:\n    # changed\n    return value >= {index}\n"
            ),
        )
        .unwrap();
    }
    commit_git_project(project.path(), "candidate");

    let (_, report) = run_cli_ci_test_quality(project.path(), &base, &[test_file.as_path()], 2);
    assert_eq!(
        test_quality_stage(&ci_file(&report, "a.py")["report"])["detail"]["counts"]["planned"]
            .as_u64(),
        Some(1)
    );
    assert_eq!(
        test_quality_stage(&ci_file(&report, "b.py")["report"])["detail"]["counts"]["planned"]
            .as_u64(),
        Some(1)
    );
    let exhausted = test_quality_stage(&ci_file(&report, "c.py")["report"]);
    assert_eq!(exhausted["detail"]["counts"]["planned"].as_u64(), Some(0));
    assert!(
        exhausted["message"].as_str().is_some_and(|message| {
            let message = message.to_ascii_lowercase();
            message.contains("global") && message.contains("budget") && message.contains("exhaust")
        }),
        "candidate-bearing zero quota must be distinguishable from no candidates: {exhausted:#?}"
    );
}

#[test]
fn ci_nonzero_unjudged_equals_invalid_blocked_and_no_coverage() {
    let project = tempfile::tempdir().unwrap();
    initialize_git_project(project.path());
    let source = project.path().join("target.py");
    let test_file = project.path().join("quality_checks.py");
    fs::write(&source, "def enabled() -> bool:\n    return True\n").unwrap();
    fs::write(
        &test_file,
        "import subprocess\nimport target\nvalue = target.enabled()\nif not value:\n    subprocess.run(['echo', 'blocked'], check=True)\nassert value is True\n",
    )
    .unwrap();
    let base = commit_git_project(project.path(), "baseline");
    fs::write(
        &source,
        "def enabled() -> bool:\n    # changed\n    return True\n",
    )
    .unwrap();
    commit_git_project(project.path(), "candidate");

    let (output, report) =
        run_cli_ci_test_quality(project.path(), &base, &[test_file.as_path()], 1);
    assert!(
        output.status.success(),
        "a blocked mutant remains advisory: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary = &report["test_quality"];
    let invalid = summary["invalid"].as_u64().unwrap();
    let blocked = summary["blocked"].as_u64().unwrap();
    let no_coverage = summary["no_coverage"].as_u64().unwrap();
    let unjudged = summary["unjudged"].as_u64().unwrap();
    assert!(
        unjudged > 0,
        "the real process-policy blocker must exercise a nonzero unjudged bucket: {report:#?}"
    );
    assert_eq!(unjudged, invalid + blocked + no_coverage);
    assert_eq!(
        summary["unjudged"],
        test_quality_stage(&ci_file(&report, "target.py")["report"])["detail"]["counts"]["blocked"],
        "the single blocked campaign must aggregate without fabricating invalid mutants"
    );
}

#[tokio::test]
async fn react_hooks_require_an_authoritative_renderer_context() {
    let code = r#"
import { useQuery } from "@tanstack/react-query";

export function useCurrentGraph(tenantId: string) {
  return useQuery({ queryKey: ["graph", tenantId] });
}
"#;
    let mut opts = default_opts(None);
    opts.coverage_gate = CoverageGate::None;
    let report = verify(code, &Language::TypeScript, opts).await;

    assert_ne!(report.verdict, VerificationVerdict::Fail, "{report:#?}");
    assert_eq!(report.summary.findings.total, 0, "{report:#?}");
    let hook = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"] == "useCurrentGraph")
        })
        .expect("hook coverage");
    assert_eq!(hook["status"].as_str(), Some("skipped_no_fuzzable_surface"));
    assert!(hook["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("renderer")));
}

#[tokio::test]
async fn auto_runner_prefers_exact_node_test_file_over_package_vitest() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    let test_file = project.path().join("target.test.ts");
    let node_log = project.path().join("node.log");
    let tool_dir = project.path().join("node_modules").join(".bin");
    let code = "export function formatValue(value: number): number { return value; }\n";
    let tests = "import test from \"node:test\";\nimport { formatValue } from \"./target.ts\";\ntest(\"formats\", () => { if (formatValue(1) !== 1) throw new Error(\"bad\"); });\n";
    fs::write(&source, code).unwrap();
    fs::write(&test_file, tests).unwrap();
    fs::write(
        project.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"3.2.6"}}"#,
    )
    .unwrap();
    install_fake_tool_at(
        &tool_dir,
        "node",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"event\":\"target_entered\",\"surface_id\":\"formatValue:1\"}}' >&2\nprintf '%s\\n' 'TAP version 13' 'ok 1 - formats' '1..1'\n",
            node_log.display()
        ),
    );
    install_fake_tool_at(&tool_dir, "vitest", "#!/bin/sh\nexit 97\n");

    let mut opts = default_opts(Some(tests));
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_file.to_str();
    opts.tests_only = true;
    let report = verify(code, &Language::TypeScript, opts).await;

    let test = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage");
    assert_eq!(test.status, StageStatus::Passed, "{report:#?}");
    assert_eq!(
        test.detail.as_ref().unwrap()["test_runner_selected"].as_str(),
        Some("node")
    );
    let args = fs::read_to_string(node_log).unwrap();
    assert!(args.lines().any(|arg| arg == "--test"), "{args}");
}

#[tokio::test]
async fn authoritative_python_test_can_resolve_unclassified_generated_exceptions() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.py");
    let test_file = project.path().join("test_target.py");
    let code = "def correlation_for_incident(identity: str) -> str:\n    if not identity.startswith('INC-'):\n        raise ValueError('invalid incident identity')\n    return identity.lower()\n";
    let tests = "import target\nassert target.correlation_for_incident('INC-42') == 'inc-42'\n";
    fs::write(&source, code).unwrap();
    fs::write(&test_file, tests).unwrap();
    let mut opts = default_opts(Some(tests));
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_file.to_str();

    let report = verify(code, &Language::Python, opts).await;

    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
    assert_eq!(report.summary.findings.gating, 0, "{report:#?}");
    assert!(repair_summary(&report, &Language::Python)
        .findings
        .iter()
        .all(|finding| finding.input_classification == InputClassification::Unknown));
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail");
    assert_eq!(
        execute["harness_events"]["harness_completed"].as_bool(),
        Some(true)
    );
}

#[tokio::test]
async fn unclassified_python_domain_exceptions_do_not_become_process_failures() {
    let code = r#"class PolicyError(RuntimeError):
    pass

def load_policy(path: str) -> str:
    raise PolicyError("policy file not found")
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "{report:#?}"
    );
    assert!(report.summary.findings.total > 0, "{report:#?}");
    assert_eq!(report.summary.findings.gating, 0, "{report:#?}");
    assert!(repair_summary(&report, &Language::Python)
        .findings
        .iter()
        .all(|finding| finding.input_classification == InputClassification::Unknown));
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != FailureKind::NonzeroExit),
        "{report:#?}"
    );
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    assert_eq!(execute.status, StageStatus::Inconclusive);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == FailureKind::AmbiguousGeneratedInput));
    assert_eq!(
        repair_summary(&report, &Language::Python).recommended_action,
        "add_contract_or_test"
    );
}

#[tokio::test]
async fn python_interpreter_shutdown_does_not_invent_a_completed_rejection() {
    let code = "def stop(value: str) -> str:\n    raise SystemExit('stop')\n";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail");

    assert_eq!(
        execute["harness_events"]["harness_completed"].as_bool(),
        Some(false),
        "{report:#?}"
    );
    let invocation = &execute["harness_events"]["surfaces"]["stop:1"];
    assert_eq!(invocation["started"], 1);
    assert_eq!(invocation["completed"], 0);
    assert_eq!(invocation["rejected"], 0);
    assert_eq!(execute["valid_invocations"], 0);
    assert_eq!(report.summary.coverage.behaviorally_checked, 0);
    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .unwrap();
    assert_eq!(coverage["counts"]["reached_direct"], 1);
    assert_eq!(coverage["counts"]["checked_direct"], 0);
    assert_ne!(report.verdict, VerificationVerdict::Pass);
}

#[tokio::test]
async fn unresolved_python_plugin_context_is_not_fuzzed_with_primitives() {
    let code = r#"
from __future__ import annotations
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    from hermes import PluginContext

def register(context: PluginContext) -> None:
    context.register_slack_action_handler("decision", lambda: None)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert_ne!(report.verdict, VerificationVerdict::Fail, "{report:#?}");
    assert_eq!(report.summary.findings.total, 0, "{report:#?}");
    let register = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail["functions"].as_array())
        .and_then(|functions| {
            functions
                .iter()
                .find(|function| function["function"] == "register")
        })
        .expect("register coverage");
    assert_eq!(
        register["status"].as_str(),
        Some("skipped_unsupported_type")
    );
}

#[tokio::test]
async fn deeply_nested_python_syntax_returns_structured_inconclusive_report() {
    let depth = 2_000;
    let code = format!(
        "def normalize(value: {}int{}) -> int:\n    return 1\n",
        "list[".repeat(depth),
        "]".repeat(depth)
    );
    let report = verify(&code, &Language::Python, default_opts(None)).await;

    assert_eq!(
        report.verdict,
        VerificationVerdict::Inconclusive,
        "{report:#?}"
    );
    let parse = report
        .stages
        .iter()
        .find(|stage| stage.name == "parse")
        .expect("parse stage");
    assert_eq!(parse.status, StageStatus::Inconclusive);
    assert_eq!(
        parse.detail.as_ref().unwrap()["parse_diagnostics"][0]["kind"].as_str(),
        Some("unsupported")
    );
}

#[tokio::test]
async fn typescript_campaign_preserves_required_object_shape() {
    let project = tempfile::tempdir().unwrap();
    let code = r#"
export type ResourceType = 'IMAGE' | 'DOCUMENT'

export function resolveListingFeaturedImage(input: {
  canonicalProductFeaturedImage?: string | null
  itemFeaturedImage?: string | null
  resources: Array<{ resourceType: ResourceType; src: string }>
}): string | undefined {
  const imageResource = input.resources.find(
    (resource) => resource.resourceType === 'IMAGE'
  )
  return input.itemFeaturedImage ?? imageResource?.src ?? input.canonicalProductFeaturedImage ?? undefined
}
"#;

    let source = project.path().join("repro.ts");
    let test_file = project.path().join("repro.test.ts");
    let tests = r#"
import { resolveListingFeaturedImage } from './repro'
resolveListingFeaturedImage({
  canonicalProductFeaturedImage: 'https://example.com/product.png',
  itemFeaturedImage: undefined,
  resources: [{ resourceType: 'IMAGE', src: 'https://example.com/resource.png' }]
})
"#;
    fs::write(&source, code).unwrap();
    fs::write(&test_file, tests).unwrap();
    let mut opts = default_opts(Some(tests));
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    opts.test_source_file = test_file.to_str();
    let report = verify(code, &Language::TypeScript, opts).await;
    assert_eq!(report.summary.findings.total, 0, "{report:#?}");
    assert_eq!(report.verdict, VerificationVerdict::Pass, "{report:#?}");
}
