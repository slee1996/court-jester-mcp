use court_jester::resolve_execution_context;
use court_jester::tools::sandbox::{build_instrumentation_overlay, execute, execute_harness};
use court_jester::types::{
    ContextRequest, FailureDomain, HarnessArtifact, HarnessKind, HarnessRuntime, HarnessSpec,
    InstrumentationMode, Language, NetworkPolicy, ProcessTerminationKind, RuntimeProfile,
    SandboxOptions, SourceMode, TestRunner,
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
        instrumentation_target: None,
        instrumented_source: None,
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
async fn typescript_worker_exports_deny_direct_and_forged_internal_workers() {
    let code = r#"
import { Worker } from "node:worker_threads";

const workerTypes = [
  Worker,
  class InternalWorker extends Worker {},
];
for (const WorkerType of workerTypes) {
  try {
    const worker = new WorkerType("setInterval(() => {}, 1000)", { eval: true });
    console.log("worker-created");
    void worker.terminate();
  } catch (error) {
    console.error(String(error));
  }
}
"#;
    let result = execute(
        code,
        &Language::TypeScript,
        sandbox_options(10.0, 128, None, None),
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert!(
        !result.stdout.contains("worker-created"),
        "a guarded Worker constructor remained usable: {result:?}"
    );
    assert_eq!(
        result
            .stderr
            .matches("court-jester process spawn denied")
            .count(),
        2,
        "both direct and subclassed Worker construction must be denied: {result:?}"
    );
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
async fn generated_typescript_harness_resolves_scoped_package_from_target_package_root() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let package = workspace.join("packages/api");
    let source = package.join("src/routes/hotels.ts");
    let dependency = package.join("node_modules/@prisma/client");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&dependency).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const route = true;\n").unwrap();
    std::fs::write(
        dependency.join("package.json"),
        r#"{"name":"@prisma/client","type":"module","exports":{".":{"import":"./index.js","require":"./index.cjs"}}}"#,
    )
    .unwrap();
    std::fs::write(
        dependency.join("index.js"),
        "export const workspaceMarker = 'prisma-workspace-ok';\n",
    )
    .unwrap();
    std::fs::write(
        dependency.join("index.cjs"),
        "exports.workspaceMarker = 'wrong-require-condition';\n",
    )
    .unwrap();
    let harness_code =
        "import { workspaceMarker } from '@prisma/client';\nconsole.log(workspaceMarker);\n";
    let existing_harness = workspace.join(".court-jester/existing.ts");
    std::fs::create_dir_all(existing_harness.parent().unwrap()).unwrap();
    std::fs::write(&existing_harness, harness_code).unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    let generated = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: harness_code.into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(project_dir.as_ref()),
            Some(source_file.as_ref()),
        ),
    )
    .await
    .process;

    assert_eq!(generated.exit_code, Some(0), "stderr: {}", generated.stderr);
    assert_eq!(generated.stdout.trim(), "prisma-workspace-ok");

    let existing = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Existing {
                relative_path: ".court-jester/existing.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(project_dir.as_ref()),
            Some(source_file.as_ref()),
        ),
    )
    .await
    .process;

    assert_eq!(existing.exit_code, Some(0), "stderr: {}", existing.stderr);
    assert_eq!(existing.stdout.trim(), "prisma-workspace-ok");

    let bun_available = std::process::Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if bun_available {
        let bun = execute_harness(
            &context,
            HarnessSpec {
                kind: HarnessKind::Standalone,
                runtime: HarnessRuntime::BunScript,
                test_adapter: None,
                source_mode: SourceMode::TypeScript,
                artifact: HarnessArtifact::Generated {
                    code: harness_code.into(),
                    relative_path: ".court-jester/generated/execute.ts".into(),
                },
                args: Vec::new(),
                network: NetworkPolicy::Deny,
            },
            sandbox_options(
                10.0,
                128,
                Some(project_dir.as_ref()),
                Some(source_file.as_ref()),
            ),
        )
        .await
        .process;

        assert_eq!(bun.exit_code, Some(0), "stderr: {}", bun.stderr);
        assert_eq!(bun.stdout.trim(), "prisma-workspace-ok");
    }
}

