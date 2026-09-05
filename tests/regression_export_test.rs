use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(args)
        .output()
        .unwrap()
}

fn fixture(
    root: &Path,
    language: &str,
    code: &str,
    oracle: Option<&str>,
) -> (PathBuf, PathBuf, String) {
    let source = root.join(if language == "python" {
        "target.py"
    } else {
        "target.ts"
    });
    std::fs::write(&source, code).unwrap();
    let output = cli(&[
        "verify",
        "--file",
        source.to_str().unwrap(),
        "--language",
        language,
        "--project-dir",
        root.to_str().unwrap(),
        "--summary",
        "repair-json",
    ]);
    let mut report: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    let finding = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["input_classification"] == "valid"
                && oracle.is_none_or(|oracle| finding["oracle"]["kind"] == oracle)
        })
        .unwrap_or_else(|| panic!("no eligible finding: {report}"))
        .clone();
    let id = finding["id"].as_str().unwrap().to_string();
    report["findings"] = serde_json::json!([finding]);
    let report_path = root.join("original-report.json");
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    (source, report_path, id)
}

fn export(root: &Path, report: &Path, id: &str, output: &Path, accept: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_court-jester"));
    command
        .args(["replay", "--report"])
        .arg(report)
        .args(["--finding", id, "--dependency-project-dir"])
        .arg(root)
        .arg("--export-regression")
        .arg(output);
    if accept {
        command.arg("--accept-inferred");
    }
    command.output().unwrap()
}

fn run_test(bundle: &Path, language: &str, binary: &str) -> Output {
    let mut command = Command::new(if language == "python" {
        "python3"
    } else {
        "node"
    });
    if language != "python" {
        command.arg("--test");
    }
    command
        .arg(bundle.join(if language == "python" {
            "test_regression.py"
        } else {
            "regression.test.mjs"
        }))
        .env("COURT_JESTER_BINARY", binary)
        .output()
        .unwrap()
}

#[test]
fn exported_regressions_require_positive_completion_and_current_source() {
    for (language, bug, other_bug, fixed) in [
        ("python", "def first_character(value: str) -> str:\n    return value[0]\n",
         "def first_character(value: str) -> str:\n    raise ValueError('different failure')\n",
         "def first_character(value: str) -> str:\n    return value[0] if value else ''\n"),
        ("typescript", "export function firstCharacter(value: string): string { return value[0].toUpperCase(); }",
         "export function firstCharacter(value: string): string { throw new Error('different failure'); }",
         "export function firstCharacter(value: string): string { return value[0]?.toUpperCase() ?? ''; }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (source, report, id) = fixture(root.path(), language, bug, Some("runtime_contract"));
        let bundle = root.path().join("regression");
        let result = export(root.path(), &report, &id, &bundle, false);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let binary = env!("CARGO_BIN_EXE_court-jester");
        assert!(!run_test(&bundle, language, binary).status.success(), "original bug must fail");
        std::fs::write(&source, other_bug).unwrap();
        assert!(!run_test(&bundle, language, binary).status.success(), "different bug must not pass");
        std::fs::write(&source, fixed).unwrap();
        let result = run_test(&bundle, language, binary);
        assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        assert!(!run_test(&bundle, language, "/missing/court-jester").status.success());
        std::fs::remove_file(&source).unwrap();
        assert!(!run_test(&bundle, language, binary).status.success(), "missing candidate must fail");
    }
}

#[test]
fn inferred_export_requires_acceptance_and_keeps_original_confidence() {
    let root = tempfile::tempdir().unwrap();
    let (source, report, id) = fixture(root.path(), "python",
        "# court-jester-properties pep440_version_ordering\ndef compare_versions(left: str, right: str) -> int:\n    return 0\n", Some("inferred_semantic"));
    let bundle = root.path().join("regression");
    let denied = export(root.path(), &report, &id, &bundle, false);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("--accept-inferred"));
    assert!(!bundle.exists());
    let accepted = export(root.path(), &report, &id, &bundle, true);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("regression.json")).unwrap()).unwrap();
    assert_eq!(manifest["accepted_inferred"], true);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("report.json")).unwrap()).unwrap();
    assert_eq!(
        saved["stages"][0]["detail"]["findings"][0]["confidence"],
        "low"
    );
    assert!(
        !run_test(&bundle, "python", env!("CARGO_BIN_EXE_court-jester"))
            .status
            .success()
    );
    let expected = saved["stages"][0]["detail"]["findings"][0]["oracle"]["expected"]
        .as_str()
        .unwrap();
    let expected: Value = serde_json::from_str(expected).unwrap();
    // Satisfy this recorded observation, not the whole comparison specification.
    std::fs::write(
        source,
        format!("def compare_versions(left: str, right: str) -> int:\n    return {expected}\n"),
    )
    .unwrap();
    let fixed = run_test(&bundle, "python", env!("CARGO_BIN_EXE_court-jester"));
    assert!(
        fixed.status.success(),
        "{}",
        String::from_utf8_lossy(&fixed.stderr)
    );
}

