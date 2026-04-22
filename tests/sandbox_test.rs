use court_jester_mcp::tools::sandbox::execute;
use court_jester_mcp::types::Language;

fn tsx_loader_from_path() -> Option<String> {
    let path_env = std::env::var("PATH").ok()?;
    for dir in path_env.split(':') {
        let candidate = std::path::Path::new(dir).join("tsx");
        if candidate.exists() {
            let canonical = std::fs::canonicalize(candidate).ok()?;
            let loader = canonical.parent()?.join("loader.mjs");
            if loader.exists() {
                return Some(loader.to_string_lossy().to_string());
            }
        }
    }
    None
}

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
async fn python_source_file_executes_original_file_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("main.py");
    let code = "import os\nprint(os.path.basename(__file__))";
    std::fs::write(&source_path, code).unwrap();

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
    assert_eq!(r.stdout.trim(), "main.py");
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
async fn python_relative_import_source_file_executes_original_module_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("mypkg");
    std::fs::create_dir(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("helper.py"), "VALUE = 42").unwrap();

    let source_path = pkg.join("main.py");
    let code = "from .helper import VALUE\nfrom pathlib import Path\nprint(VALUE)\nprint(Path(__file__).name)";
    std::fs::write(&source_path, code).unwrap();

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
    let lines: Vec<_> = r.stdout.lines().collect();
    assert_eq!(lines, vec!["42", "main.py"]);
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

#[tokio::test]
async fn typescript_memory_limit_counts_child_processes() {
    let code = r#"
import { spawn } from "node:child_process";

spawn(
  process.execPath,
  ["-e", "const buf = new Uint8Array(200_000_000); buf.fill(1); setInterval(() => {}, 1000);"],
  { stdio: "ignore" }
);

setInterval(() => {}, 1000);
"#;
    let r = execute(code, &Language::TypeScript, 5.0, 64, None, None).await;
    assert!(
        r.memory_error,
        "expected child-process RSS to trip memory limit, got: {:?}",
        r
    );
}
#[tokio::test]
async fn typescript_source_file_retries_with_node_loader_for_type_alias_imports() {
    let dir = tempfile::tempdir().unwrap();
    let helper_path = dir.path().join("internals.ts");
    let source_path = dir.path().join("object.ts");

    std::fs::write(
        &helper_path,
        "export type PathValue = string | number | Array<string | number>;\n",
    )
    .unwrap();
    let code = r#"
import { PathValue } from "./internals.ts";

function pick(object: Record<string, unknown>, path: PathValue): unknown {
  const key = String(path);
  return object[key];
}

const mode = process.execArgv.includes("--import") ? "loader" : "transform";
console.log(`${mode}:${String(pick({ timezone: "UTC" }, "timezone"))}`);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "loader:UTC");
}

#[tokio::test]
async fn typescript_source_file_uses_loader_for_type_only_reexport_chain() {
    let dir = tempfile::tempdir().unwrap();
    let helper_path = dir.path().join("internals.ts");
    let index_path = dir.path().join("index.ts");
    let source_path = dir.path().join("object.ts");

    std::fs::write(
        &helper_path,
        "export type PathValue = string | number | Array<string | number>;\n",
    )
    .unwrap();
    std::fs::write(
        &index_path,
        "export type { PathValue } from \"./internals.ts\";\n",
    )
    .unwrap();

    let code = r#"
import { PathValue } from "./index.ts";

function pick(object: Record<string, unknown>, path: PathValue): unknown {
  const key = String(path);
  return object[key];
}

const mode = process.execArgv.includes("--import") ? "loader" : "transform";
console.log(`${mode}:${String(pick({ timezone: "UTC" }, "timezone"))}`);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "loader:UTC");
}

#[tokio::test]
async fn typescript_source_file_prefers_node_transform_over_bun_for_plain_relative_imports() {
    let tsx_loader = tsx_loader_from_path();
    assert!(
        tsx_loader.is_some(),
        "tsx loader must be available for this regression test"
    );

    let dir = tempfile::tempdir().unwrap();
    let helper_path = dir.path().join("helper.ts");
    let source_path = dir.path().join("main.ts");

    std::fs::write(&helper_path, "export const value = 7;\n").unwrap();
    let code = r#"
import { value } from "./helper.ts";

const runtime = typeof process.versions.bun === "string" ? "bun" : "node";
const mode = process.execArgv.includes("--import") ? "loader" : "transform";
console.log(`${mode}:${runtime}:${value}`);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "transform:node:7");
}

#[tokio::test]
async fn typescript_project_dir_without_imports_uses_node_transform_path() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("main.ts");

    let code = r#"
const hasLoader = process.execArgv.includes("--import");
console.log(hasLoader ? "loader" : "transform");
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        Some(dir.path().to_str().unwrap()),
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "transform");
}

#[tokio::test]
async fn typescript_source_file_executes_original_file_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("main.ts");

    let code = r#"
console.log(process.argv[1]);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        None,
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stdout.trim().ends_with("main.ts"),
        "should execute original source file, got: {}",
        result.stdout
    );
}

#[tokio::test]
async fn typescript_source_file_resolves_repo_local_package_imports() {
    let dir = tempfile::tempdir().unwrap();
    let node_modules = dir.path().join("node_modules").join("demo-pkg");
    std::fs::create_dir_all(&node_modules).unwrap();
    std::fs::write(
        node_modules.join("package.json"),
        r#"{"name":"demo-pkg","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(node_modules.join("index.js"), "export const value = 42;\n").unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let source_path = src_dir.join("main.ts");
    let code = r#"
import { value } from "demo-pkg";
console.log(value);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        Some(dir.path().to_str().unwrap()),
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "42");
}

#[tokio::test]
async fn typescript_bun_repo_falls_back_from_node_for_extensionless_relative_imports() {
    let bun_ok = std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    assert!(bun_ok, "bun must be available for this regression test");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bun.lock"), "").unwrap();
    let helper_path = dir.path().join("helper.ts");
    let source_path = dir.path().join("main.ts");
    std::fs::write(&helper_path, "export const value = 9;\n").unwrap();
    let code = r#"
import { value } from "./helper";
console.log(`${typeof process.versions.bun === "string" ? "bun" : "node"}:${value}`);
"#;
    std::fs::write(&source_path, code).unwrap();

    let result = execute(
        code,
        &Language::TypeScript,
        10.0,
        128,
        Some(dir.path().to_str().unwrap()),
        Some(source_path.to_str().unwrap()),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "bun:9");
}
