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
fn doctor_entrypoint_probe_is_opt_in_and_preserves_test_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let target = root.join("target.py");
    let tests = root.join("checks.py");
    let marker = root.join("entrypoint-ran");
    std::fs::write(&target, "def inspect(value: bool):\n    return value\n").unwrap();
    std::fs::write(&tests, format!("from pathlib import Path\nfrom target import inspect\nPath({:?}).touch()\nassert inspect(False) is False\n", marker.to_str().unwrap())).unwrap();
    std::fs::write(root.join(".court-jester.json"), serde_json::json!({"schema_version":1,"defaults":{"memory_mb":256},"targets":[{"source":"target.py","test_files":["checks.py"]}]}).to_string()).unwrap();
    let (_, report) = doctor(root, &["--file", target.to_str().unwrap()]);
    assert!(!marker.exists());
    assert!(report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|check| check["name"] != "entrypoint_probe"));
    let (output, report) = doctor(
        root,
        &["--file", target.to_str().unwrap(), "--probe-entrypoint"],
    );
    assert!(output.status.success(), "{report:#}");
    assert!(marker.exists());
    let probe = check(&report, "entrypoint_probe");
    assert_eq!(probe["status"], "passed", "{probe:#}");
    assert_eq!(probe["detail"]["coverage_checked"], false);
    assert_eq!(probe["detail"]["fuzzing_started"], false);
    assert_eq!(probe["detail"]["memory_mb"], 256);
    for code in [
        "import missing_doctor_probe_dependency\n",
        "def inspect(value: bool):\n    return True\n",
    ] {
        std::fs::write(&target, code).unwrap();
        let (output, report) = doctor(
            root,
            &["--file", target.to_str().unwrap(), "--probe-entrypoint"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(check(&report, "entrypoint_probe")["status"], "failed");
    }
    std::fs::write(&target, "def inspect(value: bool):\n    return value\n").unwrap();
    std::fs::write(&tests, "import time\ntime.sleep(3)\n").unwrap();
    let (output, report) = doctor(
        root,
        &[
            "--file",
            target.to_str().unwrap(),
            "--probe-entrypoint",
            "--timeout-seconds",
            "0.2",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let probe = check(&report, "entrypoint_probe");
    assert_eq!(
        probe["detail"]["test_stage"]["detail"]["timed_out"], true,
        "{probe:#}"
    );
}

#[test]
fn doctor_probe_requires_an_unambiguous_entrypoint_and_inspection_never_executes_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let target = root.join("target.py");
    std::fs::write(&target, "raise RuntimeError('must not run')\n").unwrap();
    for args in [
        vec!["doctor", "--probe-entrypoint"],
        vec![
            "doctor",
            "--probe-entrypoint",
            "--file",
            target.to_str().unwrap(),
            "--language",
            "python",
        ],
        vec!["verify", "--probe-entrypoint", "--show-config"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--probe-entrypoint"));
    }
    let (output, config) = doctor(
        root,
        &[
            "--file",
            target.to_str().unwrap(),
            "--probe-entrypoint",
            "--show-config",
        ],
    );
    assert!(output.status.success());
    assert_eq!(config["execution_started"], false);
}

#[test]
fn doctor_typescript_probe_uses_the_configured_node_test_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("target.ts");
    let tests = root.join("checks.test.ts");
    std::fs::write(&tests, "import { test } from 'node:test';\nimport assert from 'node:assert/strict';\nimport { inspect } from './target.ts';\ntest('entrypoint', () => assert.equal(inspect(false), false));\n").unwrap();
    for (code, expected) in [
        (
            "export function inspect(value: boolean): boolean { return value; }",
            "passed",
        ),
        (
            "export function inspect(value: boolean): boolean { return true; }",
            "failed",
        ),
    ] {
        std::fs::write(&source, code).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args([
                "doctor",
                "--language",
                "typescript",
                "--project-dir",
                root.to_str().unwrap(),
                "--file",
                source.to_str().unwrap(),
                "--test-file",
                tests.to_str().unwrap(),
                "--test-runner",
                "node",
                "--probe-entrypoint",
            ])
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let probe = check(&report, "entrypoint_probe");
        assert_eq!(probe["status"], expected, "{probe:#}");
        assert_eq!(
            probe["detail"]["test_stage"]["detail"]["test_runner_selected"],
            "node"
        );
    }
}

#[test]
fn doctor_probe_does_not_accept_zero_exit_without_loading_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = root.join("target.py");
    let tests = root.join("checks.py");
    std::fs::write(&source, "def inspect(value: bool):\n    return value\n").unwrap();
    std::fs::write(
        &tests,
        "import target\nassert target.inspect(False) is False\n",
    )
    .unwrap();
    executable(&root.join(".venv/bin/python3"), "#!/bin/sh\necho '__COURT_JESTER_DOCTOR__{\"version\":\"3.12.0\",\"executable\":\"fake-python\"}'\nexit 0\n");
    let (_, report) = doctor(
        root,
        &[
            "--file",
            source.to_str().unwrap(),
            "--test-file",
            tests.to_str().unwrap(),
            "--probe-entrypoint",
        ],
    );
    assert_eq!(check(&report, "runtime")["status"], "passed");
    assert_eq!(check(&report, "entrypoint_probe")["status"], "failed");
}

#[test]
fn doctor_resolves_configured_tests_without_executing_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let target = root.join("target.py");
    std::fs::write(&target, "raise RuntimeError('must not import target')\n").unwrap();
    std::fs::write(
        root.join(".court-jester.json"),
        serde_json::json!({
            "schema_version": 1, "defaults": {"memory_mb": 256},
            "targets": [{"source": "target.py", "test_files": ["checks.py"]}]
        })
        .to_string(),
    )
    .unwrap();
    let tests = root.join("checks.py");
    std::fs::write(&tests, "from pathlib import Path\nPath('unexpected-doctor-test').write_text('ran')\nraise RuntimeError('must not execute tests')\n").unwrap();
    let (_, report) = doctor(root, &["--file", target.to_str().unwrap()]);
    assert_eq!(
        check(&report, "repository_config")["detail"]["limits"]["memory_mb"],
        256
    );
    assert_eq!(check(&report, "configured_entrypoints")["status"], "passed");
    assert_eq!(
        check(&report, "configured_entrypoints")["detail"]["executed"],
        false
    );
    assert!(!root.join("unexpected-doctor-test").exists());
    std::fs::remove_file(&tests).unwrap();
    let (output, report) = doctor(root, &["--file", target.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(check(&report, "configured_entrypoints")["status"], "failed");
    assert!(check(&report, "configured_entrypoints")["message"]
        .as_str()
        .unwrap()
        .contains("test_files"));
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
