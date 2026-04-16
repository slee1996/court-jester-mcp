use court_jester_mcp::tools::lint::{lint, lint_with_options, LintOptions};
use court_jester_mcp::tools::verify::{verify, VerifyOptions};
use court_jester_mcp::types::Language;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn path_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    key: &'static str,
    old_value: Option<String>,
}

impl EnvVarGuard {
    fn prepend_path(prefix: &std::path::Path) -> Self {
        let old_value = std::env::var("PATH").ok();
        let old_path = old_value.clone().unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", prefix.display(), old_path));
        Self {
            key: "PATH",
            old_value,
        }
    }

    fn set(key: &'static str, value: &std::path::Path) -> Self {
        let old_value = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old_value }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(old_value) = &self.old_value {
            std::env::set_var(self.key, old_value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn install_fake_tool(name: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    install_fake_tool_at(dir.path(), name, body);
    dir
}

fn install_fake_tool_at(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script_path = dir.join(name);
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&script_path, body).unwrap();
    #[cfg(unix)]
    make_executable(&script_path);
    script_path
}

fn normalize_logged_path(path: &str) -> String {
    path.trim()
        .strip_prefix("/private")
        .unwrap_or(path.trim())
        .to_string()
}

fn assert_log_contains_path(log: &str, prefix: &str, expected: &Path) {
    let expected = normalize_logged_path(&expected.to_string_lossy());
    assert!(
        log.lines().any(|line| {
            line.strip_prefix(prefix)
                .map(normalize_logged_path)
                .as_deref()
                == Some(expected.as_str())
        }),
        "expected log to contain {prefix}{expected}, got:\n{log}"
    );
}

#[test]
fn lint_reports_python_runner_failure() {
    let _guard = path_lock().lock().unwrap_or_else(|e| e.into_inner());
    let tool_dir = install_fake_tool("ruff", "#!/bin/sh\necho 'bad ruff config' 1>&2\nexit 2\n");
    let _path = EnvVarGuard::prepend_path(tool_dir.path());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint(
        "def add(a: int, b: int) -> int:\n    return a + b",
        &Language::Python,
    ));

    assert!(result.error.is_some(), "lint runner failure should surface");
    assert!(
        result.diagnostics.is_empty(),
        "runner failure is not a lint finding"
    );
}

#[test]
fn lint_reports_python_unavailable_when_ruff_missing() {
    let _guard = path_lock().lock().unwrap_or_else(|e| e.into_inner());
    let empty_dir = tempfile::tempdir().unwrap();
    let fake_home = tempfile::tempdir().unwrap();
    let _path = EnvVarGuard::set("PATH", empty_dir.path());
    let _home = EnvVarGuard::set("HOME", fake_home.path());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint(
        "def add(a: int, b: int) -> int:\n    return a + b",
        &Language::Python,
    ));

    assert!(
        result.unavailable,
        "missing ruff should be marked unavailable"
    );
    assert_eq!(
        result.error.as_deref(),
        Some("ruff not available in project, on PATH, or next to court-jester")
    );
}

#[test]
fn verify_keeps_python_lint_runner_errors_advisory() {
    let _guard = path_lock().lock().unwrap_or_else(|e| e.into_inner());
    let tool_dir = install_fake_tool("ruff", "#!/bin/sh\necho 'bad ruff config' 1>&2\nexit 2\n");
    let _path = EnvVarGuard::prepend_path(tool_dir.path());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let report = runtime.block_on(verify(
        "def add(a: int, b: int) -> int:\n    return a + b",
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            tests_only: false,
            complexity_threshold: None,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            source_file: None,
            output_dir: None,
        },
    ));

    let lint_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "lint")
        .expect("lint stage should exist");
    assert!(!lint_stage.ok, "lint stage should fail on runner error");
    assert!(
        report.overall_ok,
        "lint runner errors should stay advisory for verify"
    );
}

#[test]
fn lint_reports_typescript_runner_failure() {
    let _guard = path_lock().lock().unwrap_or_else(|e| e.into_inner());
    let tool_dir = install_fake_tool("biome", "#!/bin/sh\necho 'biome crashed' 1>&2\nexit 2\n");
    let _path = EnvVarGuard::prepend_path(tool_dir.path());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint(
        "export function add(a: number, b: number): number { return a + b; }",
        &Language::TypeScript,
    ));

    assert!(result.error.is_some(), "lint runner failure should surface");
    assert!(
        result.diagnostics.is_empty(),
        "runner failure is not a lint finding"
    );
}

#[cfg(unix)]
#[test]
fn lint_reports_signal_kill_with_actionable_hint() {
    let _guard = path_lock().lock().unwrap_or_else(|e| e.into_inner());
    let tool_dir = install_fake_tool("ruff", "#!/bin/sh\nkill -9 $$\n");
    let _path = EnvVarGuard::prepend_path(tool_dir.path());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint(
        "def add(a: int, b: int) -> int:\n    return a + b",
        &Language::Python,
    ));

    assert!(
        result.unavailable,
        "signal-killed ruff should be treated as unavailable"
    );
    let error = result
        .error
        .expect("signal kill should surface an unavailable message");
    assert!(
        error.contains("signal 9"),
        "expected signal number in error, got: {error}"
    );
    assert!(
        error.contains(tool_dir.path().join("ruff").to_string_lossy().as_ref()),
        "expected executable path in error, got: {error}"
    );
    #[cfg(target_os = "macos")]
    assert!(
        error.contains("Gatekeeper/quarantine"),
        "expected macOS hint in error, got: {error}"
    );
}