#[test]
fn replay_requires_well_formed_evidence_and_successful_process() {
    let root = tempfile::tempdir().unwrap();
    let (_, path, id) = fixture(
        root.path(),
        "python",
        "def first_character(value: str) -> str:\n    return value[0]\n",
        None,
    );
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (reproduced, positive, suffix) in [
        (Some(false), Some(true), "raise SystemExit(7)"),
        (None, Some(true), ""),
        (Some(true), Some(true), ""),
        (Some(false), None, ""),
    ] {
        let mut report = original.clone();
        let mut payload = report["findings"][0]["repro"]["expectation"].clone();
        if let Some(value) = reproduced {
            payload["reproduced"] = Value::Bool(value);
        }
        if let Some(value) = positive {
            payload["check_passed"] = Value::Bool(value);
        }
        let literal = serde_json::to_string(&payload.to_string()).unwrap();
        report["findings"][0]["repro"]["snippet"] = Value::String(format!(
            "print('__COURT_JESTER_REPLAY_JSON__')\nprint({literal})\n{suffix}\n"
        ));
        std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        let replay = cli(&[
            "replay",
            "--report",
            path.to_str().unwrap(),
            "--finding",
            &id,
        ]);
        let value: Value = serde_json::from_slice(&replay.stdout).unwrap();
        let expected = if positive.is_none() {
            "not_reproduced"
        } else {
            "inconclusive"
        };
        assert_eq!(value["outcome"], expected, "{value}");
        assert!(value.get("check_passed").is_none());
        let bundle = root.path().join("regression");
        assert!(!export(root.path(), &path, &id, &bundle, false)
            .status
            .success());
        assert!(!bundle.exists());
    }
}

