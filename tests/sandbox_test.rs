use court_jester_mcp::tools::sandbox::execute;
use court_jester_mcp::types::Language;

#[tokio::test]
async fn python_hello_world() {
    let r = execute("print('hello')", &Language::Python, 10.0, 128, None, None).await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "hello");
    assert!(!r.timed_out);
    assert!(!r.memory_error);
}

#[tokio::test]
async fn python_syntax_error() {
    let r = execute("def foo(:", &Language::Python, 10.0, 128, None, None).await;
    assert_ne!(r.exit_code, Some(0));
    assert!(!r.stderr.is_empty());
}

#[tokio::test]
async fn python_timeout() {
    let r = execute(
        "import time\ntime.sleep(100)",
        &Language::Python,
        2.0,
        128,
        None,
        None,
    )
    .await;
    assert!(r.timed_out, "expected timeout, got: {:?}", r);
}

#[tokio::test]
async fn python_no_env_leak() {
    // USER, API keys should not be available (HOME is set for npx compat)
    let code = "import os\nprint(os.environ.get('USER', 'NONE'))\nprint(os.environ.get('OPENAI_API_KEY', 'NONE'))";
    let r = execute(code, &Language::Python, 10.0, 128, None, None).await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert!(
        r.stdout.contains("NONE"),
        "should not leak USER or API keys, got: {}",
        r.stdout
    );
}

#[tokio::test]
async fn project_dir_none_unchanged() {
    let r = execute("print(1+1)", &Language::Python, 10.0, 128, None, None).await;
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.stdout.trim(), "2");
}

#[tokio::test]
async fn python_project_dir_imports_local_module() {
    let dir = tempfile::tempdir().unwrap();
    let mod_path = dir.path().join("mymod.py");
    std::fs::write(&mod_path, "x = 42").unwrap();

    let code = "from mymod import x\nprint(x)";
    let r = execute(
        code,
        &Language::Python,
        10.0,
        128,
        Some(dir.path().to_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "42");
}

#[tokio::test]
async fn project_dir_still_has_resource_limits() {
    let dir = tempfile::tempdir().unwrap();
    let r = execute(
        "import time\ntime.sleep(100)",
        &Language::Python,
        2.0,
        128,
        Some(dir.path().to_str().unwrap()),
        None,
    )
    .await;
    assert!(
        r.timed_out,
        "expected timeout with project_dir, got: {:?}",
        r
    );
}

#[tokio::test]
async fn source_file_resolves_relative_imports() {
    // Create a temp dir simulating a Python package with relative imports
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("mypkg");
    std::fs::create_dir(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("helper.py"), "ANSWER = 42").unwrap();

    // The "source file" uses a relative import
    let source_path = pkg.join("main.py");
    std::fs::write(&source_path, "").unwrap();

    // Code with a relative import — triggers sibling + python -m mode
    let code = "from .helper import ANSWER\nprint(ANSWER)";
    let r = execute(
        code,
        &Language::Python,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "42");
}

#[tokio::test]
async fn source_file_cleanup() {
    // Verify that sibling fuzz files are cleaned up after execution
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("mypkg");
    std::fs::create_dir(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("helper.py"), "X = 1").unwrap();
    let source_path = pkg.join("main.py");
    std::fs::write(&source_path, "").unwrap();

    // Code with relative import to trigger sibling mode
    let code = "from .helper import X\nprint(X)";
    let r = execute(
        code,
        &Language::Python,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);

    // Check no court_jester_fuzz_* files remain
    let remaining: Vec<_> = std::fs::read_dir(&pkg)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("court_jester_fuzz_")
        })
        .collect();
    assert!(
        remaining.is_empty(),
        "sibling file should be cleaned up, found: {:?}",
        remaining
    );
}
