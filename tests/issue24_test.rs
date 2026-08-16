use court_jester::tools::verify::{verify, VerifyOptions};
use court_jester::types::{
    ComplexityMetric, CoverageGate, ExecuteGate, InferredOracleGate, Language, NetworkPolicy,
    ReportLevel, RuntimeProfile, TestRunner, DEFAULT_PYTHON_DOCKER_IMAGE,
    DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};
use serde_json::{json, Value};

fn options() -> VerifyOptions<'static> {
    VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
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

fn arguments_for<'a>(findings: &'a [Value], function: &str) -> Vec<&'a Value> {
    findings
        .iter()
        .filter(|finding| finding["location"]["function"] == function)
        .flat_map(|finding| {
            finding["repro"]["arguments"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .collect()
}

#[tokio::test]
async fn typescript_findings_losslessly_serialize_and_reproduce_special_values() {
    let code = r#"
export function compareNullish(left: unknown, right: unknown): number {
  if (left === right) return 0
  if (left === undefined || left === null) return 1
  if (right === undefined || right === null) return -1
  return 0
}

export function compareNumbers(left: number, right: number): number {
  if (!Number.isFinite(left) || !Number.isFinite(right)) return 1
  return left === right ? 0 : left < right ? -1 : 1
}

export function compareArrays(left: unknown[], right: unknown[]): number {
  if (left.includes(undefined) || right.includes(undefined)) return 1
  return 0
}

export function compareObjects(left: { value: unknown }, right: { value: unknown }): number {
  if (left.value === undefined || right.value === undefined) return 1
  return 0
}
"#;

    let report = verify(code, &Language::TypeScript, options()).await;
    let execute = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage");
    let detail = execute.detail.as_ref().expect("execute detail");
    let findings = detail["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("findings missing from execute detail: {detail:#}"));

    let undefined_tag = json!({"type": "undefined"});
    let nullish_arguments = arguments_for(findings, "compareNullish");
    assert!(
        nullish_arguments
            .iter()
            .any(|argument| argument["json_value"] == undefined_tag),
        "undefined must remain distinct from null: {nullish_arguments:#?}"
    );
    assert!(
        findings.iter().any(|finding| {
            finding["location"]["function"] == "compareNullish"
                && finding["repro"]["snippet"]
                    .as_str()
                    .is_some_and(|snippet| snippet.contains("undefined"))
        }),
        "the nullish counterexample must use runnable undefined syntax"
    );

    let numeric_arguments = arguments_for(findings, "compareNumbers");
    for special in ["NaN", "Infinity", "-Infinity"] {
        let tag = json!({"type": "number", "value": special});
        assert!(
            numeric_arguments
                .iter()
                .any(|argument| argument["json_value"] == tag),
            "missing distinct {special} report value: {numeric_arguments:#?}"
        );
        assert!(
            findings.iter().any(|finding| {
                finding["location"]["function"] == "compareNumbers"
                    && finding["repro"]["snippet"]
                        .as_str()
                        .is_some_and(|snippet| snippet.contains(special))
            }),
            "missing runnable {special} repro syntax"
        );
    }

    let array_arguments = arguments_for(findings, "compareArrays");
    assert!(
        array_arguments.iter().any(|argument| {
            argument["json_value"]
                .as_array()
                .is_some_and(|items| items.contains(&undefined_tag))
        }),
        "nested array undefined values must be tagged: {array_arguments:#?}"
    );

    let object_arguments = arguments_for(findings, "compareObjects");
    assert!(
        object_arguments
            .iter()
            .any(|argument| argument["json_value"]["value"] == undefined_tag),
        "object keys with undefined values must be retained: {object_arguments:#?}"
    );
}

#[tokio::test]
async fn typescript_special_value_tags_do_not_collide_with_ordinary_objects() {
    let code = r#"
export function collideUndefined(
  special: unknown,
  ordinary: { type: "undefined" },
): number {
  if (special === undefined && ordinary?.type === "undefined") {
    throw new ReferenceError("undefined collision")
  }
  if (ordinary === undefined && (special as { type?: unknown })?.type === "undefined") {
    throw new ReferenceError("undefined collision")
  }
  return 0
}

export function collideNaN(
  special: number,
  ordinary: { type: "number"; value: "NaN" },
): number {
  if (Number.isNaN(special) && ordinary?.type === "number" && ordinary.value === "NaN") {
    throw new ReferenceError("NaN collision")
  }
  if (
    ordinary === undefined
    && (special as unknown as { type?: unknown })?.type === "number"
    && (special as unknown as { value?: unknown }).value === "NaN"
  ) {
    throw new ReferenceError("NaN collision")
  }
  return 0
}
"#;

    let report = verify(code, &Language::TypeScript, options()).await;
    let detail = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute detail");
    let findings = detail["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("findings missing from execute detail: {detail:#}"));

    for (function, special_tag, special_expression) in [
        (
            "collideUndefined",
            json!({"type": "undefined"}),
            "undefined",
        ),
        (
            "collideNaN",
            json!({"type": "number", "value": "NaN"}),
            "NaN",
        ),
    ] {
        let finding = findings
            .iter()
            .find(|finding| {
                finding["location"]["function"] == function
                    && finding["repro"]["arguments"][0]["expression"] == special_expression
            })
            .unwrap_or_else(|| panic!("missing {function} collision finding: {findings:#?}"));
        let arguments = finding["repro"]["arguments"]
            .as_array()
            .expect("repro arguments");
        assert_eq!(arguments[0]["json_value"], special_tag);
        assert_ne!(
            arguments[1]["json_value"], special_tag,
            "ordinary tag-shaped objects must not collide with special values: {finding:#}"
        );
        assert_eq!(
            arguments[1]["json_value"]["type"], "object",
            "ordinary tag-shaped objects need an explicit escape envelope: {finding:#}"
        );

        if function == "collideUndefined" {
            assert!(
                finding["minimization"]["attempts"]
                    .as_u64()
                    .is_some_and(|attempts| attempts > 21),
                "shrink dedupe must attempt the distinct ordinary-object candidate: {finding:#}"
            );
        }
    }
}