#[test]
fn regression_export_is_relocatable_and_never_overwrites_existing_output() {
    let container = tempfile::tempdir().unwrap();
    let root = container.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let (source, report, id) = fixture(
        &root,
        "python",
        "def first_character(value: str) -> str:\n    return value[0]\n",
        None,
    );
    let bundle = root.join("regression");
    let outputs = std::thread::scope(|scope| {
        let first = scope.spawn(|| export(&root, &report, &id, &bundle, false));
        let second = scope.spawn(|| export(&root, &report, &id, &bundle, false));
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(
        outputs
            .iter()
            .filter(|result| result.status.success())
            .count(),
        1
    );
    let manifest = std::fs::read(bundle.join("regression.json")).unwrap();
    assert!(!export(&root, &report, &id, &bundle, false).status.success());
    assert_eq!(
        std::fs::read(bundle.join("regression.json")).unwrap(),
        manifest
    );
    std::fs::write(
        &source,
        "def first_character(value: str) -> str:\n    return value[0] if value else ''\n",
    )
    .unwrap();
    let moved = container.path().join("moved project");
    std::fs::rename(&root, &moved).unwrap();
    let result = run_test(
        &moved.join("regression"),
        "python",
        env!("CARGO_BIN_EXE_court-jester"),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn export_preserves_effective_replay_limits_and_refuses_unknown_inputs() {
    let root = tempfile::tempdir().unwrap();
    let (_, report, id) = fixture(
        root.path(),
        "python",
        "def first_character(value: str) -> str:\n    return value[0]\n",
        None,
    );
    let bundle = root.path().join("regression");
    let output = cli(&[
        "replay",
        "--report",
        report.to_str().unwrap(),
        "--finding",
        &id,
        "--dependency-project-dir",
        root.path().to_str().unwrap(),
        "--export-regression",
        bundle.to_str().unwrap(),
        "--runtime-profile",
        "local-trusted",
        "--timeout-seconds",
        "5",
        "--memory-mb",
        "256",
        "--network",
        "deny",
        "--harness-args-json",
        "[{\"literal\":\"marker\"}]",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("report.json")).unwrap()).unwrap();
    let launch = &saved["stages"][0]["detail"]["findings"][0]["launch_context"];
    assert_eq!(launch["limits"]["runtime_profile"], "local-trusted");
    assert_eq!(launch["limits"]["timeout_seconds"], 5.0);
    assert_eq!(launch["limits"]["memory_mb"], 256);
    assert_eq!(launch["limits"]["network_policy"], "deny");
    assert_eq!(launch["harness_args"][0]["literal"], "marker");
    let mut original: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    original["findings"][0]["input_classification"] = Value::String("unknown".into());
    std::fs::write(&report, serde_json::to_vec(&original).unwrap()).unwrap();
    let denied = root.path().join("unknown-regression");
    let result = export(root.path(), &report, &id, &denied, true);
    assert!(!result.status.success());
    assert!(!denied.exists());
}

#[test]
fn property_pair_and_factory_exports_fail_before_and_pass_after_repair() {
    for (language, oracle, bug, fixed) in [
        ("python", "declared_property", "# court-jester-properties bounded\ndef grow(value: str) -> str:\n    return value + '!'\n", "def grow(value: str) -> str:\n    return value\n"),
        ("typescript", "declared_property", "// court-jester-properties bounded\nexport function grow(value: string): string { return value + '!'; }", "export function grow(value: string): string { return value; }"),
        ("python", "inferred_semantic", "def encode(value: str) -> str:\n    return value\ndef decode(value: str) -> str:\n    return value + 'x'\n", "def encode(value: str) -> str:\n    return value\ndef decode(value: str) -> str:\n    return value\n"),
        ("typescript", "inferred_semantic", "export function encode(value: string): string { return value; }\nexport function decode(value: string): string { return value + 'x'; }", "export function encode(value: string): string { return value; }\nexport function decode(value: string): string { return value; }"),
        ("python", "runtime_contract", "def create_counter():\n    calls = 0\n    def push(value: int) -> int:\n        nonlocal calls\n        calls += 1\n        if calls == 2:\n            raise IndexError('second step')\n        return value\n    return {'push': push}\n", "def create_counter():\n    def push(value: int) -> int:\n        return value\n    return {'push': push}\n"),
        ("typescript", "runtime_contract", "export function createCounter() { let calls = 0; function push(value: number): number { calls++; if (calls === 2) throw new ReferenceError('second step'); return value; } return { push }; }", "export function createCounter() { function push(value: number): number { return value; } return { push }; }"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (source, report, id) = fixture(root.path(), language, bug, Some(oracle));
        let bundle = root.path().join("regression");
        let result = export(root.path(), &report, &id, &bundle, true);
        assert!(result.status.success(), "{language}/{oracle}: {}", String::from_utf8_lossy(&result.stderr));
        assert!(!run_test(&bundle, language, env!("CARGO_BIN_EXE_court-jester")).status.success());
        if oracle == "runtime_contract" {
            let missing = if language == "python" { "def create_counter():\n    return {}\n" }
                else { "export function createCounter() { return {}; }" };
            std::fs::write(&source, missing).unwrap();
            assert!(!run_test(&bundle, language, env!("CARGO_BIN_EXE_court-jester")).status.success(), "missing recorded action must not pass");
        }
        std::fs::write(&source, fixed).unwrap();
        let result = run_test(&bundle, language, env!("CARGO_BIN_EXE_court-jester"));
        assert!(result.status.success(), "{language}/{oracle}: {}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    }
}

#[test]
fn relative_replay_source_belongs_to_explicit_project_not_callers_directory() {
    let root = tempfile::tempdir().unwrap();
    let decoy = tempfile::tempdir().unwrap();
    let (_, report, id) = fixture(
        root.path(),
        "python",
        "def first_character(value: str) -> str:\n    return value[0]\n",
        None,
    );
    std::fs::write(
        decoy.path().join("target.py"),
        "def first_character(value: str) -> str:\n    return ''\n",
    )
    .unwrap();
    let mut data: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    data["meta"]["source_file"] = Value::String("target.py".into());
    std::fs::write(&report, serde_json::to_vec(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["replay", "--report"])
        .arg(&report)
        .args(["--finding", &id, "--dependency-project-dir"])
        .arg(root.path())
        .current_dir(decoy.path())
        .output()
        .unwrap();
    let replay: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(replay["outcome"], "reproduced", "{replay}");
    assert_eq!(replay["check_passed"], false);
}
