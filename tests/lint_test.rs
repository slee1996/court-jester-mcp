use court_jester_mcp::tools::lint::lint;
use court_jester_mcp::tools::verify::{verify, VerifyOptions};
use court_jester_mcp::types::Language;
use std::fs;
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
    let script_path = dir.path().join(name);
    fs::write(&script_path, body).unwrap();
    #[cfg(unix)]
    make_executable(&script_path);
    dir
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
        Some("ruff not available on PATH or next to court-jester-mcp")
    );
}

#[test]
fn verify_fails_when_python_lint_runner_errors() {
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
            complexity_threshold: None,
            project_dir: None,
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
    assert!(!report.overall_ok, "runner error should fail verify");
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
