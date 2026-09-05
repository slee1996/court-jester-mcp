use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(args)
        .output()
        .unwrap()
}

fn executable(path: &Path, source: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn replay(report: &Path, root: &Path, id: &str, outcome: &str, check_passed: bool) {
    let output = cli(&[
        "replay",
        "--report",
        report.to_str().unwrap(),
        "--finding",
        id,
        "--dependency-project-dir",
        root.to_str().unwrap(),
    ]);
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(value["outcome"], outcome, "{value:#?}");
    assert_eq!(value["check_passed"], check_passed, "{value:#?}");
    assert_eq!(
        output.status.code(),
        Some(if outcome == "reproduced" { 0 } else { 1 })
    );
}

#[test]
fn native_replay_contract_rebinds_arguments_in_fresh_processes() {
    for (language, extension, engine, source) in [
        ("python", "py", "atheris", "_calls = 0\ndef inspect(*, value: str) -> int:\n    global _calls\n    _calls += 1\n    if _calls == 1 and value:\n        raise ValueError('stable failure')\n    return 0\n"),
        ("typescript", "ts", "jazzer", "let calls = 0; export async function inspect(value: string): Promise<number> { calls++; if (calls === 1 && value) throw new Error('stable failure'); return 0; }"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = root.join(format!("target.{extension}"));
        fs::write(&target, source).unwrap();
        if language == "python" {
            fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeUnicodeNoSurrogates(self, size): return 'long native input'\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'input')\n").unwrap();
        } else {
            executable(&root.join("node_modules/.bin/jazzer"), r#"#!/bin/sh
exec node --experimental-transform-types --input-type=module -e 'import {pathToFileURL} from "node:url"; const target = await import(pathToFileURL(process.argv[1]).href); try { await target.fuzz(new Uint8Array([0,3,97,98,99])); } catch { process.exitCode = 1; }' "$1"
"#);
        }
        let output = cli(&["verify", "--file", target.to_str().unwrap(), "--language", language,
            "--project-dir", root.to_str().unwrap(), "--native-fuzz-engine", engine, "--native-fuzz-runs", "1", "--summary", "repair-json"]);
        let mut report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let finding = report["findings"].as_array().unwrap().iter().find(|finding| finding["classification"] == "native_coverage_guided").unwrap_or_else(|| panic!("{report}")).clone();
        assert_eq!(finding["repro"]["native_replay"]["schema_version"], 1, "{finding}");
        assert_eq!(finding["minimization"]["status"], "preserved", "{finding}");
        assert!(finding["minimization"]["attempts"].as_u64().unwrap() > 1);
        assert_eq!(finding["minimization"]["minimized"]["arguments"][0]["json_value"].as_str().unwrap().chars().count(), 1);
        assert!(finding["minimization"]["original"]["arguments"][0]["json_value"].as_str().unwrap().chars().count() > 1);
        assert_eq!(finding["input_classification"], "unknown");
        assert!(finding["repro"]["input_text"].is_null());
        assert!(finding["minimization"]["original"]["input_text"].as_str().is_some());
        let id = finding["id"].as_str().unwrap();
        report["findings"] = serde_json::json!([finding.clone()]);
        let path = root.join("rebound.json");
        for (value, expected, checked) in [("x", "reproduced", false), ("x", "reproduced", false), ("", "not_reproduced", true)] {
            report["findings"][0]["repro"]["arguments"] = serde_json::json!([{"expression": serde_json::to_string(value).unwrap(), "json_value": value}]);
            // The original display snippet deliberately stays unchanged.
            fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
            replay(&path, root, id, expected, checked);
        }
        for fault in ["version", "body", "arity"] {
            let mut invalid = report.clone();
            if fault == "version" { invalid["findings"][0]["repro"]["native_replay"]["schema_version"] = 99.into(); }
            else if fault == "body" { invalid["findings"][0]["repro"]["native_replay"]["body"] = " ".into(); }
            else { invalid["findings"][0]["repro"]["arguments"] = serde_json::json!([]); }
            fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            let output = cli(&["replay", "--report", path.to_str().unwrap(), "--finding", id, "--dependency-project-dir", root.to_str().unwrap()]);
            assert_eq!(output.status.code(), Some(2));
            assert!(String::from_utf8_lossy(&output.stderr).contains("native replay binding contract"));
        }
    }
}

#[test]
fn native_minimization_retains_original_when_revalidation_or_budget_stops_search() {
    for (condition, expected_status, retains_smaller) in [
        ("'atheris' in sys.modules", "failed", false),
        ("len(value) == 64", "budget_exhausted", false),
        ("len(value) >= 32", "budget_exhausted", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = root.join("target.py");
        fs::write(&target, format!("import sys\ndef inspect(value: str) -> int:\n    if {condition}:\n        raise ValueError('native failure')\n    return 0\n")).unwrap();
        fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeUnicodeNoSurrogates(self, size): return ''.join(chr(33 + n) for n in range(64))\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'input')\n").unwrap();
        let output = cli(&[
            "verify",
            "--file",
            target.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            root.to_str().unwrap(),
            "--native-fuzz-engine",
            "atheris",
            "--native-fuzz-runs",
            "1",
            "--summary",
            "repair-json",
        ]);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let finding = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["classification"] == "native_coverage_guided")
            .unwrap();
        assert_eq!(
            finding["minimization"]["status"], expected_status,
            "{finding}"
        );
        if retains_smaller {
            assert_eq!(
                finding["repro"]["arguments"][0]["json_value"]
                    .as_str()
                    .unwrap()
                    .len(),
                32
            );
            assert_eq!(
                finding["repro"]["arguments"],
                finding["minimization"]["minimized"]["arguments"]
            );
            assert!(finding["repro"]["input_text"].is_null());
        } else {
            assert_eq!(
                finding["repro"]["arguments"],
                finding["minimization"]["original"]["arguments"]
            );
            assert!(finding["minimization"]["minimized"].is_null());
            assert!(finding["repro"]["input_text"].as_str().is_some());
        }
        assert_eq!(
            finding["minimization"]["original"]["arguments"][0]["json_value"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert!(finding["minimization"]["attempts"].as_u64().unwrap() <= 32);
        assert_eq!(finding["input_classification"], "unknown");
    }
}

#[test]
fn native_replay_preserves_mutable_runtime_inputs_and_current_source() {
    native_mutable_input_contract(true);
}

#[test]
fn native_snapshots_preserve_mutable_runtime_inputs() {
    native_mutable_input_contract(false);
}

#[test]
fn admitted_native_findings_export_regressions_for_current_source() {
    for (language, extension, engine, buggy, fixed, runtime, wrapper) in [
        ("python", "py", "atheris", "def inspect(*, value: bool) -> int:\n    raise ValueError('native admitted failure')\n", "def inspect(*, value: bool) -> int:\n    return 0\n", "python3", "test_regression.py"),
        ("typescript", "ts", "jazzer", "export async function inspect(value: boolean): Promise<number> { throw new Error('native admitted failure'); }", "export async function inspect(value: boolean): Promise<number> { return 0; }", "node", "regression.test.mjs"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join(format!("target.{extension}"));
        let reports = root.join("reports");
        let bundle = root.join("regression");
        fs::write(&source, buggy).unwrap();
        if language == "python" {
            fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeBool(self): return True\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'input')\n").unwrap();
        } else {
            executable(&root.join("node_modules/.bin/jazzer"), r#"#!/bin/sh
exec node --experimental-transform-types --input-type=module -e 'import {pathToFileURL} from "node:url"; const target = await import(pathToFileURL(process.argv[1]).href); try { await target.fuzz(new Uint8Array([0,1])); } catch { process.exitCode = 1; }' "$1"
"#);
        }
        let output = cli(&["verify", "--file", source.to_str().unwrap(), "--language", language,
            "--project-dir", root.to_str().unwrap(), "--native-fuzz-engine", engine,
            "--native-fuzz-runs", "1", "--output-dir", reports.to_str().unwrap()]);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let stages = report["stages"].as_array().unwrap();
        let native = stages.iter().find(|stage| stage["name"] == "native_fuzz").unwrap();
        assert_eq!(native["status"], "failed");
        let execute = stages.iter().find(|stage| stage["name"] == "execute").unwrap();
        let finding = execute["detail"]["findings"].as_array().unwrap().iter()
            .find(|finding| finding["classification"] == "native_coverage_guided").unwrap();
        assert_eq!(finding["input_classification"], "valid");
        assert_eq!(finding["minimization"]["status"], "preserved", "{finding}");
        assert_eq!(finding["minimization"]["original"]["arguments"][0]["json_value"], true);
        assert_eq!(finding["repro"]["arguments"][0]["json_value"], false);
        let id = finding["id"].as_str().unwrap();
        let persisted = Path::new(report["report_path"].as_str().unwrap());
        replay(persisted, root, id, "reproduced", false);
        let exported = cli(&["replay", "--report", persisted.to_str().unwrap(), "--finding", id,
            "--dependency-project-dir", root.to_str().unwrap(), "--export-regression", bundle.to_str().unwrap()]);
        assert_eq!(exported.status.code(), Some(0), "{}", String::from_utf8_lossy(&exported.stderr));
        for (implementation, success) in [(buggy, false), (fixed, true)] {
            fs::write(&source, implementation).unwrap();
            let output = Command::new(runtime).arg(bundle.join(wrapper))
                .env("COURT_JESTER_BINARY", env!("CARGO_BIN_EXE_court-jester"))
                .current_dir(root).output().unwrap();
            assert_eq!(output.status.success(), success, "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
    }
}

#[test]
fn native_typescript_replay_preserves_primitive_throw_identity() {
    for (thrown, different, supported) in [
        ("NaN", "null", true),
        ("-0", "0", true),
        ("undefined", "null", true),
        ("null", "undefined", true),
        ("8n", "9n", true),
        ("Symbol('failure')", "null", false),
        ("({message:'failure'})", "null", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("target.ts");
        let reports = root.join("reports");
        executable(
            &root.join("node_modules/.bin/jazzer"),
            r#"#!/bin/sh
exec node --experimental-transform-types --input-type=module -e 'import {pathToFileURL} from "node:url"; const target = await import(pathToFileURL(process.argv[1]).href); try { await target.fuzz(new Uint8Array([0,0])); } catch { process.exitCode = 1; }' "$1"
"#,
        );
        let implementation = |value: &str| {
            format!(
                "export async function inspect(value: Date): Promise<number> {{ throw {value}; }}"
            )
        };
        fs::write(&source, implementation(thrown)).unwrap();
        let output = cli(&[
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "typescript",
            "--project-dir",
            root.to_str().unwrap(),
            "--native-fuzz-engine",
            "jazzer",
            "--native-fuzz-runs",
            "1",
            "--output-dir",
            reports.to_str().unwrap(),
        ]);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let execute = report["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["name"] == "execute")
            .unwrap();
        let finding = execute["detail"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["classification"] == "native_coverage_guided")
            .unwrap_or_else(|| panic!("missing native {thrown}: {report:#?}"));
        let id = finding["id"].as_str().unwrap();
        let persisted = Path::new(report["report_path"].as_str().unwrap());
        if supported {
            replay(persisted, root, id, "reproduced", false);
            fs::write(&source, implementation(different)).unwrap();
            replay(persisted, root, id, "not_reproduced", false);
            fs::write(
                &source,
                "export async function inspect(value: Date): Promise<number> { return 0; }",
            )
            .unwrap();
            replay(persisted, root, id, "not_reproduced", true);
        } else {
            let output = cli(&[
                "replay",
                "--report",
                persisted.to_str().unwrap(),
                "--finding",
                id,
                "--dependency-project-dir",
                root.to_str().unwrap(),
            ]);
            let replay: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(output.status.code(), Some(3));
            assert_eq!(replay["outcome"], "inconclusive");
            assert!(replay["check_passed"].is_null());
        }
    }
}

fn native_mutable_input_contract(check_replay: bool) {
    for (language, extension, engine, bug, other, fixed, expression_fragments) in [
        ("python", "py", "atheris",
         "def inspect(*, value: bytearray) -> int:\n    if not isinstance(value, bytearray):\n        raise TypeError('wrong input shape')\n    if value == bytearray(b'\\x00\\x80\\xff'):\n        value.clear()\n        raise ValueError('native failure: original')\n    return 0\n",
         "def inspect(*, value: bytearray) -> int:\n    raise ValueError('native failure: different')\n",
         "def inspect(*, value: bytearray) -> int:\n    return 0\n",
         vec!["bytearray", "\\x80", "\\xff"]),
        ("typescript", "ts", "jazzer",
         "export async function inspect(value: Uint8Array, date: Date, token: bigint): Promise<number> { if (!(value instanceof Uint8Array) || !(date instanceof Date) || typeof token !== 'bigint') throw new Error('wrong input shape'); if (value[0] === 0 && value[1] === 128 && date.getTime() === 7 && token === 8n) { value[0] = 99; throw new Error('native failure: original'); } return 0; }",
         "export async function inspect(value: Uint8Array, date: Date, token: bigint): Promise<number> { throw new Error('native failure: different'); }",
         "export async function inspect(value: Uint8Array, date: Date, token: bigint): Promise<number> { return 0; }",
         vec!["Uint8Array", "128", "255"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join(format!("target.{extension}"));
        let reports = root.join("reports");
        fs::write(&source, bug).unwrap();
        if language == "python" {
            fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeBytes(self, count): return b'\\x00\\x80\\xff'\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'original native bytes')\n").unwrap();
        } else {
            executable(&root.join("node_modules/.bin/jazzer"), r#"#!/bin/sh
exec node --experimental-transform-types --input-type=module -e 'import {pathToFileURL} from "node:url"; const target = await import(pathToFileURL(process.argv[1]).href); try { await target.fuzz(new Uint8Array([0,3,0,128,255,128,0,0,0,0,7,128,0,0,0,0,8])); } catch { process.exitCode = 1; }' "$1"
"#);
        }
        let output = cli(&["verify", "--file", source.to_str().unwrap(), "--language", language,
            "--project-dir", root.to_str().unwrap(), "--native-fuzz-engine", engine,
            "--native-fuzz-runs", "1", "--output-dir", reports.to_str().unwrap()]);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let execute = report["stages"].as_array().unwrap().iter().find(|stage| stage["name"] == "execute").unwrap();
        let finding = execute["detail"]["findings"].as_array().unwrap().iter().find(|finding|
            finding["classification"] == "native_coverage_guided"
            && finding["message"] == "native failure: original"
        ).unwrap_or_else(|| panic!("native observation missing: {report:#?}"));
        assert_eq!(finding["input_classification"], "unknown");
        let expression = finding["minimization"]["original"]["arguments"][0]["expression"].as_str().unwrap();
        for fragment in expression_fragments {
            assert!(expression.contains(fragment), "lost original runtime input {fragment}: {expression}");
        }
        if !check_replay {
            assert!(finding["minimization"]["original"]["arguments"][0]["json_value"].is_null());
            if language == "typescript" {
                let arguments = &finding["minimization"]["original"]["arguments"];
                assert_eq!(arguments[1]["expression"], "new Date(7)");
                assert_eq!(arguments[2]["expression"], "8n");
                assert!(arguments[1]["json_value"].is_null());
                assert!(arguments[2]["json_value"].is_null());
            }
            continue;
        }
        let id = finding["id"].as_str().unwrap();
        let persisted = Path::new(report["report_path"].as_str().unwrap());
        replay(persisted, root, id, "reproduced", false);
        fs::write(&source, other).unwrap();
        replay(persisted, root, id, "not_reproduced", false);
        fs::write(&source, fixed).unwrap();
        replay(persisted, root, id, "not_reproduced", true);
    }
}