#[tokio::test]
async fn typescript_instrumentation_intercepts_reads_without_rewriting_source() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let source = workspace.join("target.ts");
    let original = "export const marker = 'original';\n";
    let instrumented = "export const marker = 'instrumented';\n";
    std::fs::write(workspace.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, original).unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let source_file = source.to_string_lossy().into_owned();
    let mut limits = sandbox_options(
        10.0,
        128,
        Some(workspace.to_str().unwrap()),
        Some(&source_file),
    );
    limits.instrumentation_target = Some(&source_file);
    limits.instrumented_source = Some(instrumented);
    let harness = format!(
        "import {{ readFileSync }} from 'node:fs';\nprocess.stdout.write(readFileSync({}, 'utf8'));\n",
        serde_json::to_string(&source_file).unwrap()
    );

    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: harness,
                relative_path: ".court-jester/generated/instrumentation.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        limits,
    )
    .await
    .process;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout, instrumented);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), original);
}

#[cfg(unix)]
#[tokio::test]
async fn generated_harness_resolves_extensionless_imports_from_workspace_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let target = workspace.join("packages/api");
    let shared = workspace.join("packages/shared");
    let source = target.join("src/index.ts");
    let shared_link = target.join("node_modules/@acme/shared");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(shared.join("src/types")).unwrap();
    std::fs::create_dir_all(shared_link.parent().unwrap()).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(target.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const target = true;\n").unwrap();
    std::fs::write(
        shared.join("package.json"),
        r#"{"name":"@acme/shared","type":"module","exports":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        shared.join("src/index.ts"),
        "export { marker } from './types/tenant';\n",
    )
    .unwrap();
    std::fs::write(
        shared.join("src/types/tenant.ts"),
        "export const marker = 'workspace-extensionless-ok';\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("../../../shared", &shared_link).unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::GeneratedVerifier,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import { marker } from '@acme/shared';\nconsole.log(marker);\n".into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(workspace.to_str().unwrap()),
            Some(source.to_str().unwrap()),
        ),
    )
    .await
    .process;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "workspace-extensionless-ok");
}

async fn execute_typescript_alias_probe(
    tsconfig: &str,
    modules: &[(&str, &str)],
    harness_code: &str,
) -> court_jester::types::ExecutionResult {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let source = workspace.join("src/routes/probe.ts");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(workspace.join("tsconfig.json"), tsconfig).unwrap();
    std::fs::write(&source, "export const route = true;\n").unwrap();
    for (relative_path, code) in modules {
        let module = workspace.join(relative_path);
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(module, code).unwrap();
    }

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::GeneratedVerifier,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: harness_code.into(),
                relative_path: "src/routes/.court-jester-generated-verify.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(project_dir.as_ref()),
            Some(source_file.as_ref()),
        ),
    )
    .await
    .process
}

#[tokio::test]
async fn generated_node_harness_resolves_extensionless_relative_typescript_imports() {
    let result = execute_typescript_alias_probe(
        "{}",
        &[
            (
                "src/routes/model.ts",
                "import { marker } from './redaction';\nexport { marker };\n",
            ),
            (
                "src/routes/redaction.ts",
                "export const marker = 'extensionless-relative-ok';\n",
            ),
        ],
        "import { marker } from './model';\nconsole.log(marker);\n",
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "extensionless-relative-ok");
}

