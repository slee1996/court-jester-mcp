use serde_json::Value;
use std::fs;
use std::process::Command;

#[test]
fn native_admission_uses_closed_parameter_contracts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("target.py");
    fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeBool(self): return False\n    def ConsumeUnicodeNoSurrogates(self, count): return 'input'\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'input')\n").unwrap();
    let suppression = root.join("suppressions.json");
    fs::write(
        &suppression,
        r#"{"rules":[{"stage":"execute","function":"inspect","reason":"native_coverage_guided"}]}"#,
    )
    .unwrap();
    for (annotation, expected_classification, expected_stage, suppressed) in [
        ("bool", "valid", "failed", false),
        ("str", "unknown", "inconclusive", false),
        ("bool", "valid", "advisory", true),
        ("str", "unknown", "advisory", true),
    ] {
        fs::write(&source, format!("def inspect(*, value: {annotation}) -> int:\n    raise ValueError('native contract failure')\n")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_court-jester"));
        command.args([
            "verify",
            "--file",
            source.to_str().unwrap(),
            "--language",
            "python",
            "--project-dir",
            root.to_str().unwrap(),
            "--native-fuzz-engine",
            "atheris",
            "--native-fuzz-runs",
            "1",
        ]);
        if suppressed {
            command.args(["--suppressions-file", suppression.to_str().unwrap()]);
        }
        let output = command.output().unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let native = report["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["name"] == "native_fuzz")
            .unwrap();
        assert_eq!(
            native["detail"]["native_findings"][0]["input_classification"],
            expected_classification
        );
        assert_eq!(native["status"], expected_stage);
        if suppressed {
            assert_eq!(native["detail"]["gating_finding_count"], 0);
            assert_eq!(native["detail"]["unknown_finding_count"], 0);
            assert_eq!(native["detail"]["native_findings"][0]["suppressed"], true);
        }
    }
}

#[test]
fn native_decoder_runs_without_ordinary_generated_cases() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("target.py");
    fs::write(&source, "def inspect(*, value: bytearray) -> int:\n    raise ValueError('native-only observation')\n").unwrap();
    fs::write(root.join("atheris.py"), "class FuzzedDataProvider:\n    def __init__(self, data): pass\n    def ConsumeIntInRange(self, lower, upper): return lower\n    def ConsumeBytes(self, count): return b'abc'\ndef instrument_all(): pass\ndef Setup(argv, callback):\n    global _callback\n    _callback = callback\ndef Fuzz(): _callback(b'input')\n").unwrap();
    for engine in ["off", "atheris"] {
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args([
                "verify",
                "--file",
                source.to_str().unwrap(),
                "--language",
                "python",
                "--project-dir",
                root.to_str().unwrap(),
                "--native-fuzz-engine",
                engine,
                "--native-fuzz-runs",
                "1",
            ])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let stages = report["stages"].as_array().unwrap();
        let native = stages.iter().find(|stage| stage["name"] == "native_fuzz");
        let execute = stages
            .iter()
            .find(|stage| stage["name"] == "execute")
            .unwrap();
        if engine == "off" {
            assert!(native.is_none());
            assert_eq!(execute["status"], "skipped");
        } else {
            assert_eq!(native.unwrap()["status"], "inconclusive");
            assert_eq!(execute["detail"]["generated_campaign_ran"], false);
            assert_eq!(execute["detail"]["execution"]["exit_code"], Value::Null);
            assert_eq!(execute["detail"]["valid_invocations"], 0);
            let finding = execute["detail"]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|finding| finding["message"] == "native-only observation")
                .unwrap();
            assert_eq!(finding["input_classification"], "unknown");
            assert_eq!(report["verdict"], "inconclusive");
        }
    }
}
