use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_court-jester"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn fixture() -> (tempfile::TempDir, String, String) {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Tests"]);
    git(root, &["config", "user.email", "tests@example.com"]);
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
    fs::write(root.join(".court-jester.json"), json!({"schema_version":1,"defaults":{"timeout_seconds":3},"targets":[{"source":"target.py","test_files":["checks.py"]}]}).to_string()).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "candidate"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    (repo, base, head)
}

#[test]
fn committed_source_tests_and_config_ignore_dirty_tree_and_workspace_survives() {
    let (repo, base, head) = fixture();
    let root = repo.path();
    fs::write(root.join("target.py"), "invalid syntax (\n").unwrap();
    fs::write(
        root.join("checks.py"),
        "raise RuntimeError('dirty tests must not run')\n",
    )
    .unwrap();
    fs::write(root.join(".court-jester.json"), "invalid config").unwrap();
    let output = cli(
        root,
        &[
            "ci",
            "--base",
            &base,
            "--head",
            &head,
            "--candidate-state",
            "committed",
            "--output-dir",
            "reports",
            "--report",
            "json",
            "--gate",
            "parse,test",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["candidate_state"], "committed");
    assert_eq!(report["head_commit"], head);
    let workspace = Path::new(report["candidate_workspace"].as_str().unwrap());
    assert!(workspace.is_dir());
    assert_eq!(
        fs::read_to_string(workspace.join("target.py")).unwrap(),
        "def inspect(value: bool):\n    return value\n"
    );
    let stages = report["files"][0]["report"]["stages"].as_array().unwrap();
    assert_eq!(
        stages.iter().find(|s| s["name"] == "test").unwrap()["status"],
        "passed"
    );
    assert_eq!(
        fs::read_to_string(root.join("target.py")).unwrap(),
        "invalid syntax (\n"
    );
    let working = cli(
        root,
        &["ci", "--base", &base, "--candidate-state", "working-tree"],
    );
    assert_eq!(working.status.code(), Some(2));
}

#[test]
fn committed_inspection_selects_revision_and_respects_cli_overrides_without_retention() {
    let (repo, _, head) = fixture();
    let root = repo.path();
    fs::write(
        root.join(".court-jester.json"),
        json!({"schema_version":1,"defaults":{"timeout_seconds":7}}).to_string(),
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "later config"]);
    fs::create_dir(root.join("nested")).unwrap();
    for (extra, timeout) in [(vec![], 3.0), (vec!["--timeout-seconds", "5"], 5.0)] {
        let mut args = vec![
            "ci",
            "--candidate-state",
            "committed",
            "--head",
            &head,
            "--show-config",
            "--llm-plateau-command",
            "--file",
        ];
        args.extend(extra);
        let output = cli(&root.join("nested"), &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(config["config_state"], "committed");
        assert_eq!(config["execution_started"], false);
        assert_eq!(config["selected_head"], head);
        assert_eq!(config["limits"]["timeouts"]["python_seconds"], timeout);
        assert!(!Path::new(config["candidate_workspace"].as_str().unwrap()).exists());
    }
}

#[test]
fn committed_selection_rejects_external_inputs_and_requires_retained_output() {
    let (repo, base, _) = fixture();
    let root = repo.path();
    let missing = cli(
        root,
        &["ci", "--candidate-state", "committed", "--base", &base],
    );
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--output-dir"));
    let external = tempfile::tempdir().unwrap();
    let path = external.path().join("settings.json");
    fs::write(&path, "{\"schema_version\":1}").unwrap();
    let output = cli(
        root,
        &[
            "ci",
            "--candidate-state",
            "committed",
            "--show-config",
            "--repo-config",
            path.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("within the invocation repository"));
    let output = cli(
        root,
        &["verify", "--candidate-state", "committed", "--show-config"],
    );
    assert_eq!(output.status.code(), Some(2));
    let literal = cli(
        root,
        &[
            "verify",
            "--show-config",
            "--llm-plateau-command",
            "--candidate-state",
        ],
    );
    assert_eq!(
        literal.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&literal.stderr)
    );
}

#[test]
fn committed_failure_replays_from_retained_source_after_cli_exit() {
    let (repo, base, _) = fixture();
    let root = repo.path();
    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    raise ValueError('committed failure')\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "bug"]);
    fs::write(
        root.join("target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    let output = cli(
        root,
        &[
            "ci",
            "--base",
            &base,
            "--candidate-state",
            "committed",
            "--no-repo-config",
            "--output-dir",
            "reports",
            "--report",
            "json",
            "--gate",
            "execute",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let file = &report["files"][0]["report"];
    let execute = file["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["name"] == "execute")
        .unwrap();
    let id = execute["detail"]["findings"][0]["id"].as_str().unwrap();
    let persisted = file["report_path"].as_str().unwrap();
    let output = cli(root, &["replay", "--report", persisted, "--finding", id]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replay: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(replay["outcome"], "reproduced");
    let workspace = Path::new(report["candidate_workspace"].as_str().unwrap());
    fs::write(
        workspace.join("target.py"),
        "def inspect(value: bool):\n    return value\n",
    )
    .unwrap();
    let output = cli(root, &["replay", "--report", persisted, "--finding", id]);
    assert_eq!(output.status.code(), Some(1));
    let replay: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(replay["outcome"], "not_reproduced");
}

#[test]
fn committed_typescript_tests_import_committed_siblings_from_nested_invocation() {
    let (repo, _, _) = fixture();
    let root = repo.path();
    fs::create_dir(root.join("typescript")).unwrap();
    fs::write(
        root.join("typescript/helper.ts"),
        "export const expected = false;\n",
    )
    .unwrap();
    fs::write(root.join("typescript/target.ts"), "import { expected } from './helper.ts';\nexport function inspect(value: boolean): boolean { return !expected; }\n").unwrap();
    fs::write(root.join("typescript/checks.test.ts"), "import { test } from 'node:test';\nimport assert from 'node:assert/strict';\nimport { inspect } from './target.ts';\ntest('committed dependency', () => assert.equal(inspect(false), false));\n").unwrap();
    fs::write(root.join("typescript/.court-jester.json"), json!({"schema_version":1,"defaults":{"test_runner":"node"},"targets":[{"source":"target.ts","test_files":["checks.test.ts"]}]}).to_string()).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "typescript base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    fs::write(root.join("typescript/target.ts"), "import { expected } from './helper.ts';\nexport function inspect(value: boolean): boolean { return expected; }\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "typescript fix"]);
    fs::write(
        root.join("typescript/helper.ts"),
        "export const expected = true;\n",
    )
    .unwrap();
    let output = cli(
        &root.join("typescript"),
        &[
            "ci",
            "--base",
            &base,
            "--candidate-state",
            "committed",
            "--output-dir",
            "reports",
            "--report",
            "json",
            "--gate",
            "test",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checked_files"], 1);
    assert_eq!(report["files"][0]["file"], "typescript/target.ts");
    assert!(fs::read_to_string(root.join("typescript/helper.ts"))
        .unwrap()
        .contains("true"));
}

#[test]
fn concurrent_committed_runs_retain_distinct_workspaces_and_reports() {
    let (repo, base, _) = fixture();
    let root = repo.path();
    let children = (0..4)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_court-jester"))
                .current_dir(root)
                .args([
                    "ci",
                    "--base",
                    &base,
                    "--candidate-state",
                    "committed",
                    "--output-dir",
                    "reports",
                    "--report",
                    "json",
                    "--gate",
                    "parse",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mut workspaces = std::collections::HashSet::new();
    let mut reports = std::collections::HashSet::new();
    let outputs = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect::<Vec<_>>();
    for output in outputs {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(workspaces.insert(report["candidate_workspace"].as_str().unwrap().to_owned()));
        let path = report["files"][0]["report"]["report_path"]
            .as_str()
            .unwrap();
        assert!(
            reports.insert(path.to_owned()),
            "report overwritten: {path}"
        );
        let persisted: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(persisted["meta"]["source_file"]
            .as_str()
            .unwrap()
            .starts_with(report["candidate_workspace"].as_str().unwrap()));
    }
}