#[tokio::test]
async fn generated_node_harness_loads_project_json_without_import_attributes() {
    let result = execute_typescript_alias_probe(
        "{}",
        &[("src/routes/tenant.json", r#"{"tenant":"json-import-ok"}"#)],
        "import tenant from './tenant.json';\nconsole.log(tenant.tenant);\n",
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "json-import-ok");
}

#[tokio::test]
async fn generated_typescript_harness_resolves_base_url_without_paths() {
    let result = execute_typescript_alias_probe(
        r#"{"compilerOptions":{"baseUrl":"."}}"#,
        &[(
            "src/lib/base-url.ts",
            "export const marker = 'base-url-only';\n",
        )],
        "import { marker } from 'src/lib/base-url';\nconsole.log(marker);\n",
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "base-url-only");
}

#[tokio::test]
async fn generated_typescript_harness_prefers_most_specific_overlapping_path() {
    let result = execute_typescript_alias_probe(
        r#"{"compilerOptions":{"paths":{"*":["fallback/*"],"@app/*":["specific/*"]}}}"#,
        &[
            (
                "fallback/@app/value.ts",
                "export const marker = 'generic';\n",
            ),
            (
                "specific/value.ts",
                "export const marker = 'most-specific';\n",
            ),
        ],
        "import { marker } from '@app/value';\nconsole.log(marker);\n",
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "most-specific");
}

#[tokio::test]
async fn generated_typescript_harness_preserves_jsonc_strings_when_removing_trailing_commas() {
    let result = execute_typescript_alias_probe(
        r#"{
            // JSONC comments and trailing commas are valid in tsconfig.
            "compilerOptions": {
                "paths": {
                    "punctuation/*": ["src/,]/*",],
                },
            },
        }"#,
        &[(
            "src/,]/value.ts",
            "export const marker = 'jsonc-string-preserved';\n",
        )],
        "import { marker } from 'punctuation/value';\nconsole.log(marker);\n",
    )
    .await;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "jsonc-string-preserved");
}

#[tokio::test]
async fn generated_typescript_harness_resolves_extended_tsconfig_path_alias_inside_project() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let source = workspace.join("src/routes/oauth.ts");
    let aliased = workspace.join("src/lib/logs.ts");
    let generated_config = workspace.join(".nuxt/types/tsconfig.json");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(aliased.parent().unwrap()).unwrap();
    std::fs::create_dir_all(generated_config.parent().unwrap()).unwrap();
    std::fs::write(
        workspace.join("tsconfig.json"),
        r#"{"extends":"./.nuxt/types/tsconfig.json"}"#,
    )
    .unwrap();
    std::fs::write(
        &generated_config,
        r#"{"compilerOptions":{"paths":{"~/*":["../../*"]}}}"#,
    )
    .unwrap();
    std::fs::write(&source, "export const route = true;\n").unwrap();
    std::fs::write(&aliased, "export const marker = 'tsconfig-alias-ok';\n").unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    let mut runtimes = vec![HarnessRuntime::NodeScript];
    if std::process::Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        runtimes.push(HarnessRuntime::BunScript);
    }
    for runtime in runtimes {
        let result = execute_harness(
            &context,
            HarnessSpec {
                kind: HarnessKind::GeneratedVerifier,
                runtime,
                test_adapter: None,
                source_mode: SourceMode::TypeScript,
                artifact: HarnessArtifact::Generated {
                    code: "import { marker } from '~/src/lib/logs';\nconsole.log(marker);\n".into(),
                    relative_path: "src/routes/.court-jester-generated-verify.ts".into(),
                },
                args: Vec::new(),
                network: NetworkPolicy::Deny,
            },
            sandbox_options(
                10.0,
                128,
                Some(project_dir.as_ref()),
                Some(source_file.as_ref()),
            ),
        )
        .await
        .process;

        assert_eq!(
            result.exit_code,
            Some(0),
            "runtime {runtime:?}, result: {result:?}"
        );
        assert_eq!(result.stdout.trim(), "tsconfig-alias-ok");
    }
}

#[tokio::test]
async fn generated_typescript_harness_rejects_path_alias_outside_project_as_environment() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    let outside = dir.path().join("outside");
    let source = workspace.join("src/index.ts");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        workspace.join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"~/*":["../outside/*"]}}}"#,
    )
    .unwrap();
    std::fs::write(&source, "export const route = true;\n").unwrap();
    std::fs::write(
        outside.join("secret.ts"),
        "export const secret = 'leaked';\n",
    )
    .unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: &workspace,
        explicit_project_dir: Some(&workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::GeneratedVerifier,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import { secret } from '~/secret';\nconsole.log(secret);\n".into(),
                relative_path: "src/.court-jester-generated-verify.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(project_dir.as_ref()),
            Some(source_file.as_ref()),
        ),
    )
    .await
    .process;

    assert_ne!(result.exit_code, Some(0), "result: {result:?}");
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.domain == FailureDomain::Environment
            && diagnostic.kind == court_jester::types::FailureKind::ModuleLoad
            && diagnostic.component == court_jester::types::DiagnosticComponent::ModuleLoader
            && diagnostic.message.contains("escapes the project mirror")
    }));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.domain != FailureDomain::VerifierHarness));
    assert!(!result.stdout.contains("leaked"));
}

