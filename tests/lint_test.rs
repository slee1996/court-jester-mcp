use court_jester_mcp::tools::lint::lint;
use court_jester_mcp::tools::verify::{verify, VerifyOptions};
use court_jester_mcp::types::Language;
use std::fs;
use std::sync::{Mutex, OnceLock};

fn path_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PathGuard {
    old_path: String,
}

impl PathGuard {
    fn install(prefix: &std::path::Path) -> Self {
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", prefix.display(), old_path));
        Self { old_path }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        std::env::set_var("PATH", &self.old_path);
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
    let _guard = path_lock().lock().unwrap();
    let tool_dir = install_fake_tool("ruff", "#!/bin/sh\necho 'bad ruff config' 1>&2\nexit 2\n");
    let _path = PathGuard::install(tool_dir.path());

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
fn verify_fails_when_python_lint_runner_errors() {
    let _guard = path_lock().lock().unwrap();
    let tool_dir = install_fake_tool("ruff", "#!/bin/sh\necho 'bad ruff config' 1>&2\nexit 2\n");
    let _path = PathGuard::install(tool_dir.path());

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
    let _guard = path_lock().lock().unwrap();
    let tool_dir = install_fake_tool("biome", "#!/bin/sh\necho 'biome crashed' 1>&2\nexit 2\n");
    let _path = PathGuard::install(tool_dir.path());

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
