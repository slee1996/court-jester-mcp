use serde_json::Value;
use std::process::Command;

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
        std::fs::write(project.path().join(&dependency), fixed).unwrap();
        let repaired = replay(true);
        assert_eq!(repaired["outcome"], "not_reproduced", "{repaired}");
        assert_eq!(repaired["check_passed"], true, "{repaired}");
        assert_eq!(replay(false)["outcome"], "reproduced", "historical replay must retain the embedded candidate");
        std::fs::write(project.path().join(&dependency), broken).unwrap();
        assert_ne!(replay(true)["check_passed"], true);
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
        }
    }
}