#[tokio::test]
async fn generated_typescript_harness_resolves_target_package_self_reference() {
    for has_hoisted_node_modules in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path();
        let package = workspace.join("packages/api");
        let source = package.join("src/routes/hotels.ts");
        let exported_module = package.join("src/self.js");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"@acme/api","type":"module","exports":{"./self":"./src/self.js"}}"#,
        )
        .unwrap();
        std::fs::write(&source, "export const route = true;\n").unwrap();
        std::fs::write(
            &exported_module,
            "export const packageMarker = 'target-self-reference';\n",
        )
        .unwrap();
        if has_hoisted_node_modules {
            std::fs::create_dir_all(workspace.join("node_modules")).unwrap();
        }

        let context = resolve_execution_context(ContextRequest {
            invocation_dir: workspace,
            explicit_project_dir: Some(workspace),
            target_file: Some(&source),
            test_file: None,
            language: Language::TypeScript,
            virtual_file_path: None,
        })
        .unwrap();
        assert_eq!(
            context
                .dependency_roots
                .iter()
                .any(|root| root == &context.target_package_root),
            !has_hoisted_node_modules
        );
        let project_dir = workspace.to_string_lossy();
        let source_file = source.to_string_lossy();
        let result = execute_harness(
            &context,
            HarnessSpec {
                kind: HarnessKind::Standalone,
                runtime: HarnessRuntime::NodeScript,
                test_adapter: None,
                source_mode: SourceMode::TypeScript,
                artifact: HarnessArtifact::Generated {
                    code: "import { packageMarker } from '@acme/api/self';\nconsole.log(packageMarker);\n"
                        .into(),
                    relative_path: ".court-jester/generated/execute.ts".into(),
                },
                args: Vec::new(),
                network: NetworkPolicy::Deny,
            },
            sandbox_options(
                10.0,
                128,
                Some(project_dir.as_ref()),
                Some(source_file.as_ref()),
            ),
        )
        .await
        .process;

        assert_eq!(
            result.exit_code,
            Some(0),
            "hoisted node_modules={has_hoisted_node_modules}, stderr: {}",
            result.stderr
        );
        assert_eq!(result.stdout.trim(), "target-self-reference");
    }
}

#[tokio::test]
async fn generated_typescript_harness_does_not_fall_through_broken_nearer_package() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let package = workspace.join("packages/api");
    let source = package.join("src/routes/hotels.ts");
    let nearer_dependency = package.join("node_modules/fixture-package");
    let farther_dependency = workspace.join("node_modules/fixture-package");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&nearer_dependency).unwrap();
    std::fs::create_dir_all(&farther_dependency).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const route = true;\n").unwrap();
    std::fs::write(
        nearer_dependency.join("package.json"),
        r#"{"name":"fixture-package","type":"module","exports":"./missing.js"}"#,
    )
    .unwrap();
    std::fs::write(
        farther_dependency.join("package.json"),
        r#"{"name":"fixture-package","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        farther_dependency.join("index.js"),
        "export const packageMarker = 'farther-workspace-version';\n",
    )
    .unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import { packageMarker } from 'fixture-package';\nconsole.log(packageMarker);\n"
                    .into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(project_dir.as_ref()),
            Some(source_file.as_ref()),
        ),
    )
    .await
    .process;

    assert_ne!(result.exit_code, Some(0), "result: {result:?}");
    assert!(
        result.stderr.contains("missing.js"),
        "expected nearer broken package error, stderr: {}",
        result.stderr
    );
    assert!(!result.stdout.contains("farther-workspace-version"));
}

