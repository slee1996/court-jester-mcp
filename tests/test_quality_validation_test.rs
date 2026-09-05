#[path = "../examples/test_quality_validation.rs"]
mod corpus;

#[test]
fn invalid_candidates_and_valid_controls_use_the_production_validation_boundary() {
    let report = corpus::run_cases();
    assert_eq!(report["status"], "passed", "{report:#}");
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    assert_eq!(
        cases
            .iter()
            .filter(|case| case["classification"] == "invalid")
            .count(),
        10
    );
    assert!(cases
        .iter()
        .all(|case| case["mutant_execution_started"] == false));
    for language in ["python", "typescript"] {
        assert_eq!(
            cases
                .iter()
                .filter(|case| case["language"] == language && case["classification"] == "valid")
                .count(),
            1
        );
    }
}