#[test]
fn lint_uses_project_local_ruff_with_config_and_virtual_path() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join(".venv").join("bin");
    let log_path = project_dir.path().join("ruff.log");
    let stdin_path = project_dir.path().join("ruff.stdin");
    let config_path = project_dir.path().join("ruff.toml");
    fs::write(&config_path, "[lint]\n").unwrap();

    install_fake_tool_at(
        &tool_dir,
        "ruff",
        &format!(
            r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" > "{log}"
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{log}"
done
cat > "{stdin}"
cat <<'EOF'
[{{"code":"F401","message":"unused import","location":{{"row":1,"column":1}}}}]
EOF
exit 1
"#,
            log = log_path.display(),
            stdin = stdin_path.display(),
        ),
    );

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint_with_options(
        "import os\n",
        &Language::Python,
        LintOptions {
            source_file: None,
            project_dir: Some(project_dir.path().to_str().unwrap()),
            config_path: Some(config_path.to_str().unwrap()),
            virtual_file_path: Some("pkg/module.py"),
        },
    ));

    assert!(
        result.error.is_none(),
        "diagnostics should not be runner errors"
    );
    assert_eq!(result.diagnostics.len(), 1, "expected one lint diagnostic");

    let log = fs::read_to_string(&log_path).unwrap();
    assert_log_contains_path(&log, "cwd=", project_dir.path());
    assert!(log.contains("arg=check"));
    assert!(log.contains("arg=--output-format=json"));
    assert!(log.contains("arg=--config"));
    assert_log_contains_path(&log, "arg=", &config_path);
    assert!(log.contains("arg=--stdin-filename"));
    assert!(log.contains("arg=pkg/module.py"));
    assert_eq!(fs::read_to_string(&stdin_path).unwrap(), "import os\n");
}

#[test]
fn lint_uses_project_local_biome_with_source_file_and_config_path() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join("node_modules").join(".bin");
    let log_path = project_dir.path().join("biome.log");
    let config_path = project_dir.path().join("biome.json");
    let source_path = project_dir.path().join("src").join("app.ts");
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    fs::write(&config_path, "{}\n").unwrap();
    fs::write(
        &source_path,
        "export function add(a: number, b: number): number { return a + b; }\n",
    )
    .unwrap();

    install_fake_tool_at(
        &tool_dir,
        "biome",
        &format!(
            r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" > "{log}"
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{log}"
done
cat <<'EOF'
{{"diagnostics":[{{"category":"lint/suspicious/noDebugger","description":"avoid debugger","severity":"warning","location":{{"start":{{"line":1,"column":1}}}}}}]}}
EOF
exit 1
"#,
            log = log_path.display(),
        ),
    );

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint_with_options(
        "export function add(a: number, b: number): number { return a + b; }\n",
        &Language::TypeScript,
        LintOptions {
            source_file: Some(source_path.to_str().unwrap()),
            project_dir: Some(project_dir.path().to_str().unwrap()),
            config_path: Some(config_path.to_str().unwrap()),
            virtual_file_path: None,
        },
    ));

    assert!(
        result.error.is_none(),
        "diagnostics should not be runner errors"
    );
    assert_eq!(result.diagnostics.len(), 1, "expected one lint diagnostic");

    let log = fs::read_to_string(&log_path).unwrap();
    assert_log_contains_path(&log, "cwd=", project_dir.path());
    assert!(log.contains("arg=lint"));
    assert!(log.contains("arg=--reporter=json"));
    assert!(log.contains("arg=--config-path"));
    assert_log_contains_path(&log, "arg=", &config_path);
    assert_log_contains_path(&log, "arg=", &source_path);
}

#[test]
fn lint_materializes_inline_typescript_at_virtual_path_and_cleans_up() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join("node_modules").join(".bin");
    let log_path = project_dir.path().join("biome-inline.log");
    let content_path = project_dir.path().join("biome-inline-content.ts");
    let virtual_path = project_dir.path().join("src").join("inline_rule.test.ts");

    install_fake_tool_at(
        &tool_dir,
        "biome",
        &format!(
            r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" > "{log}"
last=""
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{log}"
  last="$arg"
done
cat "$last" > "{content}"
cat <<'EOF'
{{"diagnostics":[]}}
EOF
exit 0
"#,
            log = log_path.display(),
            content = content_path.display(),
        ),
    );

    let code = "export const answer = 42;\n";
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(lint_with_options(
        code,
        &Language::TypeScript,
        LintOptions {
            source_file: None,
            project_dir: Some(project_dir.path().to_str().unwrap()),
            config_path: None,
            virtual_file_path: Some("src/inline_rule.test.ts"),
        },
    ));

    assert!(
        result.error.is_none(),
        "inline TypeScript lint should succeed"
    );
    assert!(result.diagnostics.is_empty(), "expected no diagnostics");
    assert_eq!(fs::read_to_string(&content_path).unwrap(), code);
    assert!(
        !virtual_path.exists(),
        "inline lint file should be cleaned up after biome finishes"
    );

    let log = fs::read_to_string(&log_path).unwrap();
    assert_log_contains_path(&log, "cwd=", project_dir.path());
    assert_log_contains_path(&log, "arg=", &virtual_path);
}
