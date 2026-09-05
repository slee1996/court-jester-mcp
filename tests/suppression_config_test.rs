use court_jester::tools::verify::{validate_suppressions, verify, VerifyOptions};
use court_jester::types::*;

#[tokio::test]
async fn invalid_suppression_schema_blocks_library_verification_before_execution() {
    let invalid = [
        "not json",
        "null",
        r#"{"rules":null}"#,
        r#"{"rule":[]}"#,
        r#"{"rules":[{"functoin":"inspect"}]}"#,
        r#"{"rules":[{"severity":"typo"}]}"#,
        r#"{"rules":[{}]}"#,
        r#"{"rules":[{"path":" "}]}"#,
        r#"{"rules":[{"stage":"excute"}]}"#,
    ];
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("target.py");
    let marker = dir.path().join("executed");
    let code = format!(
        "from pathlib import Path\nPath({:?}).touch()\n",
        marker.to_str().unwrap()
    );
    std::fs::write(&source, &code).unwrap();
    for raw in invalid {
        assert!(validate_suppressions(raw).is_err(), "{raw}");
        for tests_only in [false, true] {
            let report = verify(
                &code,
                &Language::Python,
                VerifyOptions {
                    test_code: Some(&code),
                    test_source_file: Some(source.to_str().unwrap()),
                    test_runner: TestRunner::Auto,
                    tests_only,
                    test_quality_max_mutants: Some(1),
                    complexity_threshold: None,
                    complexity_metric: ComplexityMetric::Cyclomatic,
                    project_dir: Some(dir.path().to_str().unwrap()),
                    lint_config_path: None,
                    lint_virtual_file_path: None,
                    diff: None,
                    suppressions: Some(raw),
                    suppression_source: Some("suppression.json"),
                    auto_seed: false,
                    source_file: Some(source.to_str().unwrap()),
                    base_code: None,
                    base_source_file: None,
                    base_project_dir: None,
                    output_dir: None,
                    report_level: ReportLevel::Full,
                    execute_gate: ExecuteGate::All,
                    coverage_gate: CoverageGate::ChangedExports,
                    inferred_oracle_gate: InferredOracleGate::Advisory,
                    runtime_profile: RuntimeProfile::LocalTrusted,
                    memory_mb: 256,
                    network: NetworkPolicy::Deny,
                    harness_args: vec![],
                    python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
                    typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
                },
            )
            .await;
            assert_eq!(report.verdict, VerificationVerdict::Inconclusive, "{raw}");
            assert_eq!(report.stages[0].name, "configuration");
            assert_eq!(
                report.stages[0].detail.as_ref().unwrap()["diagnostic"]["kind"],
                "invalid_configuration"
            );
            let diagnostics = court_jester::tools::verify::stage_diagnostics(&report.stages[0]);
            assert!(diagnostics.iter().any(|diagnostic| diagnostic.kind
                == FailureKind::InvalidConfiguration
                && diagnostic.impact == DiagnosticImpact::Blocking));
            assert!(!marker.exists());
            assert!(report
                .stages
                .iter()
                .all(|stage| !matches!(stage.name.as_str(), "parse" | "lint" | "test")));
        }
    }
}

#[test]
fn empty_files_and_explicit_selectors_remain_valid() {
    for raw in [
        "{}",
        r#"{"rules":[]}"#,
        r#"{"rules":[{"stage":"execute"}]}"#,
        r#"{"rules":[{"path":"target.py","function":"inspect","severity":"crash","error_type":"ValueError","reason":"known"}]}"#,
    ] {
        assert!(validate_suppressions(raw).is_ok(), "{raw}");
    }
}
