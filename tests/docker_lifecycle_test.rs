#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn timed_out_docker_wait_does_not_leave_client_descendants() {
    check_delayed_operation("wait", false);
}

#[test]
fn exited_docker_client_with_open_descendant_pipes_is_still_bounded() {
    check_delayed_operation("wait", true);
}

#[test]
fn docker_setup_observation_and_cleanup_commands_are_bounded() {
    for operation in ["image", "create", "start", "inspect", "logs", "rm"] {
        check_delayed_operation(operation, false);
    }
}

#[test]
fn failed_cleanup_and_incomplete_container_state_cannot_pass() {
    for (state, cleanup_fails, passed) in [
        ("{}", false, false),
        (
            r#"{"ExitCode":0,"OOMKilled":false,"Running":true}"#,
            false,
            false,
        ),
        (
            r#"{"ExitCode":0,"OOMKilled":false,"Running":false}"#,
            true,
            false,
        ),
        (
            r#"{"ExitCode":0,"OOMKilled":false,"Running":false}"#,
            false,
            true,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let docker = root.path().join("docker");
        let cleanup = if cleanup_fails {
            "echo 'cleanup denied' >&2; exit 42"
        } else {
            "exit 0"
        };
        std::fs::write(&docker, format!("#!/bin/sh\ncase \"$1\" in\ninspect) echo '{state}' ;;\nrm) {cleanup} ;;\nesac\nexit 0\n")).unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let source = root.path().join("target.py");
        std::fs::write(&source, "print('fixture')\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .args([
                "execute",
                "--language",
                "python",
                "--runtime-profile",
                "isolated",
                "--file",
            ])
            .arg(&source)
            .env("PATH", root.path())
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["exit_code"] == 0, passed, "{report:#}");
        if !passed {
            assert!(
                report["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["impact"] == "blocking"),
                "{report:#}"
            );
        }
        if cleanup_fails {
            assert!(
                report["stderr"]
                    .as_str()
                    .unwrap()
                    .contains("container court-jester"),
                "{report:#}"
            );
        }
    }
}

fn check_delayed_operation(operation: &str, parent_exits: bool) {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("orphan");
    let docker = root.path().join("docker");
    let started = root.path().join("delayed-operation-started");
    let parent = if parent_exits { "exit 0" } else { "wait" };
    std::fs::write(&docker, format!("#!/bin/sh\ncase \"$1\" in\n{operation}) echo started > '{}'; (/bin/sleep 1.5; echo leaked > '{}') & {parent} ;;\ninspect) echo '{{\"ExitCode\":0,\"OOMKilled\":false,\"Running\":false}}' ;;\nesac\nexit 0\n", started.display(), marker.display())).unwrap();
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
    let source = root.path().join("target.py");
    std::fs::write(&source, "print('fixture')\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .args([
            "execute",
            "--language",
            "python",
            "--runtime-profile",
            "isolated",
            "--timeout-seconds",
            "1",
            "--file",
        ])
        .arg(&source)
        .env("PATH", root.path())
        .output()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"));
    assert_eq!(report["timed_out"], true, "{report:#}");
    assert_eq!(report["diagnostics"][0]["kind"], "timeout", "{report:#}");
    assert_eq!(report["diagnostics"][0]["process"], report["termination"]);
    assert!(
        started.exists(),
        "fixture never reached delayed {operation}"
    );
    std::thread::sleep(std::time::Duration::from_millis(1600));
    assert!(
        !marker.exists(),
        "timed-out Docker client left a descendant"
    );
}
