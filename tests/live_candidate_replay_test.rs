use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn run_bundle(bundle: &Path, language: &str) -> Output {
    let mut command = Command::new(if language == "python" {
        "python3"
    } else {
        "node"
    });
    if language == "typescript" {
        command.arg("--test");
    }
    command
        .arg(bundle.join(if language == "python" {
            "test_regression.py"
        } else {
            "regression.test.mjs"
        }))
        .env("COURT_JESTER_BINARY", env!("CARGO_BIN_EXE_court-jester"))
        .output()
        .unwrap()
}

#[test]
fn differential_replay_explicitly_checks_live_candidate_and_imports() {
    for (language, extension, target, original, fixed, broken) in [
        ("python", "py", "from dependency import answer\ndef inspect(value: bool) -> bool:\n    return answer(value)\n",
         "def answer(value):\n    return not value\n", "def answer(value):\n    return value\n", "def answer(value):\n    raise RuntimeError('broken repair')\n"),
        ("typescript", "ts", "import { answer } from './dependency.ts';\nexport function inspect(value: boolean): boolean { return answer(value); }\n",
         "export function answer(value: boolean): boolean { return !value; }\n", "export function answer(value: boolean): boolean { return value; }\n", "export function answer(value: boolean): boolean { throw new Error('broken repair'); }\n"),
    ] {
        let project = tempfile::tempdir().unwrap();
        let baseline = tempfile::tempdir().unwrap();
        let entry = format!("target.{extension}");
        let dependency = format!("dependency.{extension}");
        for root in [project.path(), baseline.path()] {
            std::fs::write(root.join(&entry), target).unwrap();
            if language == "typescript" {
                std::fs::write(root.join("package.json"), "{\"type\":\"module\"}").unwrap();
            }
        }
        std::fs::write(project.path().join(&dependency), original).unwrap();
        std::fs::write(baseline.path().join(&dependency), fixed).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args(["verify", "--language", language, "--file"])
            .arg(project.path().join(&entry)).arg("--project-dir").arg(project.path())
            .arg("--base-file").arg(baseline.path().join(&entry))
            .arg("--base-project-dir").arg(baseline.path())
            .args(["--summary", "repair-json"]).output().unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        let id = report["findings"].as_array().unwrap().iter()
            .find(|finding| finding["category"] == "differential").unwrap_or_else(|| panic!("{report}")).get("id").unwrap().as_str().unwrap();
        let report_path = project.path().join("report.json");
        std::fs::write(&report_path, &output.stdout).unwrap();
        let replay = |live: bool| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_court-jester"));
            command.args(["replay", "--report"]).arg(&report_path).args(["--finding", id]);
            if live { command.arg("--candidate-project-dir").arg(project.path()); }
            let output = command.output().unwrap();
            serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
        };
        assert_eq!(replay(true)["outcome"], "reproduced");
        let bundle = project.path().join("regression");
        let export = |accept: bool, candidate: Option<&Path>| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_court-jester"));
            command.args(["replay", "--report"]).arg(&report_path).args(["--finding", id])
                .arg("--dependency-project-dir").arg(project.path())
                .arg("--export-regression").arg(&bundle);
            if let Some(candidate) = candidate { command.arg("--candidate-project-dir").arg(candidate); }
            if accept { command.arg("--accept-inferred"); }
            command.output().unwrap()
        };
        assert!(!export(false, Some(project.path())).status.success(), "baseline expectations require explicit acceptance");
        let historical = export(true, None);
        assert!(!historical.status.success());
        assert!(String::from_utf8_lossy(&historical.stderr).contains("requires explicit --candidate-project-dir"));
        let mismatched = export(true, Some(baseline.path()));
        assert!(!mismatched.status.success());
        assert!(String::from_utf8_lossy(&mismatched.stderr).contains("must be the same directory"));
        let launch = court_jester::tools::verify::replay_launch_context(report_path.to_str().unwrap(), id).unwrap().unwrap();
        for fault in ["historical", "contradiction", "timeout", "wrong_entry"] {
            let mut evidence = replay(fault != "historical");
            if fault == "historical" { evidence["check_passed"] = Value::Bool(false); }
            if fault == "contradiction" { evidence["check_passed"] = Value::Bool(true); }
            if fault == "timeout" { evidence["execution"]["timed_out"] = Value::Bool(true); }
            if fault == "wrong_entry" {
                let stdout = evidence["execution"]["stdout"].as_str().unwrap();
                let mut payload: Value = serde_json::from_str(stdout.strip_prefix("__COURT_JESTER_REPLAY_JSON__").unwrap().trim()).unwrap();
                payload["candidate_entry"] = Value::String("/wrong/project/target".into());
                evidence["execution"]["stdout"] = Value::String(format!("__COURT_JESTER_REPLAY_JSON__{payload}\n"));
            }
            let destination = project.path().join(format!("refused-{fault}"));
            let plan = court_jester::tools::verify::prepare_regression_export_with_candidate(
                report_path.to_str().unwrap(), id, project.path().to_str().unwrap(),
                destination.to_str().unwrap(), true, project.path().to_str()).unwrap();
            let evidence = serde_json::from_value(evidence).unwrap();
            assert!(court_jester::tools::verify::write_regression_export(plan, &evidence, launch.clone()).is_err(), "{fault}");
            assert!(!destination.exists(), "refused export must not write files");
        }
        assert!(!bundle.exists());
        let exported = export(true, Some(project.path()));
        assert!(exported.status.success(), "{}", String::from_utf8_lossy(&exported.stderr));
        let manifest: Value = serde_json::from_slice(&std::fs::read(bundle.join("regression.json")).unwrap()).unwrap();
        assert_eq!(manifest["replay_mode"], "differential_live");
        assert_eq!(manifest["accepted_inferred"], true);
        let retained: Value = serde_json::from_slice(&std::fs::read(bundle.join("report.json")).unwrap()).unwrap();
        let original_finding = report["findings"].as_array().unwrap().iter().find(|finding| finding["id"] == id).unwrap();
        assert_eq!(retained["stages"][0]["detail"]["findings"][0]["confidence"], original_finding["confidence"]);
        assert_eq!(retained["stages"][0]["detail"]["findings"][0]["oracle"], original_finding["oracle"]);
        assert!(bundle.join("BASELINE.md").is_file());
        assert!(!run_bundle(&bundle, language).status.success(), "export must fail on the original difference");
        std::fs::write(project.path().join(&dependency), fixed).unwrap();
        let repaired = replay(true);
        assert_eq!(repaired["outcome"], "not_reproduced", "{repaired}");
        assert_eq!(repaired["check_passed"], true, "{repaired}");
        let passed = run_bundle(&bundle, language);
        assert!(passed.status.success(), "{}{}", String::from_utf8_lossy(&passed.stdout), String::from_utf8_lossy(&passed.stderr));
        for wrong_mode in ["current_source", "unknown"] {
            let mut changed = manifest.clone();
            changed["replay_mode"] = Value::String(wrong_mode.into());
            std::fs::write(bundle.join("regression.json"), serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(!run_bundle(&bundle, language).status.success(), "{wrong_mode} must not pass a differential live check");
        }
        std::fs::write(bundle.join("regression.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(replay(false)["outcome"], "reproduced", "historical replay must retain the embedded candidate");
        std::fs::write(project.path().join(&dependency), broken).unwrap();
        assert_ne!(replay(true)["check_passed"], true);
        assert!(!run_bundle(&bundle, language).status.success(), "a different exception is not a repair");
        std::fs::write(project.path().join(&entry), if language == "python" {
            "def inspect(value: bool, extra: bool) -> bool:\n    return value\n"
        } else {
            "export function inspect(value: boolean, extra: boolean): boolean { return value; }"
        }).unwrap();
        let incompatible = replay(true);
        assert_eq!(incompatible["outcome"], "inconclusive");
        assert_ne!(incompatible["check_passed"], true);
        std::fs::remove_file(project.path().join(&entry)).unwrap();
        let missing = replay(true);
        assert_eq!(missing["outcome"], "inconclusive");
        assert_ne!(missing["check_passed"], true);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(baseline.path().join(&entry), project.path().join(&entry)).unwrap();
            let escaped = replay(true);
            assert_eq!(escaped["outcome"], "inconclusive");
            assert_ne!(escaped["check_passed"], true);
            std::fs::remove_file(project.path().join(&entry)).unwrap();
        }
        std::fs::write(project.path().join(&entry), target).unwrap();
        std::fs::write(project.path().join(&dependency), fixed).unwrap();
        let relocated = tempfile::tempdir().unwrap();
        let moved = relocated.path().join("moved-project");
        std::fs::rename(project.path(), &moved).unwrap();
        let result = run_bundle(&moved.join("regression"), language);
        assert!(result.status.success(), "relocated live test: {}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        std::fs::write(moved.join(&dependency), original).unwrap();
        assert!(!run_bundle(&moved.join("regression"), language).status.success());
    }
}
