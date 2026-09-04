use court_jester::tools::sandbox::parse_harness_events;
use court_jester::tools::verify::{repair_summary, replay_report, verify, VerifyOptions};
use court_jester::types::{
    ComplexityMetric, CoverageGate, ExecuteGate, InferredOracleGate, Language, NetworkPolicy,
    ReplayOutcome, ReportLevel, RuntimeProfile, TestRunner, DEFAULT_PYTHON_DOCKER_IMAGE,
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
async fn special_numeric_findings_preserve_values_through_persisted_replay() {
    let code = "export function inspect(value: number): number { if (Object.is(value, -0)) throw new ReferenceError('negative zero'); if (Number.isNaN(value)) throw new ReferenceError('nan'); if (value === Infinity) throw new ReferenceError('positive infinity'); if (value === -Infinity) throw new ReferenceError('negative infinity'); return value; }";
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    std::fs::write(&source, code).unwrap();
    let mut opts = options();
    opts.source_file = source.to_str();
    opts.project_dir = project.path().to_str();
    let report = verify(code, &Language::TypeScript, opts).await;
    let repair = repair_summary(&report, &Language::TypeScript);
    let path = project.path().join("repair.json");
    std::fs::write(&path, serde_json::to_vec(&repair).unwrap()).unwrap();
    for (message, value) in [
        ("negative zero", "-0"),
        ("nan", "NaN"),
        ("positive infinity", "Infinity"),
        ("negative infinity", "-Infinity"),
    ] {
        let finding = repair
            .findings
            .iter()
            .find(|finding| finding.message == message)
            .expect("numeric counterexample");
        assert_eq!(
            finding.repro.arguments[0].json_value,
            Some(json!({"type":"number","value":value}))
        );
        for (candidate, expected) in [
            (code, ReplayOutcome::Reproduced),
            (
                "export function inspect(value: number): number { return value; }",
                ReplayOutcome::NotReproduced,
            ),
        ] {
            std::fs::write(&source, candidate).unwrap();
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
            assert_eq!(replay.outcome, expected, "{value}: {replay:?}");
        }
    }
}

#[tokio::test]
async fn saved_typescript_corpus_values_are_decoded_before_invocation() {
    let code = r#"export function observe(value: unknown): number { _capture(value); return 0; }
function _capture(value: unknown): void {
  console.log('__OBSERVED__' + JSON.stringify(value, (_key, item) => {
    if (item === undefined) return { observed: 'undefined' };
    if (typeof item === 'number' && (Object.is(item, -0) || !Number.isFinite(item))) return { observed: Object.is(item, -0) ? '-0' : String(item) };
    return item;
  }));
}"#;
    let output = tempfile::tempdir().unwrap();
    let mut opts = options();
    opts.output_dir = output.path().to_str();
    verify(code, &Language::TypeScript, opts).await;
    let corpus = std::fs::read_dir(output.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".court-jester-corpus-")
        })
        .expect("saved corpus");
    for (encoded, observed) in [
        (json!({"type":"undefined"}), json!({"observed":"undefined"})),
        (
            json!({"type":"number","value":"NaN"}),
            json!({"observed":"NaN"}),
        ),
        (
            json!({"type":"number","value":"Infinity"}),
            json!({"observed":"Infinity"}),
        ),
        (
            json!({"type":"number","value":"-Infinity"}),
            json!({"observed":"-Infinity"}),
        ),
        (
            json!({"type":"number","value":"-0"}),
            json!({"observed":"-0"}),
        ),
        (
            json!({"type":"object","value":{"type":"undefined"}}),
            json!({"type":"undefined"}),
        ),
        (
            json!({"type":"object","value":{"type":"number","value":"NaN"}}),
            json!({"type":"number","value":"NaN"}),
        ),
        (
            json!({"type":"object","value":{"type":"number","value":"-0"}}),
            json!({"type":"number","value":"-0"}),
        ),
        (
            json!([{"type":"undefined"},{"type":"number","value":"NaN"},{"type":"object","value":{"type":"undefined"}}]),
            json!([{"observed":"undefined"},{"observed":"NaN"},{"type":"undefined"}]),
        ),
        (
            json!({"__proto__":{"type":"undefined"}}),
            json!({"__proto__":{"observed":"undefined"}}),
        ),
        (
            json!({"type":"object","value":{"type":"object","value":{"type":"object","value":{"type":"undefined"}}}}),
            json!({"type":"object","value":{"type":"undefined"}}),
        ),
        (
            json!({"type":"number","value":"ordinary"}),
            json!({"type":"number","value":"ordinary"}),
        ),
        (
            json!([true, null, 17, "plain"]),
            json!([true, null, 17, "plain"]),
        ),
    ] {
        std::fs::write(
            &corpus,
            serde_json::to_vec(&json!({"observe:1":[[encoded]]})).unwrap(),
        )
        .unwrap();
        let mut opts = options();
        opts.output_dir = output.path().to_str();
        let report = verify(code, &Language::TypeScript, opts).await;
        let detail = report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .and_then(|stage| stage.detail.as_ref())
            .unwrap();
        let first = detail["execution"]["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("__OBSERVED__"))
            .expect("target observation");
        assert_eq!(
            serde_json::from_str::<Value>(first).unwrap(),
            observed,
            "corpus seed must be invoked with decoded values"
        );
    }
    let mut too_deep = json!({"type":"undefined"});
    for _ in 0..66 {
        too_deep = json!([too_deep]);
    }
    for malformed in [json!({"type":"object","value":null}), too_deep] {
        std::fs::write(
            &corpus,
            serde_json::to_vec(&json!({"observe:1":[[malformed],["healthy"]]})).unwrap(),
        )
        .unwrap();
        let mut opts = options();
        opts.output_dir = output.path().to_str();
        let report = verify(code, &Language::TypeScript, opts).await;
        let detail = report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .and_then(|stage| stage.detail.as_ref())
            .unwrap();
        let first = detail["execution"]["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("__OBSERVED__"))
            .expect("target observation");
        assert_eq!(
            serde_json::from_str::<Value>(first).unwrap(),
            json!("healthy"),
            "malformed cached rows must not prevent subsequent valid rows from running"
        );
    }
}

#[tokio::test]
async fn typescript_generated_records_losslessly_serialize_special_values() {
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
    // Serialization is a producer contract. Public summaries deliberately
    // coalesce equivalent findings and keep only bounded representative inputs.
    let events = parse_harness_events(detail["execution"]["stdout"].as_str().unwrap()).unwrap();
    let raw_findings = serde_json::to_value(events.findings).unwrap();
    let findings = raw_findings.as_array().unwrap();

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

export function collideNegativeZero(special: number, ordinary: { type: "number"; value: "-0" }): number {
  if (Object.is(special, -0) && ordinary?.type === "number" && ordinary.value === "-0") {
    throw new ReferenceError("negative zero collision")
  }
  return 0
}
"#;

    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("target.ts");
    std::fs::write(&source, code).unwrap();
    let mut opts = options();
    opts.project_dir = project.path().to_str();
    opts.source_file = source.to_str();
    let report = verify(code, &Language::TypeScript, opts).await;
    let path = project.path().join("repair.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&repair_summary(&report, &Language::TypeScript)).unwrap(),
    )
    .unwrap();
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
        (
            "collideNegativeZero",
            json!({"type":"number","value":"-0"}),
            "-0",
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

        for (candidate, expected) in [(code, ReplayOutcome::Reproduced),
            ("export function collideUndefined(...args: unknown[]) { return 0; }\nexport function collideNaN(...args: unknown[]) { return 0; }\nexport function collideNegativeZero(...args: unknown[]) { return 0; }", ReplayOutcome::NotReproduced)] {
            std::fs::write(&source, candidate).unwrap();
            let replay = replay_report(path.to_str().unwrap(), finding["id"].as_str().unwrap(), None,
                RuntimeProfile::LocalTrusted, DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE).await.unwrap();
            assert_eq!(replay.outcome, expected, "{function}: {replay:?}");
        }
    }
}