#[tokio::test]
async fn generated_typescript_harness_preserves_workspace_packages_nearest_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let target = workspace.join("packages/api");
    let shared = workspace.join("packages/shared");
    let source = target.join("src/index.ts");
    let target_dependency = target.join("node_modules/fixture-package");
    let shared_dependency = shared.join("node_modules/fixture-package");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&target_dependency).unwrap();
    std::fs::create_dir_all(&shared_dependency).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(target.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(shared.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const target = true;\n").unwrap();
    std::fs::write(
        shared.join("index.js"),
        "import { marker } from 'fixture-package';\nconsole.log(marker);\n",
    )
    .unwrap();
    for (dependency, marker) in [
        (&target_dependency, "target-version"),
        (&shared_dependency, "shared-version"),
    ] {
        std::fs::write(
            dependency.join("package.json"),
            r#"{"name":"fixture-package","type":"module","exports":"./index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            dependency.join("index.js"),
            format!("export const marker = '{marker}';\n"),
        )
        .unwrap();
    }

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import '../../packages/shared/index.js';\n".into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(workspace.to_str().unwrap()),
            Some(source.to_str().unwrap()),
        ),
    )
    .await
    .process;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "shared-version");
}

#[tokio::test]
async fn generated_typescript_package_self_reference_loads_overlay_export() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let target = workspace.join("packages/api");
    let source = target.join("src/self.ts");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(
        target.join("package.json"),
        r#"{"name":"@acme/api","type":"module","exports":{"./self":"./src/self.ts"}}"#,
    )
    .unwrap();
    std::fs::write(
        &source,
        "export const marker = 'stale-disk-export';\nconsole.log(marker);\n",
    )
    .unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import { marker as selfMarker } from '@acme/api/self';\nexport const marker = 'candidate-overlay-export';\nconsole.log(selfMarker);\n".into(),
                relative_path: "packages/api/src/self.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(workspace.to_str().unwrap()),
            Some(source.to_str().unwrap()),
        ),
    )
    .await
    .process;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "candidate-overlay-export");
}

#[tokio::test]
async fn generated_typescript_dependency_preserves_broken_nested_package_failure() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let target = workspace.join("packages/api");
    let source = target.join("src/index.ts");
    let consumer = target.join("node_modules/consumer-package");
    let broken_nested = consumer.join("node_modules/fixture-package");
    let valid_farther = target.join("node_modules/fixture-package");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&broken_nested).unwrap();
    std::fs::create_dir_all(&valid_farther).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(target.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const target = true;\n").unwrap();
    std::fs::write(
        consumer.join("package.json"),
        r#"{"name":"consumer-package","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        consumer.join("index.js"),
        "import { marker } from 'fixture-package';\nconsole.log(marker);\n",
    )
    .unwrap();
    std::fs::write(
        broken_nested.join("package.json"),
        r#"{"name":"fixture-package","type":"module","exports":"./missing.js"}"#,
    )
    .unwrap();
    std::fs::write(
        valid_farther.join("package.json"),
        r#"{"name":"fixture-package","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        valid_farther.join("index.js"),
        "export const marker = 'farther-target-version';\n",
    )
    .unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import 'consumer-package';\n".into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(workspace.to_str().unwrap()),
            Some(source.to_str().unwrap()),
        ),
    )
    .await
    .process;

    assert_ne!(result.exit_code, Some(0), "result: {result:?}");
    assert!(result.stderr.contains("missing.js"), "result: {result:?}");
    assert!(!result.stdout.contains("farther-target-version"));
}

#[tokio::test]
async fn existing_typescript_harness_keeps_native_package_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();
    let target = workspace.join("packages/api");
    let source = target.join("src/index.ts");
    let harness = workspace.join(".court-jester/existing.ts");
    let ambient_dependency = workspace.join("node_modules/fixture-package");
    let target_dependency = target.join("node_modules/fixture-package");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(harness.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&ambient_dependency).unwrap();
    std::fs::create_dir_all(&target_dependency).unwrap();
    std::fs::write(
        workspace.join("package.json"),
        r#"{"private":true,"type":"module"}"#,
    )
    .unwrap();
    std::fs::write(target.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const target = true;\n").unwrap();
    std::fs::write(
        &harness,
        "import { marker } from 'fixture-package';\nconsole.log(marker);\n",
    )
    .unwrap();
    for (dependency, marker) in [
        (&ambient_dependency, "native-ambient-version"),
        (&target_dependency, "target-version"),
    ] {
        std::fs::write(
            dependency.join("package.json"),
            r#"{"name":"fixture-package","type":"module","exports":"./index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            dependency.join("index.js"),
            format!("export const marker = '{marker}';\n"),
        )
        .unwrap();
    }

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let result = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            test_adapter: None,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Existing {
                relative_path: ".court-jester/existing.ts".into(),
            },
            args: Vec::new(),
            network: NetworkPolicy::Deny,
        },
        sandbox_options(
            10.0,
            128,
            Some(workspace.to_str().unwrap()),
            Some(source.to_str().unwrap()),
        ),
    )
    .await
    .process;

    assert_eq!(result.exit_code, Some(0), "result: {result:?}");
    assert_eq!(result.stdout.trim(), "native-ambient-version");
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
        instrumentation_target: None,
        instrumented_source: None,
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
        instrumentation_target: None,
        instrumented_source: None,
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
        instrumentation_target: None,
        instrumented_source: None,
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
        instrumentation_target: None,
        instrumented_source: None,
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
        instrumentation_target: None,
        instrumented_source: None,
    };
    let result = execute("print('isolated')", &Language::Python, options).await;
    assert_eq!(result.exit_code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "isolated");
    assert!(!result.timed_out);
}

