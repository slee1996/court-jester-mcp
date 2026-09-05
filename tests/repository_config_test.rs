use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};

#[test]
fn ci_validates_configured_suppressions_even_without_changed_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for args in [
        vec!["init", "--quiet"],
        vec![
            "-c",
            "user.name=Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "base",
        ],
    ] {
        assert!(Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    fs::write(
        root.join(".court-jester.json"),
        json!({"schema_version": 1, "defaults": {"suppressions_file": "suppression.json"}})
            .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("target.py"),
        "raise RuntimeError('must not execute')\n",
    )
    .unwrap();
    for contents in [
        None,
        Some("not json"),
        Some(r#"{"rules":null}"#),
        Some(r#"{"rule":[]}"#),
        Some(r#"{"rules":[{"functoin":"inspect"}]}"#),
        Some(r#"{"rules":[{"severity":"typo"}]}"#),
    ] {
        if let Some(contents) = contents {
            fs::write(root.join("suppression.json"), contents).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .current_dir(root)
            .args(["ci", "--base", "HEAD", "--report", "json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("suppression.json"));
        let direct = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .current_dir(root)
            .args(["verify", "--file", "target.py", "--language", "python"])
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&direct.stderr).contains("suppression.json"));
    }
}

fn run(root: &std::path::Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(root)
        .args([
            "verify",
            "--repo-config",
            "project/settings.json",
            "--file",
            "project/target.py",
            "--language",
            "python",
            "--summary",
            "repair-json",
        ])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn show_config_reports_selected_limits_without_executing_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(root.join("project/target.py"), "from pathlib import Path\nPath('unexpected-execution').write_text('ran')\nraise RuntimeError('must not execute')\n").unwrap();
    fs::write(
        root.join("project/settings.json"),
        json!({"schema_version": 1, "defaults": {"timeout_seconds": 3, "memory_mb": 256}})
            .to_string(),
    )
    .unwrap();
    let output = run(root, &["--show-config", "--timeout-seconds", "5"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(config["command"], "verify");
    assert_eq!(config["execution_started"], false);
    assert_eq!(config["limits"]["memory_mb"], 256);
    assert_eq!(config["limits"]["timeouts"]["python_seconds"], 5.0);
    assert_eq!(config["limits"]["timeouts"]["typescript_seconds"], 5.0);
    assert_eq!(config["limits"]["timeouts"]["test_seconds"], 5.0);
    assert!(config["config_path"]
        .as_str()
        .unwrap()
        .ends_with("project/settings.json"));
    assert!(!root.join("unexpected-execution").exists());
    assert!(!root.join("project/unexpected-execution").exists());
    fs::write(
        root.join("project/settings.json"),
        json!({"schema_version": 1}).to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(root)
        .args([
            "verify",
            "--repo-config",
            "project/settings.json",
            "--show-config",
        ])
        .env("COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS", "7")
        .env("COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS", "8")
        .env("COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS", "9")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(config["limits"]["timeouts"]["python_seconds"], 7.0);
    assert_eq!(config["limits"]["timeouts"]["typescript_seconds"], 8.0);
    assert_eq!(config["limits"]["timeouts"]["test_seconds"], 9.0);
    assert_eq!(config["readiness_checked"], false);
}

#[test]
fn ci_uses_configured_authoritative_tests_without_requiring_mutation_testing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "tests@example.com"]);
    git(&["config", "user.name", "Court Jester Tests"]);
    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    return True\n",
    )
    .unwrap();
    fs::write(
        root.join("checks.py"),
        "from target import inspect\nassert inspect(False) is False\n",
    )
    .unwrap();
    fs::write(
        root.join(".court-jester.json"),
        json!({"schema_version": 1, "defaults": {"test_files": ["checks.py"], "memory_mb": 256}})
            .to_string(),
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    git(&["add", "target.py"]);
    git(&["commit", "--quiet", "-m", "candidate"]);
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(root)
        .args(["ci", "--base", &base, "--report", "json"])
        .output()
        .unwrap();
    assert_ne!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["base_commit"], base);
    assert_eq!(report["head_commit"], git(&["rev-parse", "HEAD"]));
    assert_eq!(report["candidate_state"], "working_tree");
    let files = report["files"].as_array().unwrap();
    let target = files
        .iter()
        .find(|file| file["file"] == "target.py")
        .unwrap();
    let stages = target["report"]["stages"].as_array().unwrap();
    let test = stages.iter().find(|stage| stage["name"] == "test").unwrap();
    assert_eq!(test["status"], "passed", "{test:#?}");
    assert!(stages.iter().all(|stage| stage["name"] != "test_quality"));

    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    if value:\n        return True\n    return False\n",
    )
    .unwrap();
    git(&["add", "target.py"]);
    git(&["commit", "--quiet", "-m", "branching candidate"]);
    fs::write(
        root.join("suppression.json"),
        json!({"rules": [{"path": "target.py", "stage": "complexity", "function": "inspect"}]})
            .to_string(),
    )
    .unwrap();
    fs::write(
        root.join(".court-jester.json"),
        json!({"schema_version": 1, "defaults": {"suppressions_file": "suppression.json"}})
            .to_string(),
    )
    .unwrap();
    for (extra, expected) in [(vec![], "passed"), (vec!["--no-repo-config"], "failed")] {
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .current_dir(root)
            .args([
                "ci",
                "--base",
                &base,
                "--report",
                "json",
                "--gate",
                "complexity",
                "--complexity-threshold",
                "1",
            ])
            .args(extra)
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let stage = report["files"][0]["report"]["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["name"] == "complexity")
            .unwrap();
        assert_eq!(stage["status"], expected, "{stage:#}");
        if expected == "passed" {
            assert_eq!(
                stage["detail"]["suppressed_violations"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert!(stage["detail"]["suppression_source"]
                .as_str()
                .unwrap()
                .ends_with("suppression.json"));
        }
    }
}

#[test]
fn ci_routes_same_language_targets_to_their_own_tests() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "tests@example.com"]);
    git(&["config", "user.name", "Court Jester Tests"]);
    for name in ["first", "second"] {
        fs::write(
            root.join(format!("{name}.py")),
            "def inspect(value: bool):\n    return True\n",
        )
        .unwrap();
        fs::write(
            root.join(format!("check_{name}.py")),
            format!("from {name} import inspect\nassert inspect(False) is False\n"),
        )
        .unwrap();
    }
    fs::write(
        root.join(".court-jester.json"),
        json!({"schema_version": 1, "targets": [
            {"source": "first.py", "test_files": ["check_first.py"]},
            {"source": "second.py", "test_files": ["check_second.py"]}
        ]})
        .to_string(),
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    for name in ["first", "second"] {
        fs::write(
            root.join(format!("{name}.py")),
            "def inspect(value: bool):\n    return value\n",
        )
        .unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "candidate"]);
    for mutation in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_court-jester"));
        command
            .current_dir(root)
            .args(["ci", "--base", &base, "--report", "json"]);
        if mutation {
            command.args(["--test-quality", "2"]);
        }
        let output = command.output().unwrap();
        let report: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        for name in ["first.py", "second.py"] {
            let file = report["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|file| file["file"] == name)
                .unwrap();
            let stages = file["report"]["stages"].as_array().unwrap();
            assert_eq!(
                stages.iter().find(|stage| stage["name"] == "test").unwrap()["status"],
                "passed",
                "{file:#?}"
            );
            assert_eq!(
                stages.iter().any(|stage| stage["name"] == "test_quality"),
                mutation
            );
        }
    }
}

#[test]
fn source_mapping_overrides_default_tests_but_explicit_tests_win() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(
        root.join("project/target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    fs::write(
        root.join("project/default.py"),
        "from target import inspect\ninspect(False)\nassert False, 'default test'\n",
    )
    .unwrap();
    fs::write(
        root.join("project/mapped.py"),
        "from target import inspect\nassert inspect(False) is False\n",
    )
    .unwrap();
    fs::write(
        root.join("project/settings.json"),
        json!({"schema_version": 1,
            "defaults": {"test_files": ["default.py"]},
            "targets": [{"source": "target.py", "test_files": ["mapped.py"]}]
        })
        .to_string(),
    )
    .unwrap();
    let mapped = run(root, &["--tests-only"]);
    assert_eq!(
        mapped.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&mapped.stdout),
        String::from_utf8_lossy(&mapped.stderr)
    );
    let explicit = run(root, &["--tests-only", "--test-file", "project/default.py"]);
    assert_eq!(explicit.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&explicit.stdout).contains("default test"));
}

#[test]
fn automatic_config_uses_nearest_repo_config_and_honors_explicit_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path();
    let repo = parent.join("repo");
    fs::create_dir_all(repo.join("nested")).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(parent.join(".court-jester.json"), "invalid parent config").unwrap();
    fs::write(
        repo.join(".court-jester.json"),
        json!({"schema_version": 1, "defaults": {"timeout_seconds": 3}}).to_string(),
    )
    .unwrap();
    fs::write(
        repo.join("nested/.court-jester.json"),
        json!({"schema_version": 1, "defaults": {"timeout_seconds": 4}}).to_string(),
    )
    .unwrap();
    fs::write(
        repo.join("nested/target.py"),
        "def inspect(value: bool):\n    raise ValueError('config discovery')\n",
    )
    .unwrap();
    for (extra, timeout) in [
        (vec![], 4.0),
        (vec!["--project-dir", "."], 3.0),
        (vec!["--timeout-seconds", "5"], 5.0),
        (vec!["--no-repo-config", "--timeout-seconds", "6"], 6.0),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .current_dir(&repo)
            .args([
                "verify",
                "--file",
                "nested/target.py",
                "--language",
                "python",
                "--summary",
                "repair-json",
            ])
            .args(extra)
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        let finding = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["message"] == "config discovery")
            .unwrap();
        assert_eq!(
            finding["launch_context"]["limits"]["timeout_seconds"],
            timeout
        );
    }
    fs::remove_file(repo.join("nested/.court-jester.json")).unwrap();
    fs::remove_file(repo.join(".court-jester.json")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(&repo)
        .args([
            "verify",
            "--file",
            "nested/target.py",
            "--language",
            "python",
        ])
        .output()
        .unwrap();
    assert_ne!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn malformed_discovered_config_can_be_bypassed_without_loading_parent_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(
        root.join(".court-jester.json"),
        "invalid parent outside repository",
    )
    .unwrap();
    fs::write(
        root.join("project/target.py"),
        "def inspect(value: bool):\n    raise ValueError('expected observation')\n",
    )
    .unwrap();
    let config = root.join("project/.court-jester.json");
    let invoke = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_court-jester"))
            .current_dir(root)
            .args([
                "verify",
                "--file",
                "project/target.py",
                "--language",
                "python",
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    // Without a Git root, discovery checks only the target directory.
    assert_ne!(invoke(&[]).status.code(), Some(2));
    fs::write(&config, "invalid discovered config").unwrap();
    let malformed = invoke(&[]);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("invalid repository config"));
    assert_ne!(invoke(&["--no-repo-config"]).status.code(), Some(2));
    let conflict = invoke(&[
        "--no-repo-config",
        "--repo-config",
        "project/.court-jester.json",
    ]);
    assert_eq!(conflict.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflicts"));
}

#[test]
fn repository_defaults_reach_execution_and_explicit_flags_win() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(
        root.join("project/target.py"),
        "def inspect(value: bool) -> int:\n    raise ValueError('declared-domain failure')\n",
    )
    .unwrap();
    fs::write(
        root.join("project/settings.json"),
        json!({"schema_version": 1, "defaults": {"timeout_seconds": 3, "memory_mb": 256}})
            .to_string(),
    )
    .unwrap();
    for (extra, timeout) in [
        (Vec::<&str>::new(), 3.0),
        (vec!["--timeout-seconds", "5"], 5.0),
    ] {
        let output = run(root, &extra);
        let report: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        let findings = report["findings"].as_array().unwrap();
        let finding = findings
            .iter()
            .find(|finding| finding["message"] == "declared-domain failure")
            .unwrap();
        assert_eq!(
            finding["launch_context"]["limits"]["timeout_seconds"],
            timeout
        );
        assert_eq!(finding["launch_context"]["limits"]["memory_mb"], 256);
    }
}

#[test]
fn configured_test_paths_are_relative_to_config_and_cli_replaces_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(
        root.join("project/target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    fs::write(
        root.join("project/checks.py"),
        "from target import inspect\ninspect(False)\nassert False, 'configured test reached'\n",
    )
    .unwrap();
    fs::write(
        root.join("project/override.py"),
        "from target import inspect\nassert inspect(False) is False\n",
    )
    .unwrap();
    fs::write(
        root.join("project/settings.json"),
        json!({"schema_version": 1, "defaults": {"test_files": ["checks.py"]}}).to_string(),
    )
    .unwrap();
    let configured = run(root, &["--tests-only"]);
    assert_eq!(
        configured.status.code(),
        Some(1),
        "{}\n{}",
        String::from_utf8_lossy(&configured.stdout),
        String::from_utf8_lossy(&configured.stderr)
    );
    assert!(String::from_utf8_lossy(&configured.stdout).contains("configured test reached"));
    let overridden = run(
        root,
        &["--tests-only", "--test-file", "project/override.py"],
    );
    assert_eq!(
        overridden.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&overridden.stdout),
        String::from_utf8_lossy(&overridden.stderr)
    );
}

#[test]
fn invalid_repository_defaults_fail_before_execution_even_when_overridden() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("project")).unwrap();
    fs::write(
        root.join("project/target.py"),
        "raise RuntimeError('must not execute')\n",
    )
    .unwrap();
    for (config, expected) in [
        (
            json!({"schema_version": 2}),
            "unsupported repository config schema_version",
        ),
        (
            json!({"schema_version": 1, "defualts": {}}),
            "unknown field",
        ),
        (
            json!({"schema_version": 1, "defaults": {"timeout_seconds": 0}}),
            "timeout-seconds",
        ),
        (
            json!({"schema_version": 1, "defaults": {"memory_mb": 0}}),
            "memory-mb",
        ),
        (
            json!({"schema_version": 1, "defaults": {"runtime_profile": "typo"}}),
            "runtime-profile",
        ),
        (
            json!({"schema_version": 1, "targets": [{"source": "target.py", "test_files": []}]}),
            "at least one test file",
        ),
        (
            json!({"schema_version": 1, "targets": [{"source": "target.py", "test_files": ["checks.py"]}, {"source": "./target.py", "test_files": ["other.py"]}]}),
            "duplicate configured source",
        ),
        (
            json!({"schema_version": 1, "targets": [{"source": "missing.py", "test_files": ["checks.py"]}]}),
            "configured target missing.py unavailable",
        ),
        (
            json!({"schema_version": 1, "targets": [{"source": "target.py", "test_files": [""]}]}),
            "paths must not be empty",
        ),
    ] {
        fs::write(root.join("project/settings.json"), config.to_string()).unwrap();
        let output = run(
            root,
            &[
                "--timeout-seconds",
                "5",
                "--memory-mb",
                "256",
                "--runtime-profile",
                "local-trusted",
            ],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
