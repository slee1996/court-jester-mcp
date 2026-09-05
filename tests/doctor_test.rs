#![cfg(unix)]

use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn executable(path: &Path, source: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn doctor(root: &Path, extra: &[&str]) -> (std::process::Output, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--language", "python", "--project-dir"])
        .arg(root)
        .args(extra)
        .env_remove("VIRTUAL_ENV")
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("stdout: {:?}; stderr: {:?}", output.stdout, output.stderr));
    (output, report)
}

fn check<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == name)
        .unwrap()
}

#[test]
fn doctor_uses_project_tools_without_running_target() {
    let root = tempfile::tempdir().unwrap();
    let python = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = String::from_utf8(python.stdout).unwrap();
    let local_python = root.path().join(".venv/bin/python3");
    std::fs::create_dir_all(local_python.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(python.trim(), &local_python).unwrap();
    executable(
        &root.path().join(".venv/bin/ruff"),
        "#!/bin/sh\nprintf 'ruff project-test\\n'\n",
    );
    let target = root.path().join("target.py");
    std::fs::write(
        &target,
        "raise RuntimeError('doctor must not import target')\n",
    )
    .unwrap();
    let (output, report) = doctor(root.path(), &["--file", target.to_str().unwrap()]);
    assert!(output.status.success(), "{report}");
    assert_eq!(check(&report, "runtime")["status"], "passed");
    let expected = root
        .path()
        .canonicalize()
        .unwrap()
        .join(".venv/bin/python3");
    assert_eq!(
        check(&report, "runtime")["detail"]["executable"],
        expected.to_str().unwrap()
    );
    assert_eq!(
        check(&report, "linter")["detail"]["version"],
        "ruff project-test"
    );
}

#[test]
fn doctor_does_not_pass_a_broken_project_linter() {
    let root = tempfile::tempdir().unwrap();
    executable(
        &root.path().join(".venv/bin/ruff"),
        "#!/bin/sh\necho broken-tool >&2\nexit 7\n",
    );
    let (_, report) = doctor(root.path(), &[]);
    assert_eq!(check(&report, "linter")["status"], "advisory");
    assert!(check(&report, "linter")["message"]
        .as_str()
        .unwrap()
        .contains("broken-tool"));
}

#[test]
fn doctor_rejects_missing_project() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--project-dir"])
        .arg(root.path().join("missing"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a directory"));
}

#[test]
fn doctor_preserves_project_runtime_failure_and_timeout() {
    for script in [
        "#!/bin/sh\necho broken-runtime >&2\nexit 9\n",
        "#!/bin/sh\nexec /bin/sleep 10\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        executable(&root.path().join(".venv/bin/python3"), script);
        executable(
            &root.path().join(".venv/bin/ruff"),
            "#!/bin/sh\necho ready\n",
        );
        let (output, report) = doctor(root.path(), &["--timeout-seconds", "0.2"]);
        assert_eq!(output.status.code(), Some(1), "{report}");
        let runtime = check(&report, "runtime");
        assert_eq!(runtime["status"], "failed");
        if script.contains("sleep") {
            assert_eq!(runtime["detail"]["execution"]["timed_out"], true);
        } else {
            assert_eq!(runtime["detail"]["execution"]["exit_code"], 9);
            assert!(runtime["detail"]["execution"]["stderr"]
                .as_str()
                .unwrap()
                .contains("broken-runtime"));
        }
    }
}

#[test]
fn doctor_requires_successful_runtime_evidence_not_just_exit_zero() {
    for (version, exit, expected) in [
        ("24.0.0", 0, "passed"),
        ("23.0.0", 0, "failed"),
        ("unknown", 0, "failed"),
        ("24.0.0", 1, "failed"),
    ] {
        let root = tempfile::tempdir().unwrap();
        executable(&root.path().join("node_modules/.bin/node"), &format!(
            "#!/bin/sh\nprintf '%s\\n' '__COURT_JESTER_DOCTOR__{{\"version\":\"{version}\",\"executable\":\"test-node\"}}'\nexit {exit}\n"
        ));
        executable(
            &root.path().join("node_modules/.bin/biome"),
            "#!/bin/sh\necho biome-project\n",
        );
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args(["doctor", "--language", "typescript", "--project-dir"])
            .arg(root.path())
            .env_remove("VIRTUAL_ENV")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(check(&report, "runtime")["status"], expected, "{report}");
        assert_eq!(
            check(&report, "linter")["detail"]["version"],
            "biome-project"
        );
    }
}

#[test]
fn doctor_linter_requires_nonempty_evidence_and_is_bounded() {
    for script in ["#!/bin/sh\nexit 0\n", "#!/bin/sh\nexec /bin/sleep 10\n"] {
        let root = tempfile::tempdir().unwrap();
        executable(&root.path().join(".venv/bin/ruff"), script);
        let started = std::time::Instant::now();
        let (_, report) = doctor(root.path(), &["--timeout-seconds", "0.2"]);
        assert!(started.elapsed().as_secs() < 5);
        assert_eq!(check(&report, "linter")["status"], "advisory");
    }
}

#[test]
fn doctor_timeout_terminates_linter_descendants() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("orphan-was-here");
    executable(
        &root.path().join(".venv/bin/ruff"),
        &format!(
            "#!/bin/sh\n(/bin/sleep 1; echo orphan > '{}') &\nwait\n",
            marker.display()
        ),
    );
    let (_, report) = doctor(root.path(), &["--timeout-seconds", "0.2"]);
    assert_eq!(check(&report, "linter")["status"], "advisory");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        !marker.exists(),
        "timed-out readiness probe left a live descendant"
    );
}

#[test]
fn doctor_does_not_claim_project_readiness_for_isolated_profile() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--runtime-profile", "isolated", "--project-dir"])
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("image readiness only"));
}

#[test]
fn doctor_file_requires_one_language_and_can_autodetect_project() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.py");
    std::fs::write(&target, "raise RuntimeError('must not execute')\n").unwrap();
    executable(
        &root.path().join(".venv/bin/ruff"),
        "#!/bin/sh\necho autodetected-ruff\n",
    );
    for language in ["all", "ALL"] {
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args(["doctor", "--language", language, "--file"])
            .arg(&target)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--language", "python", "--file"])
        .arg(&target)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        check(&report, "linter")["detail"]["version"],
        "autodetected-ruff"
    );
}

#[test]
fn doctor_tsx_uses_the_project_tsx_runner() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.tsx");
    std::fs::write(&target, "throw new Error('must not execute');\n").unwrap();
    executable(
        &root.path().join("node_modules/.bin/node"),
        "#!/bin/sh\nexit 9\n",
    );
    executable(&root.path().join("node_modules/.bin/tsx"),
        "#!/bin/sh\nprintf '%s\\n' '__COURT_JESTER_DOCTOR__{\"version\":\"24.0.0\",\"executable\":\"tsx-node\"}'\n");
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args(["doctor", "--language", "typescript", "--file"])
        .arg(&target)
        .env_remove("VIRTUAL_ENV")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(check(&report, "runtime")["status"], "passed", "{report}");
    assert_eq!(check(&report, "runtime")["detail"]["source_mode"], "tsx");
}
