use court_jester::tools::sandbox::{parse_harness_events, HARNESS_EVENT_SENTINEL};
use serde_json::{json, Value};

fn stream(events: Vec<Value>) -> String {
    let bootstrap = vec![
        json!({"event":"bootstrap_started"}),
        json!({"event":"target_resolved","data":{"module":"fixture"}}),
        json!({"event":"target_ready"}),
    ];
    bootstrap
        .into_iter()
        .chain(events)
        .enumerate()
        .map(|(sequence, mut event)| {
            event["sequence"] = json!(sequence);
            event["protocol_version"] = json!(2);
            format!("{HARNESS_EVENT_SENTINEL}{event}\n")
        })
        .collect()
}

fn start(surface: &str, iteration: usize, validity: &str) -> Value {
    json!({"event":"unit_started","data":{"surface_id":surface,"iteration":iteration,"input_classification":validity,"input_origin":"generated"}})
}

fn finish(surface: &str, iteration: usize, outcome: &str) -> Value {
    json!({"event":"unit_completed","data":{"surface_id":surface,"iteration":iteration,"outcome":outcome}})
}

fn check(surface: &str, iteration: usize, passed: bool) -> Value {
    json!({"event":"oracle_evaluated","data":{"surface_id":surface,"iteration":iteration,"oracle_id":"type","passed":passed}})
}

#[test]
fn checks_require_matching_valid_completed_invocations() {
    let output = stream(vec![
        start("a:1", 0, "valid"),
        check("a:1", 0, true),
        check("a:1", 0, false),
        finish("a:1", 0, "target_exception"),
        start("a:1", 1, "unknown"),
        check("a:1", 1, true),
        finish("a:1", 1, "passed"),
        start("a:1", 2, "valid"),
        check("a:1", 2, true),
        finish("a:1", 2, "rejected"),
        start("a:1", 3, "invalid"),
        check("a:1", 3, true),
        finish("a:1", 3, "passed"),
        start("a:1", 4, "valid"),
        check("a:1", 4, true),
    ]);
    let summary = parse_harness_events(&output).unwrap();
    assert_eq!(
        (
            summary.surfaces["a:1"].passed_oracles,
            summary.surfaces["a:1"].failed_oracles
        ),
        (1, 1)
    );
    for events in [
        vec![check("a:1", 0, true)],
        vec![start("a:1", 0, "valid"), check("b:1", 0, true)],
        vec![start("a:1", 0, "valid"), check("a:1", 1, true)],
    ] {
        assert!(parse_harness_events(&stream(events)).is_err());
    }
}

#[test]
fn a_failed_check_cannot_be_closed_as_a_passed_unit() {
    assert!(parse_harness_events(&stream(vec![
        start("a:1", 0, "valid"),
        check("a:1", 0, false),
        finish("a:1", 0, "passed")
    ]))
    .is_err());
}

#[test]
fn unclassified_exceptions_do_not_supply_valid_invocation_or_check_credit() {
    let summary = parse_harness_events(&stream(vec![
        start("a:1", 0, "valid"),
        check("a:1", 0, true),
        finish("a:1", 0, "unclassified_exception"),
    ]))
    .unwrap();
    let evidence = &summary.surfaces["a:1"];
    assert_eq!(
        (
            evidence.completed,
            evidence.unknown_completed,
            evidence.valid_completed,
            evidence.passed_oracles
        ),
        (1, 1, 0, 0)
    );
}

#[test]
fn check_protocol_preserves_legacy_streams_without_accepting_mixed_versions() {
    let current = stream(vec![
        start("a:1", 0, "valid"),
        check("a:1", 0, true),
        finish("a:1", 0, "passed"),
    ]);
    assert!(parse_harness_events(&current.replacen(
        "\"protocol_version\":2",
        "\"protocol_version\":1",
        1
    ))
    .is_err());
    assert!(parse_harness_events(
        &current.replace("\"protocol_version\":2", "\"protocol_version\":1")
    )
    .is_err());
    let legacy = stream(vec![start("a:1", 0, "valid"), finish("a:1", 0, "passed")])
        .replace("\"protocol_version\":2", "\"protocol_version\":1");
    let summary = parse_harness_events(&legacy).unwrap();
    assert_eq!(summary.surfaces["a:1"].valid_completed, 1);
    assert_eq!(summary.surfaces["a:1"].passed_oracles, 0);
}

#[test]
fn duplicate_invocation_identity_cannot_inflate_evidence() {
    let output = stream(vec![
        start("check:1", 0, "valid"),
        finish("check:1", 0, "passed"),
        start("check:1", 0, "valid"),
        finish("check:1", 0, "passed"),
    ]);
    assert!(
        parse_harness_events(&output).is_err(),
        "a repeated invocation must not count twice"
    );
}

#[test]
fn only_valid_completed_invocations_supply_behavioral_evidence() {
    let output = stream(vec![
        start("a:1", 0, "valid"),
        finish("a:1", 0, "passed"),
        start("a:1", 1, "valid"),
        finish("a:1", 1, "target_exception"),
        start("a:1", 2, "valid"),
        finish("a:1", 2, "rejected"),
        start("a:1", 3, "invalid"),
        finish("a:1", 3, "passed"),
        start("a:1", 4, "unknown"),
        finish("a:1", 4, "target_exception"),
        start("b:2", 0, "valid"),
    ]);
    let summary = parse_harness_events(&output).unwrap();
    let a = &summary.surfaces["a:1"];
    assert_eq!(
        (
            a.started,
            a.completed,
            a.valid_completed,
            a.rejected,
            a.invalid_completed,
            a.unknown_completed
        ),
        (5, 5, 2, 1, 1, 1)
    );
    let b = &summary.surfaces["b:2"];
    assert_eq!((b.started, b.completed, b.valid_completed), (1, 0, 0));
    assert!(!summary.harness_completed);
}

#[test]
fn human_log_lines_cannot_manufacture_invocation_evidence() {
    let output = format!(
        "{}FUZZ uncalled: 100 passed, 0 rejected (of 100)\n",
        stream(vec![])
    );
    assert!(parse_harness_events(&output).unwrap().surfaces.is_empty());
}