#[cfg(unix)]
#[tokio::test]
async fn isolated_typescript_preserves_pnpm_workspace_symlinks() {
    let docker_available = std::process::Command::new("docker")
        .arg("info")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !docker_available {
        return;
    }
    let image = "node:24-bookworm-slim";
    let image_available = std::process::Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !image_available {
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path();
    let package = workspace.join("packages/app");
    let source = package.join("src/index.ts");
    let store_package =
        workspace.join("node_modules/.pnpm/pnpm-fixture@1.0.0/node_modules/pnpm-fixture");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(package.join("node_modules")).unwrap();
    std::fs::create_dir_all(&store_package).unwrap();
    std::fs::write(workspace.join("package.json"), r#"{"private":true}"#).unwrap();
    std::fs::write(package.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(&source, "export const source = true;\n").unwrap();
    std::fs::write(
        store_package.join("package.json"),
        r#"{"name":"pnpm-fixture","type":"module","exports":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        store_package.join("index.js"),
        "export const marker = 'pnpm-topology-ok';\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        "../../../node_modules/.pnpm/pnpm-fixture@1.0.0/node_modules/pnpm-fixture",
        package.join("node_modules/pnpm-fixture"),
    )
    .unwrap();
    let existing_harness = workspace.join(".court-jester/existing.ts");
    std::fs::create_dir_all(existing_harness.parent().unwrap()).unwrap();
    std::fs::write(
        &existing_harness,
        "import { marker } from 'pnpm-fixture';\nconsole.log(marker);\n",
    )
    .unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(workspace),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    let project_dir = workspace.to_string_lossy();
    let source_file = source.to_string_lossy();
    let execution = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Generated {
                code: "import { marker } from 'pnpm-fixture';\nconsole.log(marker);\n".into(),
                relative_path: ".court-jester/generated/execute.ts".into(),
            },
            network: NetworkPolicy::Deny,
            args: vec![],
            test_adapter: None,
        },
        SandboxOptions {
            timeout_seconds: 10.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
            harness_args: &[],
            docker_image: Some(image),
            project_dir: Some(project_dir.as_ref()),
            source_file: Some(source_file.as_ref()),
            instrumentation_target: None,
            instrumented_source: None,
        },
    )
    .await;

    assert_eq!(
        execution.process.exit_code,
        Some(0),
        "stderr: {}",
        execution.process.stderr
    );
    assert_eq!(execution.process.stdout.trim(), "pnpm-topology-ok");

    let existing = execute_harness(
        &context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime: HarnessRuntime::NodeScript,
            source_mode: SourceMode::TypeScript,
            artifact: HarnessArtifact::Existing {
                relative_path: ".court-jester/existing.ts".into(),
            },
            network: NetworkPolicy::Deny,
            args: vec![],
            test_adapter: None,
        },
        SandboxOptions {
            timeout_seconds: 10.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
            harness_args: &[],
            docker_image: Some(image),
            project_dir: Some(project_dir.as_ref()),
            source_file: Some(source_file.as_ref()),
            instrumentation_target: None,
            instrumented_source: None,
        },
    )
    .await;

    assert_eq!(
        existing.process.exit_code,
        Some(0),
        "stderr: {}",
        existing.process.stderr
    );
    assert_eq!(existing.process.stdout.trim(), "pnpm-topology-ok");
}
