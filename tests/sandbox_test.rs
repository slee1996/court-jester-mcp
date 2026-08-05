use court_jester::tools::sandbox::{build_instrumentation_overlay, execute};
use court_jester::types::{
    FailureDomain, InstrumentationMode, Language, NetworkPolicy, ProcessTerminationKind,
    RuntimeProfile, SandboxOptions, TestRunner,
};

fn sandbox_options<'a>(
    timeout_seconds: f64,
    memory_mb: u64,
    project_dir: Option<&'a str>,
    source_file: Option<&'a str>,
) -> SandboxOptions<'a> {
    SandboxOptions {
        timeout_seconds,
        memory_mb,
        runtime_profile: RuntimeProfile::LocalTrusted,
        network_policy: NetworkPolicy::Deny,
        harness_args: &[],
        docker_image: None,
        project_dir,
        source_file,
    }
}

#[tokio::test]
async fn python_hello_world() {
    let r = execute(
        "print('hello')",
        &Language::Python,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "hello");
    assert!(!r.timed_out);
    assert!(!r.memory_error);
}

#[tokio::test]
async fn python_network_access_is_denied_with_typed_diagnostic() {
    let r = execute(
        "import socket\nsocket.create_connection(('example.com', 80))",
        &Language::Python,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
    assert_ne!(r.exit_code, Some(0));
    assert!(r.stderr.contains("court-jester network access denied"));
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == court_jester::types::FailureKind::NetworkDenied));
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.domain == FailureDomain::Environment }));
}

#[tokio::test]
async fn typescript_network_and_process_access_are_denied_with_typed_diagnostics() {
    let r = execute(
        r#"try {
  fetch("http://example.com");
} catch (error) {
  console.error(String(error));
}
try {
  require("node:child_process").spawnSync(process.execPath, ["-e", ""]);
} catch (error) {
  console.error(String(error));
}
"#,
        &Language::TypeScript,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
    assert_eq!(
        r.exit_code,
        Some(0),
        "guarded probe should handle denied calls: {:?}",
        r
    );
    assert!(r.stderr.contains("court-jester network access denied"));
    assert!(r.stderr.contains("court-jester process spawn denied"));
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == court_jester::types::FailureKind::NetworkDenied));
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == court_jester::types::FailureKind::ProcessSpawnDenied));
    assert!(r
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.kind,
                court_jester::types::FailureKind::NetworkDenied
                    | court_jester::types::FailureKind::ProcessSpawnDenied
            )
        })
        .all(|diagnostic| diagnostic.domain == FailureDomain::Environment));
}

#[tokio::test]
async fn explicit_local_network_allow_does_not_install_guards() {
    let python_code = r#"
import socket
import subprocess
print(socket.socket.connect.__name__)
print(subprocess.Popen.__name__)
"#;
    let mut python_options = sandbox_options(10.0, 128, None, None);
    python_options.network_policy = NetworkPolicy::Allow;
    let python = execute(python_code, &Language::Python, python_options).await;
    assert_eq!(python.exit_code, Some(0), "stderr: {}", python.stderr);
    assert!(!python.stdout.contains("_deny_network"));
    assert!(!python.stdout.contains("_deny_process"));
    assert!(python
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.kind != court_jester::types::FailureKind::NetworkDenied));

    let typescript_code = r#"
console.log(fetch.name);
console.log(require("node:child_process").spawn.name);
"#;
    let mut typescript_options = sandbox_options(10.0, 128, None, None);
    typescript_options.network_policy = NetworkPolicy::Allow;
    let typescript = execute(typescript_code, &Language::TypeScript, typescript_options).await;
    assert_eq!(
        typescript.exit_code,
        Some(0),
        "stderr: {}",
        typescript.stderr
    );
    assert!(!typescript
        .stderr
        .contains("court-jester network access denied"));
    assert!(typescript
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.kind != court_jester::types::FailureKind::NetworkDenied));
}

#[tokio::test]
async fn python_syntax_error() {
    let r = execute(
        "def foo(:",
        &Language::Python,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
    assert_ne!(r.exit_code, Some(0));
    assert!(!r.stderr.is_empty());
}

#[tokio::test]
async fn python_timeout_preserves_partial_output_and_typed_termination() {
    let r = execute(
        "print('before', flush=True)\nimport time\ntime.sleep(100)",
        &Language::Python,
        sandbox_options(2.0, 128, None, None),
    )
    .await;
    assert!(r.timed_out, "expected timeout, got: {:?}", r);
    assert_eq!(
        r.termination.as_ref().map(|termination| termination.kind),
        Some(ProcessTerminationKind::TimedOut)
    );
    assert!(r.stdout.contains("before"), "partial output lost: {:?}", r);
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == court_jester::types::FailureKind::Timeout));
}

#[tokio::test]
async fn python_no_env_leak() {
    // USER, API keys should not be available (HOME is set for npx compat)
    let code = "import os\nprint(os.environ.get('USER', 'NONE'))\nprint(os.environ.get('OPENAI_API_KEY', 'NONE'))";
    let r = execute(
        code,
        &Language::Python,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
    assert!(
        r.stdout.contains("NONE"),
        "should not leak USER or API keys, got: {}",
        r.stdout
    );
}

#[tokio::test]
async fn project_dir_none_unchanged() {
    let r = execute(
        "print(1+1)",
        &Language::Python,
        sandbox_options(10.0, 128, None, None),
    )
    .await;
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
        sandbox_options(10.0, 128, Some(dir.path().to_str().unwrap()), None),
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
        sandbox_options(2.0, 128, Some(dir.path().to_str().unwrap()), None),
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
    let mut options = sandbox_options(5.0, 64, None, None);
    options.network_policy = NetworkPolicy::Allow;
    let r = execute(code, &Language::TypeScript, options).await;
    assert!(
        r.memory_error,
        "expected child-process RSS to trip memory limit, got: {:?}",
        r
    );
    assert_eq!(
        r.termination.as_ref().map(|termination| termination.kind),
        Some(ProcessTerminationKind::MemoryLimit)
    );
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == court_jester::types::FailureKind::MemoryLimit));
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "transform:UTC");
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "transform:UTC");
}

#[tokio::test]
async fn typescript_source_file_prefers_node_transform_over_bun_for_plain_relative_imports() {
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
        sandbox_options(
            10.0,
            128,
            Some(dir.path().to_str().unwrap()),
            Some(source_path.to_str().unwrap()),
        ),
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
        sandbox_options(10.0, 128, None, Some(source_path.to_str().unwrap())),
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
        sandbox_options(
            10.0,
            128,
            Some(dir.path().to_str().unwrap()),
            Some(source_path.to_str().unwrap()),
        ),
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
    if !bun_ok {
        return;
    }

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
        sandbox_options(
            10.0,
            128,
            Some(dir.path().to_str().unwrap()),
            Some(source_path.to_str().unwrap()),
        ),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "bun:9");
}
#[test]
fn sandbox_options_reject_invalid_limits_and_profile_overrides() {
    let invalid_timeout = SandboxOptions {
        timeout_seconds: f64::NAN,
        memory_mb: 128,
        runtime_profile: RuntimeProfile::LocalTrusted,
        network_policy: NetworkPolicy::Deny,
        harness_args: &[],
        docker_image: None,
        project_dir: None,
        source_file: None,
    };
    assert!(invalid_timeout.validate().is_err());
    let invalid_image = SandboxOptions {
        timeout_seconds: 1.0,
        memory_mb: 1,
        runtime_profile: RuntimeProfile::LocalTrusted,
        network_policy: NetworkPolicy::Deny,
        harness_args: &[],
        docker_image: Some("-bad:image"),
        project_dir: None,
        source_file: None,
    };
    assert!(invalid_image.validate().is_err());
    let isolated_without_image = SandboxOptions {
        timeout_seconds: 1.0,
        memory_mb: 1,
        runtime_profile: RuntimeProfile::Isolated,
        network_policy: NetworkPolicy::Deny,
        harness_args: &[],
        docker_image: None,
        project_dir: None,
        source_file: None,
    };
    assert!(isolated_without_image.validate().is_err());
    let isolated_with_network = SandboxOptions {
        timeout_seconds: 1.0,
        memory_mb: 1,
        runtime_profile: RuntimeProfile::Isolated,
        network_policy: NetworkPolicy::Allow,
        harness_args: &[],
        docker_image: Some("python:3.12-slim"),
        project_dir: None,
        source_file: None,
    };
    assert!(isolated_with_network.validate().is_err());
}

#[test]
fn authoritative_test_overlay_is_language_and_runner_specific() {
    let python = build_instrumentation_overlay(
        &Language::Python,
        TestRunner::Auto,
        "src/main.py",
        &["f:1".into()],
    );
    assert_eq!(python.mode, InstrumentationMode::PythonSitecustomize);
    assert!(python.supported);
    let node = build_instrumentation_overlay(
        &Language::TypeScript,
        TestRunner::Node,
        "src/main.ts",
        &["f:1".into()],
    );
    assert_eq!(node.mode, InstrumentationMode::NodeModuleRegister);
    assert!(node.supported);
    let native = build_instrumentation_overlay(
        &Language::TypeScript,
        TestRunner::RepoNative,
        "src/main.ts",
        &[],
    );
    assert_eq!(native.mode, InstrumentationMode::Unsupported);
    assert!(!native.supported);
    assert_eq!(
        native.reason.as_deref(),
        Some("repo-native runner does not expose a module transform hook")
    );
}

#[tokio::test]
async fn isolated_python_execution_is_guarded_and_uses_selected_image() {
    let docker_available = std::process::Command::new("docker")
        .arg("info")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !docker_available {
        return;
    }
    let image_available = std::process::Command::new("docker")
        .args(["image", "inspect", "python:3.12-slim"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !image_available {
        return;
    }
    let options = SandboxOptions {
        timeout_seconds: 10.0,
        memory_mb: 128,
        runtime_profile: RuntimeProfile::Isolated,
        network_policy: NetworkPolicy::Deny,
        harness_args: &[],
        docker_image: Some("python:3.12-slim"),
        project_dir: None,
        source_file: None,
    };
    let result = execute("print('isolated')", &Language::Python, options).await;
    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "isolated");
    assert!(!result.timed_out);
}
