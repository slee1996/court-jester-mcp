use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tree_sitter::Parser;

use crate::tools::{analyze, diff, domain, lint, sandbox, synthesize, test_quality};
use crate::types::*;

mod corpus;
mod decisions;
use decisions::{
    build_report, has_non_target_blocking_diagnostic, is_typescript_module_load_error,
    is_typescript_portability_error,
};
pub use decisions::{final_verdict, stage_diagnostics};
mod provenance;
mod regression;
mod replay;
pub use regression::{
    prepare_regression_export, prepare_regression_export_with_candidate, write_regression_export,
    RegressionExportPlan,
};
mod report_text;
mod reporting;
use provenance::{stable_digest, tree_digest};
pub use replay::{
    load_persisted_report, replay_launch_context, replay_report,
    replay_report_with_candidate_options, replay_report_with_options,
};

use reporting::write_report;
pub use reporting::{
    report_human_summary, report_json_value, test_quality_summary, TestQualitySummary,
};

use corpus::{
    corpus_inputs, parse_corpus, persist_corpus, persistent_corpus_path, read_persistent_corpus,
    PersistentCorpus,
};
use report_text::{clipped_test_failure, sanitize_report_text, sanitize_report_value};

pub struct VerifyOptions<'a> {
    pub test_code: Option<&'a str>,
    pub test_source_file: Option<&'a str>,
    pub test_runner: TestRunner,
    pub tests_only: bool,
    pub test_quality_max_mutants: Option<usize>,
    pub complexity_threshold: Option<usize>,
    pub complexity_metric: ComplexityMetric,
    pub project_dir: Option<&'a str>,
    pub lint_config_path: Option<&'a str>,
    pub lint_virtual_file_path: Option<&'a str>,
    pub diff: Option<&'a str>,
    pub suppressions: Option<&'a str>,
    pub suppression_source: Option<&'a str>,
    pub auto_seed: bool,
    pub source_file: Option<&'a str>,
    pub base_code: Option<&'a str>,
    pub base_source_file: Option<&'a str>,
    pub base_project_dir: Option<&'a str>,
    pub output_dir: Option<&'a str>,
    pub report_level: ReportLevel,
    pub execute_gate: ExecuteGate,
    pub coverage_gate: CoverageGate,
    pub inferred_oracle_gate: InferredOracleGate,
    pub runtime_profile: RuntimeProfile,
    pub memory_mb: u64,
    pub network: NetworkPolicy,
    pub harness_args: Vec<HarnessArg>,
    pub python_docker_image: &'a str,
    pub typescript_docker_image: &'a str,
}

fn sandbox_options<'a>(
    opts: &'a VerifyOptions<'a>,
    language: &Language,
    timeout_seconds: f64,
    memory_mb: u64,
    project_dir: Option<&'a str>,
    source_file: Option<&'a str>,
) -> SandboxOptions<'a> {
    let docker_image = if opts.runtime_profile == RuntimeProfile::Isolated {
        Some(match language {
            Language::Python => opts.python_docker_image,
            Language::TypeScript => opts.typescript_docker_image,
        })
    } else {
        None
    };
    SandboxOptions {
        timeout_seconds,
        memory_mb,
        runtime_profile: opts.runtime_profile,
        network_policy: opts.network,
        harness_args: opts.harness_args.as_slice(),
        docker_image,
        project_dir,
        source_file,
        instrumentation_target: None,
        instrumented_source: None,
    }
}

/// Default execute-stage timeout for synthesized Python fuzz harnesses (seconds).
/// Overridable via `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS`.
const DEFAULT_PYTHON_EXEC_TIMEOUT: f64 = 10.0;

/// Default execute-stage timeout for synthesized TypeScript fuzz harnesses (seconds).
/// TypeScript is slower to boot (Node transform/loader startup plus transpile),
/// so it gets a longer default. Overridable via
/// `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS`.
const DEFAULT_TYPESCRIPT_EXEC_TIMEOUT: f64 = 25.0;

/// Default test-stage timeout (seconds). Overridable via
/// `COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS`.
const DEFAULT_TEST_TIMEOUT: f64 = 30.0;

fn env_timeout(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(default)
}

fn execute_timeout_for(language: &Language) -> f64 {
    match language {
        Language::Python => env_timeout(
            "COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS",
            DEFAULT_PYTHON_EXEC_TIMEOUT,
        ),
        Language::TypeScript => env_timeout(
            "COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS",
            DEFAULT_TYPESCRIPT_EXEC_TIMEOUT,
        ),
    }
}

fn test_timeout() -> f64 {
    env_timeout(
        "COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS",
        DEFAULT_TEST_TIMEOUT,
    )
}

/// Effective ordinary verification timeouts after process environment overrides.
/// Native engines and specialized adapters may apply their own minimum budgets.
#[derive(Debug, serde::Serialize)]
pub struct VerificationTimeouts {
    pub python_seconds: f64,
    pub typescript_seconds: f64,
    pub test_seconds: f64,
}

pub fn verification_timeouts() -> VerificationTimeouts {
    VerificationTimeouts {
        python_seconds: execute_timeout_for(&Language::Python),
        typescript_seconds: execute_timeout_for(&Language::TypeScript),
        test_seconds: test_timeout(),
    }
}

const DEFAULT_NATIVE_FUZZ_RUNS: usize = 1_000;

#[derive(Debug, Clone, Copy)]
struct NativeFuzzConfig {
    engine: NativeFuzzEngine,
    runs: usize,
}

fn native_fuzz_config() -> NativeFuzzConfig {
    let engine = std::env::var("COURT_JESTER_NATIVE_FUZZ_ENGINE")
        .ok()
        .as_deref()
        .and_then(NativeFuzzEngine::parse)
        .unwrap_or_default();
    let runs = std::env::var("COURT_JESTER_NATIVE_FUZZ_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|runs| (1..=1_000_000).contains(runs))
        .unwrap_or(DEFAULT_NATIVE_FUZZ_RUNS);
    NativeFuzzConfig { engine, runs }
}

fn test_code_has_imports(code: &str, language: &Language) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim_start();
        match language {
            Language::Python => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("from ")
                    || trimmed.contains("importlib.import_module(")
            }
            Language::TypeScript => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("export ")
                    || trimmed.contains("require(")
            }
        }
    })
}

fn typescript_code_imports_vitest(code: &str) -> bool {
    code.lines().any(|line| {
        let statement = line.split_once("//").map_or(line, |(code, _)| code).trim();
        let is_import_statement = statement.starts_with("import ")
            || statement.starts_with("export ")
            || statement.starts_with("} from ");
        is_import_statement
            && (statement.contains("from \"vitest\"")
                || statement.contains("from 'vitest'")
                || statement == "import \"vitest\";"
                || statement == "import 'vitest';")
    })
}
fn typescript_code_imports_node_test(code: &str) -> bool {
    code.lines().any(|line| {
        let statement = line.split_once("//").map_or(line, |(code, _)| code).trim();
        let is_import_statement = statement.starts_with("import ")
            || statement.starts_with("export ")
            || statement.starts_with("} from ");
        is_import_statement
            && (statement.contains("from \"node:test\"")
                || statement.contains("from 'node:test'")
                || statement == "import \"node:test\";"
                || statement == "import 'node:test';")
    })
}

fn typescript_code_imports_react_runtime(code: &str) -> bool {
    code.lines().any(|line| {
        let statement = line.split_once("//").map_or(line, |(code, _)| code).trim();
        let is_import_statement = statement.starts_with("import ")
            || statement.starts_with("export ")
            || statement.starts_with("} from ");
        is_import_statement
            && [
                "from \"react\"",
                "from 'react'",
                "\"react/",
                "'react/",
                "react-query",
            ]
            .iter()
            .any(|module| statement.contains(module))
    })
}

fn react_hook_surface_ids(code: &str, functions: &[FunctionInfo]) -> HashSet<String> {
    if !typescript_code_imports_react_runtime(code) {
        return HashSet::new();
    }
    functions
        .iter()
        .filter(|function| {
            function
                .name
                .strip_prefix("use")
                .and_then(|tail| tail.chars().next())
                .is_some_and(char::is_uppercase)
        })
        .map(|function| format!("{}:{}", function.name, function.line))
        .collect()
}

fn exclude_context_dependent_surfaces(plan: &mut VerificationPlan, surface_ids: &HashSet<String>) {
    for surface in &mut plan.surfaces {
        if surface_ids.contains(&surface.id) {
            surface.invocable = false;
        }
    }
    plan.inputs
        .retain(|input| !surface_ids.contains(&input.surface_id));
    plan.execution_units
        .retain(|unit| !surface_ids.contains(&unit.surface_id));
}

fn apply_context_dependent_coverage(
    coverage: &mut [FuzzFunctionCoverage],
    surface_ids: &HashSet<String>,
) {
    for entry in coverage {
        let surface_id = format!("{}:{}", entry.function, entry.line);
        if surface_ids.contains(&surface_id) {
            entry.status = FuzzFunctionStatus::SkippedNoFuzzableSurface;
            entry.reason =
                Some("React hooks require an authoritative renderer and provider context".into());
        }
    }
}

const VITEST_CONFIG_FILENAMES: &[&str] = &[
    "vitest.config.ts",
    "vitest.config.tsx",
    "vitest.config.js",
    "vitest.config.mjs",
    "vitest.config.cjs",
    "vitest.config.mts",
    "vitest.config.cts",
];

fn package_json_declares_vitest(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(package) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let dependency_declares_vitest = [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ]
    .iter()
    .any(|section| {
        package
            .get(section)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|dependencies| dependencies.contains_key("vitest"))
    });
    let script_runs_vitest = package
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|scripts| {
            scripts.values().any(|script| {
                script
                    .as_str()
                    .is_some_and(|command| command.split_whitespace().any(|word| word == "vitest"))
            })
        });
    dependency_declares_vitest || script_runs_vitest
}

fn context_declares_vitest(context: &ExecutionContext) -> bool {
    let start = context
        .test_source
        .as_ref()
        .and_then(|source| source.source_file.as_deref())
        .and_then(Path::parent)
        .or(context.test_package_root.as_deref())
        .unwrap_or(context.target_package_root.as_path());
    for directory in start.ancestors() {
        if !directory.starts_with(&context.workspace_root) {
            break;
        }
        if VITEST_CONFIG_FILENAMES
            .iter()
            .any(|name| directory.join(name).is_file())
            || package_json_declares_vitest(&directory.join("package.json"))
        {
            return true;
        }
        if directory == context.workspace_root {
            break;
        }
    }
    false
}

const NUXT_CONFIG_FILENAMES: &[&str] = &[
    "nuxt.config.ts",
    "nuxt.config.js",
    "nuxt.config.mjs",
    "nuxt.config.cjs",
];

const NUXT_GENERATED_IMPORT_FILES: &[&str] = &[".nuxt/imports.d.ts", ".nuxt/types/imports.d.ts"];

const NUXT_BUILTIN_AUTO_IMPORTS: &[&str] = &[
    "computed",
    "customRef",
    "effectScope",
    "inject",
    "isProxy",
    "isReactive",
    "isReadonly",
    "isRef",
    "markRaw",
    "nextTick",
    "onActivated",
    "onBeforeMount",
    "onBeforeUnmount",
    "onBeforeUpdate",
    "onDeactivated",
    "onErrorCaptured",
    "onMounted",
    "onServerPrefetch",
    "onUnmounted",
    "onUpdated",
    "provide",
    "reactive",
    "readonly",
    "ref",
    "shallowReactive",
    "shallowReadonly",
    "shallowRef",
    "toRaw",
    "toRef",
    "toRefs",
    "triggerRef",
    "unref",
    "watch",
    "watchEffect",
    "watchPostEffect",
    "watchSyncEffect",
    "useAsyncData",
    "useCookie",
    "useError",
    "useFetch",
    "useHead",
    "useHydration",
    "useLazyAsyncData",
    "useLazyFetch",
    "useNuxtApp",
    "useRequestEvent",
    "useRequestHeaders",
    "useRoute",
    "useRouter",
    "useRuntimeConfig",
    "useSeoMeta",
    "useState",
];

fn package_json_declares_nuxt(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(package) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ]
    .iter()
    .any(|section| {
        package
            .get(section)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|dependencies| dependencies.contains_key("nuxt"))
    })
}

fn nuxt_context_root(context: &ExecutionContext) -> Option<PathBuf> {
    let start = context
        .target_source
        .source_file
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(context.target_package_root.as_path());
    for directory in start.ancestors() {
        if !directory.starts_with(&context.workspace_root) {
            break;
        }
        if NUXT_CONFIG_FILENAMES
            .iter()
            .any(|name| directory.join(name).is_file())
            || package_json_declares_nuxt(&directory.join("package.json"))
        {
            return Some(directory.to_path_buf());
        }
        if directory == context.workspace_root {
            break;
        }
    }
    None
}
fn context_has_node_package(context: &ExecutionContext, package: &str) -> bool {
    std::iter::once(&context.target_package_root)
        .chain(context.dependency_roots.iter())
        .chain(std::iter::once(&context.workspace_root))
        .any(|root| {
            root.join("node_modules")
                .join(package)
                .join("package.json")
                .is_file()
        })
}

fn nuxt_runtime_prelude(context: &ExecutionContext) -> Option<&'static str> {
    nuxt_context_root(context)?;
    context_has_node_package(context, "vue").then_some(
        "import * as __court_jester_vue_runtime from \"vue\";\n\
         Object.assign(globalThis, __court_jester_vue_runtime);\n",
    )
}

fn project_adapter_contract(context: &ExecutionContext) -> ProjectAdapterContract {
    let nuxt_root = nuxt_context_root(context);
    let root = nuxt_root
        .clone()
        .unwrap_or_else(|| context.target_package_root.clone());
    let effective_config = [
        root.join(".nuxt/tsconfig.json"),
        root.join("tsconfig.json"),
        root.join("pyproject.toml"),
        root.join("package.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .map(|path| path.to_string_lossy().into_owned());
    let selected_runner = if nuxt_root.is_some() {
        ProjectRuntimeAdapterKind::Nuxt
    } else {
        match context.target_source.language {
            Language::Python => ProjectRuntimeAdapterKind::PlainPython,
            Language::TypeScript if context_declares_vitest(context) => {
                ProjectRuntimeAdapterKind::VitestVite
            }
            Language::TypeScript => ProjectRuntimeAdapterKind::PlainTypeScript,
        }
    };
    ProjectAdapterContract {
        kind: if nuxt_root.is_some() {
            ProjectAdapterKind::Nuxt
        } else {
            ProjectAdapterKind::Standalone
        },
        root: root.to_string_lossy().into_owned(),
        package_root: context.target_package_root.to_string_lossy().into_owned(),
        workspace_root: context.workspace_root.to_string_lossy().into_owned(),
        dependency_roots: context
            .dependency_roots
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        effective_config,
        selected_runner: Some(selected_runner),
        rationale: if nuxt_root.is_some() {
            vec!["Nuxt package metadata or configuration selected the Nuxt/Vite adapter".into()]
        } else {
            vec!["language and project package metadata selected the standalone adapter".into()]
        },
        capabilities: ProjectAdapterCapabilities {
            authoritative_source_overlay: context.target_source.source_file.is_some(),
            package_runtime: context.target_package_root.is_dir(),
            project_test_runner: context.test_source.is_some() || context_declares_vitest(context),
            framework_auto_import_runtime: nuxt_runtime_prelude(context).is_some(),
        },
    }
}

fn surface_execution_plans(
    functions: &[FunctionInfo],
    adapter: &ProjectAdapterContract,
    planned_coverage: &[FuzzFunctionCoverage],
    tests_only: bool,
    has_authoritative_test: bool,
) -> Vec<SurfaceExecutionPlan> {
    let planned = planned_coverage
        .iter()
        .map(|entry| ((entry.function.as_str(), entry.line), entry))
        .collect::<HashMap<_, _>>();
    functions
        .iter()
        .filter(|function| function.is_exported)
        .map(|function| {
            let generated_supported = planned
                .get(&(function.name.as_str(), function.line))
                .is_some_and(|entry| {
                    matches!(
                        entry.status,
                        FuzzFunctionStatus::CheckedDirect
                            | FuzzFunctionStatus::CheckedViaFactory
                            | FuzzFunctionStatus::CheckedViaCaller
                    )
                });
            let strategy = if tests_only
                || (adapter.kind == ProjectAdapterKind::Nuxt && has_authoritative_test)
                || (!generated_supported && has_authoritative_test)
            {
                SurfaceExecutionStrategy::AuthoritativeProjectRunner
            } else if adapter.kind == ProjectAdapterKind::Nuxt {
                SurfaceExecutionStrategy::FrameworkRuntime
            } else if generated_supported {
                SurfaceExecutionStrategy::GeneratedHarness
            } else {
                SurfaceExecutionStrategy::StaticOnly
            };
            let unsupported_requirements =
                if matches!(strategy, SurfaceExecutionStrategy::StaticOnly) {
                    vec![planned
                        .get(&(function.name.as_str(), function.line))
                        .and_then(|entry| entry.reason.clone())
                        .unwrap_or_else(|| {
                            "no safe generated, project-test, or framework runner is available"
                                .into()
                        })]
                } else {
                    Vec::new()
                };
            SurfaceExecutionPlan {
                surface_id: format!("{}:{}", function.name, function.line),
                strategy,
                unsupported_requirements,
                expected_evidence: match strategy {
                    SurfaceExecutionStrategy::GeneratedHarness => "property_checked",
                    SurfaceExecutionStrategy::AuthoritativeProjectRunner => "authoritative_tests",
                    SurfaceExecutionStrategy::FrameworkRuntime => "runtime_smoke",
                    SurfaceExecutionStrategy::StaticOnly => "static_checked",
                }
                .into(),
            }
        })
        .collect()
}
fn nuxt_config_disables_auto_imports(root: &Path) -> bool {
    fn object_disables_auto_imports(node: tree_sitter::Node<'_>, source: &str) -> bool {
        if node.kind() != "object" {
            return false;
        }
        let mut cursor = node.walk();
        let disabled = node.named_children(&mut cursor).any(|property| {
            if property.kind() != "pair" {
                return false;
            }
            let key = property
                .child_by_field_name("key")
                .and_then(|key| key.utf8_text(source.as_bytes()).ok())
                .map(|key| key.trim_matches(['\'', '"']));
            key == Some("autoImport")
                && property
                    .child_by_field_name("value")
                    .is_some_and(|value| value.kind() == "false")
        });
        disabled
    }

    fn config_disables_auto_imports(node: tree_sitter::Node<'_>, source: &str) -> bool {
        if node.kind() == "pair" {
            let key = node
                .child_by_field_name("key")
                .and_then(|key| key.utf8_text(source.as_bytes()).ok())
                .map(|key| key.trim_matches(['\'', '"']));
            if key == Some("imports")
                && node
                    .child_by_field_name("value")
                    .is_some_and(|value| object_disables_auto_imports(value, source))
            {
                return true;
            }
        }
        let mut cursor = node.walk();
        let disabled = node
            .named_children(&mut cursor)
            .any(|child| config_disables_auto_imports(child, source));
        disabled
    }

    let mut parser = Parser::new();
    let grammar = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    if parser.set_language(&grammar).is_err() {
        return false;
    }
    NUXT_CONFIG_FILENAMES.iter().any(|filename| {
        let Ok(source) = std::fs::read_to_string(root.join(filename)) else {
            return false;
        };
        parser
            .parse(&source, None)
            .is_some_and(|tree| config_disables_auto_imports(tree.root_node(), &source))
    })
}

fn text_contains_identifier(text: &str, identifier: &str) -> bool {
    text.match_indices(identifier).any(|(start, _)| {
        let identifier_byte =
            |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$';
        let before = start
            .checked_sub(1)
            .and_then(|index| text.as_bytes().get(index));
        let after = text.as_bytes().get(start + identifier.len());
        before.is_none_or(|byte| !identifier_byte(*byte))
            && after.is_none_or(|byte| !identifier_byte(*byte))
    })
}

fn nuxt_generated_import_declares(root: &Path, identifier: &str) -> bool {
    NUXT_GENERATED_IMPORT_FILES.iter().any(|relative| {
        std::fs::read_to_string(root.join(relative))
            .is_ok_and(|text| text_contains_identifier(&text, identifier))
    })
}

fn nuxt_auto_import_source_exists(directory: &Path, identifier: &str, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let Ok(file_type) = entry.file_type() else {
            return false;
        };
        let path = entry.path();
        if file_type.is_dir() {
            return nuxt_auto_import_source_exists(&path, identifier, depth - 1);
        }
        if !file_type.is_file()
            || !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
            )
        {
            return false;
        }
        let stem_matches = path.file_stem().and_then(|stem| stem.to_str()) == Some(identifier);
        let index_parent_matches = path.file_stem().and_then(|stem| stem.to_str()) == Some("index")
            && path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some(identifier);
        stem_matches || index_parent_matches
    })
}

fn nuxt_project_auto_import_declares(root: &Path, identifier: &str) -> bool {
    ["composables", "utils"]
        .iter()
        .any(|directory| nuxt_auto_import_source_exists(&root.join(directory), identifier, 8))
}

fn missing_reference_identifier(message: &str) -> Option<&str> {
    let identifier = message.strip_suffix(" is not defined")?;
    (!identifier.is_empty()
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'))
    .then_some(identifier)
}

fn missing_reference_global(finding: &VerificationFinding) -> Option<&str> {
    if finding.severity != FindingSeverity::Crash
        || finding.error_type.as_deref() != Some("ReferenceError")
    {
        return None;
    }
    missing_reference_identifier(&finding.message)
}

fn missing_reference_global_in_stderr(stderr: &str) -> Option<&str> {
    stderr.lines().find_map(|line| {
        let (_, message) = line.split_once("ReferenceError:")?;
        missing_reference_identifier(message.trim())
    })
}

fn nuxt_framework_global(root: &Path, identifier: &str) -> bool {
    NUXT_BUILTIN_AUTO_IMPORTS.contains(&identifier)
        || nuxt_generated_import_declares(root, identifier)
        || nuxt_project_auto_import_declares(root, identifier)
}

#[derive(Debug)]
struct NuxtRuntimeBlocker {
    missing_globals: Vec<String>,
    affected_surfaces: HashSet<String>,
    affected_findings: usize,
    blocked_before_harness: bool,
}

impl NuxtRuntimeBlocker {
    fn blocks_surface(&self, surface: &str) -> bool {
        if self.blocked_before_harness {
            return true;
        }
        let root = surface
            .strip_suffix(" (factory->nested)")
            .or_else(|| surface.strip_suffix(" (factory)"))
            .unwrap_or(surface);
        self.affected_surfaces
            .iter()
            .any(|affected| affected == root || affected.starts_with(&format!("{root}().")))
    }

    fn diagnostic(&self) -> FailureDiagnostic {
        FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ContextResolution,
            component: DiagnosticComponent::FuzzHarness,
            impact: DiagnosticImpact::Blocking,
            message: format!(
                "Nuxt auto-import runtime unavailable for generated verification harness: {}. Run the target through a Nuxt test/runtime setup.",
                self.missing_globals.join(", ")
            ),
            process: None,
            limits: None,
        }
    }
}

fn take_nuxt_runtime_failures(
    context: &ExecutionContext,
    findings: &mut Vec<VerificationFinding>,
    stderr: &str,
) -> Option<NuxtRuntimeBlocker> {
    let root = nuxt_context_root(context)?;
    if nuxt_config_disables_auto_imports(&root) {
        return None;
    }
    let mut missing_globals = HashSet::new();
    let mut affected_surfaces = HashSet::new();
    let mut affected_findings = 0;
    findings.retain(|finding| {
        let Some(identifier) = missing_reference_global(finding) else {
            return true;
        };
        if !nuxt_framework_global(&root, identifier) {
            return true;
        }
        missing_globals.insert(identifier.to_string());
        affected_surfaces.insert(finding.location.function.clone());
        affected_findings += 1;
        false
    });
    let blocked_before_harness = if affected_findings == 0 {
        let identifier = missing_reference_global_in_stderr(stderr)?;
        if !nuxt_framework_global(&root, identifier) {
            return None;
        }
        missing_globals.insert(identifier.to_string());
        true
    } else {
        false
    };
    let mut missing_globals = missing_globals.into_iter().collect::<Vec<_>>();
    missing_globals.sort();
    Some(NuxtRuntimeBlocker {
        missing_globals,
        affected_surfaces,
        affected_findings,
        blocked_before_harness,
    })
}

fn apply_nuxt_runtime_coverage(
    coverage: &mut [FuzzFunctionCoverage],
    blocker: &NuxtRuntimeBlocker,
) {
    for function in coverage {
        let blocked = blocker.blocks_surface(&function.function)
            || matches!(
                &function.invocation_path,
                InvocationPath::Factory { factory, .. } if blocker.blocks_surface(factory)
            );
        if blocked
            && matches!(
                function.status,
                FuzzFunctionStatus::CheckedDirect
                    | FuzzFunctionStatus::ReachedDirect
                    | FuzzFunctionStatus::ReachedViaFactory
                    | FuzzFunctionStatus::CheckedViaFactory
                    | FuzzFunctionStatus::CheckedViaCaller
                    | FuzzFunctionStatus::BlockedModuleLoad
            )
        {
            function.status = FuzzFunctionStatus::SkippedNoFuzzableSurface;
            function.reason = Some(format!(
                "Nuxt auto-import runtime unavailable in generated harness: {}",
                blocker.missing_globals.join(", ")
            ));
        }
    }
}

fn normalize_nuxt_runtime_stdout(
    stdout: &str,
    retained_findings: &[VerificationFinding],
    blocker: &NuxtRuntimeBlocker,
) -> String {
    let mut retained_lines = Vec::new();
    let mut lines = stdout.lines();
    while let Some(line) = lines.next() {
        if line.starts_with(sandbox::HARNESS_EVENT_SENTINEL) {
            continue;
        }
        if line == "__COURT_JESTER_FINDINGS_JSON__" {
            let _ = lines.next();
            continue;
        }
        let target_crash_summary = line
            .strip_prefix("FUZZ ")
            .and_then(|rest| rest.split_once(':').map(|(surface, _)| surface))
            .is_some_and(|surface| blocker.blocks_surface(surface));
        let repeated_crash = line.trim_start().starts_with("CRASH ")
            && blocker
                .missing_globals
                .iter()
                .any(|global| line.contains(&format!("{global} is not defined")));
        if !target_crash_summary && !repeated_crash {
            retained_lines.push(line.to_string());
        }
    }
    retained_lines.push(format!(
        "ENVIRONMENT Nuxt auto-import runtime unavailable: {}",
        blocker.missing_globals.join(", ")
    ));
    if !retained_findings.is_empty() {
        retained_lines.push("__COURT_JESTER_FINDINGS_JSON__".into());
        retained_lines.push(
            serde_json::to_string(retained_findings)
                .expect("verification findings always serialize"),
        );
    }
    retained_lines.join("\n")
}

fn err_execution_result(message: &str) -> ExecutionResult {
    ExecutionResult {
        stdout: String::new(),
        stderr: message.to_string(),
        exit_code: None,
        duration_ms: 0,
        timed_out: false,
        memory_error: false,
        termination: Some(ProcessTermination {
            kind: ProcessTerminationKind::LaunchFailed,
            exit_code: None,
            signal: None,
            signal_name: None,
        }),
        diagnostics: vec![],
    }
}

fn parse_target_entered_events(stderr: &str) -> HashSet<String> {
    parse_target_entered_lines(stderr.lines().map(str::trim))
}

fn authoritative_output_line<'a>(
    line: &'a str,
    language: &Language,
    runner: TestRunner,
) -> &'a str {
    let adapter = (*language == Language::TypeScript && runner == TestRunner::Node)
        .then_some(TestAdapter::NodeTap);
    sandbox::test_output_line(line, adapter)
}

fn parse_target_entered_lines<'a>(lines: impl Iterator<Item = &'a str>) -> HashSet<String> {
    let mut entered = HashSet::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("event").and_then(|v| v.as_str()) == Some("target_entered") {
            if let Some(surface) = value.get("surface_id").and_then(|v| v.as_str()) {
                entered.insert(surface.to_string());
            }
        }
    }
    entered
}

fn apply_runtime_coverage_proof(
    coverage: &mut [FuzzFunctionCoverage],
    execution: &ExecutionResult,
) {
    let entered = parse_target_entered_events(&execution.stderr);
    let completed = sandbox::parse_harness_events(&execution.stdout).ok();
    for item in coverage {
        let proved = entered.iter().any(|surface| {
            surface == &item.function
                || surface.starts_with(&format!("{}:", item.function))
                || surface.contains(&format!("().{}", item.function))
        });
        if item.status == FuzzFunctionStatus::CheckedDirect && proved {
            let identity = format!("{}:{}", item.function, item.line);
            let valid_completed = completed
                .as_ref()
                .and_then(|summary| summary.surfaces.get(&identity))
                .is_some_and(|evidence| evidence.valid_completed > 0);
            if !valid_completed {
                item.status = FuzzFunctionStatus::ReachedDirect;
                item.reason =
                    Some("target entered but no valid completed invocation was recorded".into());
            }
        }
        if !proved
            && matches!(
                item.status,
                FuzzFunctionStatus::CheckedDirect
                    | FuzzFunctionStatus::CheckedViaFactory
                    | FuzzFunctionStatus::CheckedViaCaller
            )
        {
            item.status = FuzzFunctionStatus::SkippedNoFuzzableSurface;
            item.reason =
                Some("instrumentation did not observe target_entered for this surface".into());
        }
    }
}
fn instrument_source_for_surfaces(
    code: &str,
    functions: &[&FunctionInfo],
    language: &Language,
    source_mode: SourceMode,
) -> Result<String, String> {
    struct InstrumentationTarget<'a> {
        line: usize,
        analyzer_name: &'a str,
        surface_id: String,
    }

    fn unqualified_name(name: &str) -> &str {
        name.rsplit(['.', '#']).next().unwrap_or(name)
    }

    fn enclosing_named_node(
        mut node: tree_sitter::Node<'_>,
        kinds: &[&str],
        code: &str,
    ) -> Option<String> {
        while let Some(parent) = node.parent() {
            if kinds.contains(&parent.kind()) {
                return parent
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(code.as_bytes()).ok())
                    .map(str::to_owned);
            }
            if matches!(
                parent.kind(),
                "function_definition"
                    | "function_declaration"
                    | "method_definition"
                    | "arrow_function"
                    | "function_expression"
            ) {
                return None;
            }
            node = parent;
        }
        None
    }

    fn syntax_identity(node: tree_sitter::Node<'_>, syntax_name: &str, code: &str) -> String {
        if node.kind() == "method_definition" {
            if let Some(class_name) =
                enclosing_named_node(node, &["class_declaration", "class"], code)
            {
                return format!("{class_name}#{syntax_name}");
            }
        }
        if matches!(node.kind(), "method_definition" | "pair") {
            if let Some(object_name) = enclosing_named_node(node, &["variable_declarator"], code) {
                return format!("{object_name}.{syntax_name}");
            }
        }
        syntax_name.to_string()
    }

    fn matching_target<'a>(
        targets: &'a [InstrumentationTarget<'_>],
        line: usize,
        identity: &str,
        syntax_name: &str,
        qualified_name_allowed: bool,
    ) -> Option<&'a InstrumentationTarget<'a>> {
        if let Some(target) = targets
            .iter()
            .find(|target| target.line == line && target.analyzer_name == identity)
        {
            return Some(target);
        }
        let mut fallback = targets.iter().filter(|target| {
            target.line == line
                && (target.analyzer_name == syntax_name
                    || (qualified_name_allowed
                        && unqualified_name(target.analyzer_name) == syntax_name))
        });
        let matched = fallback.next()?;
        fallback.next().is_none().then_some(matched)
    }

    fn instrument_callable(
        callable: tree_sitter::Node<'_>,
        body: tree_sitter::Node<'_>,
        target: &InstrumentationTarget<'_>,
        code: &str,
        language: &Language,
        insertions: &mut Vec<(usize, String)>,
        instrumented: &mut HashSet<String>,
    ) {
        if instrumented.contains(&target.surface_id) {
            return;
        }
        match language {
            Language::Python if body.kind() == "block" => {
                let offset = body.start_byte();
                let line_start = code[..offset]
                    .rfind('\n')
                    .map(|index| index + 1)
                    .unwrap_or(0);
                let indent = &code[line_start..offset];
                insertions.push((offset, format!("import sys as _cj_sys, json as _cj_json; print(_cj_json.dumps({{'event': 'target_entered', 'surface_id': '{}'}}), file=_cj_sys.stderr)\n{}", target.surface_id, indent)));
                instrumented.insert(target.surface_id.clone());
            }
            Language::TypeScript if body.kind() == "statement_block" => {
                insertions.push((body.start_byte() + 1, format!("\nglobalThis.process.stderr.write(JSON.stringify({{event: 'target_entered', surface_id: '{}'}}) + \"\\n\");", target.surface_id)));
                instrumented.insert(target.surface_id.clone());
            }
            Language::TypeScript if callable.kind() == "arrow_function" => {
                insertions.push((body.start_byte(), format!("{{ globalThis.process.stderr.write(JSON.stringify({{event: 'target_entered', surface_id: '{}'}}) + \"\\n\"); return ", target.surface_id)));
                insertions.push((body.end_byte(), "; }".into()));
                instrumented.insert(target.surface_id.clone());
            }
            _ => {}
        }
    }

    fn walk(
        node: tree_sitter::Node<'_>,
        code: &str,
        language: &Language,
        targets: &[InstrumentationTarget<'_>],
        insertions: &mut Vec<(usize, String)>,
        instrumented: &mut HashSet<String>,
    ) {
        let line = node.start_position().row + 1;

        if *language == Language::TypeScript && node.kind() == "variable_declarator" {
            let name = node
                .child_by_field_name("name")
                .and_then(|child| child.utf8_text(code.as_bytes()).ok());
            let callable = node
                .child_by_field_name("value")
                .filter(|value| matches!(value.kind(), "arrow_function" | "function_expression"));
            if let (Some(name), Some(callable)) = (name, callable) {
                if let (Some(target), Some(body)) = (
                    matching_target(targets, line, name, name, false),
                    callable.child_by_field_name("body"),
                ) {
                    instrument_callable(
                        callable,
                        body,
                        target,
                        code,
                        language,
                        insertions,
                        instrumented,
                    );
                }
            }
        } else if *language == Language::TypeScript && node.kind() == "pair" {
            let name = node
                .child_by_field_name("key")
                .and_then(|child| child.utf8_text(code.as_bytes()).ok())
                .map(|name| name.trim_matches(['\'', '"']));
            let callable = node
                .child_by_field_name("value")
                .filter(|value| matches!(value.kind(), "arrow_function" | "function_expression"));
            if let (Some(name), Some(callable)) = (name, callable) {
                let identity = syntax_identity(node, name, code);
                if let (Some(target), Some(body)) = (
                    matching_target(targets, line, &identity, name, true),
                    callable.child_by_field_name("body"),
                ) {
                    instrument_callable(
                        callable,
                        body,
                        target,
                        code,
                        language,
                        insertions,
                        instrumented,
                    );
                }
            }
        } else {
            let name = node
                .child_by_field_name("name")
                .and_then(|child| child.utf8_text(code.as_bytes()).ok());
            if let (Some(name), Some(body)) = (name, node.child_by_field_name("body")) {
                let qualified_name_allowed =
                    *language == Language::TypeScript && node.kind() == "method_definition";
                let identity = syntax_identity(node, name, code);
                if let Some(target) =
                    matching_target(targets, line, &identity, name, qualified_name_allowed)
                {
                    instrument_callable(
                        node,
                        body,
                        target,
                        code,
                        language,
                        insertions,
                        instrumented,
                    );
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, code, language, targets, insertions, instrumented);
        }
    }

    let mut parser = Parser::new();
    let grammar = match source_mode {
        SourceMode::Python => tree_sitter_python::LANGUAGE.into(),
        SourceMode::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceMode::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser
        .set_language(&grammar)
        .map_err(|error| format!("instrumentation parser unavailable: {error}"))?;
    let tree = parser
        .parse(code, None)
        .ok_or_else(|| "instrumentation parser produced no tree".to_string())?;
    let targets = functions
        .iter()
        .map(|function| InstrumentationTarget {
            line: function.line,
            analyzer_name: function.name.as_str(),
            surface_id: format!("{}:{}", function.name, function.line),
        })
        .collect::<Vec<_>>();
    let mut insertions = Vec::<(usize, String)>::new();
    let mut instrumented = HashSet::new();
    walk(
        tree.root_node(),
        code,
        language,
        &targets,
        &mut insertions,
        &mut instrumented,
    );
    if instrumented.len() != targets.len() {
        return Err(format!(
            "instrumentation located {} of {} required function bodies",
            instrumented.len(),
            targets.len()
        ));
    }
    insertions.sort_by_key(|(offset, _)| *offset);
    let mut output = code.to_string();
    for (offset, text) in insertions.into_iter().rev() {
        output.insert_str(offset, &text);
    }
    Ok(output)
}

struct PreparedAuthoritativeTest {
    _root: Option<tempfile::TempDir>,
    code: String,
    project_dir: Option<String>,
    source_file: Option<String>,
    instrumented_source: Option<String>,
    overlay: InstrumentationOverlay,
}

#[cfg(unix)]
fn mirror_test_overlay(
    source_root: &Path,
    destination_root: &Path,
    relative_dir: &Path,
    special: &[PathBuf],
) -> Result<(), String> {
    use std::os::unix::fs::symlink;
    let source_dir = source_root.join(relative_dir);
    let destination_dir = destination_root.join(relative_dir);
    std::fs::create_dir_all(&destination_dir)
        .map_err(|error| format!("failed to create instrumentation overlay: {error}"))?;
    let entries = std::fs::read_dir(&source_dir)
        .map_err(|error| format!("failed to read project for instrumentation: {error}"))?;
    for entry in entries.flatten() {
        let relative = relative_dir.join(entry.file_name());
        let is_special_path = special
            .iter()
            .any(|path| path == &relative || path.starts_with(&relative));
        if is_special_path {
            if entry.path().is_dir() {
                mirror_test_overlay(source_root, destination_root, &relative, special)?;
            }
            continue;
        }
        let destination = destination_root.join(&relative);
        if entry
            .file_type()
            .map_err(|error| format!("failed to inspect overlay entry: {error}"))?
            .is_file()
        {
            std::fs::copy(entry.path(), destination).map_err(|error| {
                format!(
                    "failed to copy instrumentation overlay '{}': {error}",
                    relative.display()
                )
            })?;
        } else {
            symlink(entry.path(), destination).map_err(|error| {
                format!(
                    "failed to link instrumentation overlay '{}': {error}",
                    relative.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn mirror_test_overlay(_: &Path, _: &Path, _: &Path, _: &[PathBuf]) -> Result<(), String> {
    Err("authoritative-test import instrumentation is unsupported on this platform".into())
}

fn absolute_test_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

fn common_test_project_root(source_file: &Path, test_source_file: &Path) -> Option<PathBuf> {
    let source_parent = source_file.parent()?;
    let test_parent = test_source_file.parent()?;
    source_parent
        .ancestors()
        .find(|candidate| candidate.parent().is_some() && test_parent.starts_with(candidate))
        .map(Path::to_path_buf)
}

fn inferred_test_project_root(
    project_dir: Option<&str>,
    source_file: Option<&Path>,
    test_source_file: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(project_dir) = project_dir {
        return Some(absolute_test_path(Path::new(project_dir)));
    }
    match (source_file, test_source_file) {
        (Some(source), Some(test)) => common_test_project_root(source, test),
        (Some(source), None) => source
            .parent()
            .filter(|parent| parent.parent().is_some())
            .map(Path::to_path_buf),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_authoritative_test(
    code: &str,
    tests: &str,
    functions: &[&FunctionInfo],
    language: &Language,
    source_mode: SourceMode,
    runner: TestRunner,
    project_dir: Option<&str>,
    source_file: Option<&str>,
    test_source_file: Option<&str>,
) -> PreparedAuthoritativeTest {
    let surfaces = functions
        .iter()
        .map(|function| format!("{}:{}", function.name, function.line))
        .collect::<Vec<_>>();
    let imported = test_code_has_imports(tests, language);
    let mut overlay = sandbox::build_instrumentation_overlay(
        language,
        runner,
        source_file.unwrap_or("<inline>"),
        &surfaces,
    );
    let instrumented = match instrument_source_for_surfaces(code, functions, language, source_mode)
    {
        Ok(instrumented) => instrumented,
        Err(reason) => {
            overlay.supported = false;
            overlay.reason = Some(reason);
            return PreparedAuthoritativeTest {
                _root: None,
                code: tests.into(),
                project_dir: project_dir.map(str::to_string),
                source_file: test_source_file.map(str::to_string),
                instrumented_source: None,
                overlay,
            };
        }
    };
    if !imported {
        return PreparedAuthoritativeTest {
            _root: None,
            code: format!("{instrumented}\n\n{tests}"),
            project_dir: project_dir.map(str::to_string),
            source_file: source_file.map(str::to_string),
            instrumented_source: None,
            overlay,
        };
    }

    let source_path = source_file.map(|path| absolute_test_path(Path::new(path)));
    let test_path = test_source_file.map(|path| absolute_test_path(Path::new(path)));
    let Some(project) =
        inferred_test_project_root(project_dir, source_path.as_deref(), test_path.as_deref())
    else {
        overlay.supported = false;
        overlay.reason = Some(
            "imported authoritative tests require a source path or explicit project directory"
                .into(),
        );
        return PreparedAuthoritativeTest {
            _root: None,
            code: tests.into(),
            project_dir: None,
            source_file: test_source_file.map(str::to_string),
            instrumented_source: None,
            overlay,
        };
    };
    let Some(source_path) = source_path else {
        overlay.supported = false;
        overlay.reason = Some("imported authoritative tests require a target source path".into());
        return PreparedAuthoritativeTest {
            _root: None,
            code: tests.into(),
            project_dir: Some(project.to_string_lossy().into_owned()),
            source_file: test_source_file.map(str::to_string),
            instrumented_source: None,
            overlay,
        };
    };
    let target = source_path
        .strip_prefix(&project)
        .ok()
        .map(Path::to_path_buf);
    let test = match test_path.as_deref() {
        Some(test_path) => test_path.strip_prefix(&project).ok().map(Path::to_path_buf),
        None => target.as_ref().map(|target| {
            target
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(match language {
                    Language::Python => "court_jester_authoritative_test.py",
                    Language::TypeScript => "court_jester_authoritative_test.ts",
                })
        }),
    };
    let (Some(target), Some(test)) = (target, test) else {
        overlay.supported = false;
        overlay.reason = Some("target and test paths must be inside the project directory".into());
        return PreparedAuthoritativeTest {
            _root: None,
            code: tests.into(),
            project_dir: Some(project.to_string_lossy().into_owned()),
            source_file: test_source_file.map(str::to_string),
            instrumented_source: None,
            overlay,
        };
    };
    if *language == Language::TypeScript {
        if let Some(test_path) = test_path {
            return PreparedAuthoritativeTest {
                _root: None,
                code: tests.into(),
                project_dir: Some(project.to_string_lossy().into_owned()),
                source_file: Some(test_path.to_string_lossy().into_owned()),
                instrumented_source: Some(instrumented),
                overlay,
            };
        }
    }
    let root = match tempfile::tempdir() {
        Ok(root) => root,
        Err(error) => {
            overlay.supported = false;
            overlay.reason = Some(format!("failed to create instrumentation overlay: {error}"));
            return PreparedAuthoritativeTest {
                _root: None,
                code: tests.into(),
                project_dir: Some(project.to_string_lossy().into_owned()),
                source_file: test_source_file.map(str::to_string),
                instrumented_source: None,
                overlay,
            };
        }
    };
    if let Err(reason) = mirror_test_overlay(
        &project,
        root.path(),
        Path::new(""),
        &[target.clone(), test.clone()],
    ) {
        overlay.supported = false;
        overlay.reason = Some(reason);
        return PreparedAuthoritativeTest {
            _root: Some(root),
            code: tests.into(),
            project_dir: Some(project.to_string_lossy().into_owned()),
            source_file: test_source_file.map(str::to_string),
            instrumented_source: None,
            overlay,
        };
    }
    for (relative, content) in [(&target, instrumented.as_str()), (&test, tests)] {
        let destination = root.path().join(relative);
        if let Some(parent) = destination.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                overlay.supported = false;
                overlay.reason = Some(format!("failed to create instrumentation overlay: {error}"));
                break;
            }
        }
        if let Err(error) = std::fs::write(&destination, content) {
            overlay.supported = false;
            overlay.reason = Some(format!("failed to write instrumentation overlay: {error}"));
            break;
        }
    }
    let overlay_project = root.path().to_string_lossy().into_owned();
    let overlay_test = root.path().join(test).to_string_lossy().into_owned();
    PreparedAuthoritativeTest {
        _root: Some(root),
        code: tests.into(),
        project_dir: Some(overlay_project),
        source_file: Some(overlay_test),
        instrumented_source: None,
        overlay,
    }
}

fn function_key(func: &FunctionInfo) -> (String, usize) {
    (func.name.clone(), func.line)
}

fn differential_argument(param: &ParamInfo, language: &Language) -> Option<&'static str> {
    let annotation = param.type_annotation.as_deref()?.trim();
    match language {
        Language::Python => match annotation {
            "int" => Some("0"),
            "float" => Some("0.0"),
            "str" => Some("''"),
            "bool" => Some("False"),
            value if value.starts_with("list") || value.starts_with("List") => Some("[]"),
            value if value.starts_with("dict") || value.starts_with("Dict") => Some("{}"),
            _ => None,
        },
        Language::TypeScript => match annotation {
            "number" | "bigint" => Some("0"),
            "string" => Some("''"),
            "boolean" => Some("false"),
            value if value.ends_with("[]") || value.starts_with("Array<") => Some("[]"),
            value if value.starts_with("Record<") || value.starts_with("Map<") => Some("{}"),
            _ => None,
        },
    }
}

fn compatible_surface(candidate: &FunctionInfo, baseline: &FunctionInfo) -> bool {
    candidate.name == baseline.name
        && candidate.is_exported
        && baseline.is_exported
        && !candidate.is_method
        && !baseline.is_method
        && !candidate.is_nested
        && !baseline.is_nested
        && candidate.return_type == baseline.return_type
        && candidate.params.len() == baseline.params.len()
        && candidate
            .params
            .iter()
            .zip(&baseline.params)
            .all(|(left, right)| {
                left.name == right.name
                    && left.keyword_only == right.keyword_only
                    && left.type_annotation == right.type_annotation
            })
}

#[derive(Debug, Clone)]
enum DifferentialBinding {
    Positional,
    PythonKeyword(String),
}

#[derive(Debug, Clone)]
struct DifferentialArgument {
    value: ReproValue,
    binding: DifferentialBinding,
}

#[derive(Debug, Clone)]
struct DifferentialCase {
    arguments: Vec<DifferentialArgument>,
}

impl DifferentialCase {
    fn repro_arguments(&self) -> Vec<ReproValue> {
        self.arguments
            .iter()
            .map(|argument| argument.value.clone())
            .collect()
    }
}

fn differential_case_from_arguments(
    function: &FunctionInfo,
    arguments: &[ReproValue],
    language: &Language,
) -> Option<DifferentialCase> {
    let params = function
        .params
        .iter()
        .filter(|param| !param.is_variadic())
        .collect::<Vec<_>>();
    if params.len() != arguments.len() {
        return None;
    }
    Some(DifferentialCase {
        arguments: params
            .into_iter()
            .zip(arguments)
            .map(|(param, value)| DifferentialArgument {
                value: value.clone(),
                binding: if matches!(language, Language::Python) && param.keyword_only {
                    DifferentialBinding::PythonKeyword(param.name.clone())
                } else {
                    DifferentialBinding::Positional
                },
            })
            .collect(),
    })
}

fn differential_case(function: &FunctionInfo, language: &Language) -> Option<DifferentialCase> {
    let arguments = function
        .params
        .iter()
        .filter(|param| !param.is_variadic())
        .map(|param| {
            differential_argument(param, language).map(|expression| ReproValue {
                expression: expression.to_string(),
                json_value: None,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    differential_case_from_arguments(function, &arguments, language)
}

fn generated_target_source(code: &str, language: &Language) -> String {
    match language {
        Language::Python => {
            let source_literal =
                serde_json::to_string(code).expect("serializing a Rust string cannot fail");
            format!(
                r#"import sys as __court_jester_bootstrap_sys, types as __court_jester_bootstrap_types
__court_jester_bootstrap_name = ((__package__ + ".") if __package__ else "") + "__court_jester_target__"
__court_jester_bootstrap_module = __court_jester_bootstrap_types.ModuleType(__court_jester_bootstrap_name)
__court_jester_bootstrap_module.__file__ = __file__
__court_jester_bootstrap_module.__package__ = __package__
__court_jester_bootstrap_sys.modules[__court_jester_bootstrap_name] = __court_jester_bootstrap_module
exec(compile({source_literal}, __file__, "exec"), __court_jester_bootstrap_module.__dict__, __court_jester_bootstrap_module.__dict__)
globals().update(__court_jester_bootstrap_module.__dict__)
del __court_jester_bootstrap_sys, __court_jester_bootstrap_types, __court_jester_bootstrap_name, __court_jester_bootstrap_module
"#
            )
        }
        Language::TypeScript => code.to_string(),
    }
}

fn generated_typescript_target_import(
    context: &ExecutionContext,
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    coverage: &[FuzzFunctionCoverage],
) -> Option<String> {
    let can_bind_selected_surfaces = coverage
        .iter()
        .filter(|entry| entry.status == FuzzFunctionStatus::CheckedDirect)
        .all(|entry| {
            functions.iter().any(|function| {
                function.name == entry.function
                    && function.line == entry.line
                    && (function.is_exported || function.invocation_target.is_some())
            })
        });
    if !can_bind_selected_surfaces {
        return None;
    }

    let source = context.target_source.source_file.as_deref()?;
    let filename = source.file_name()?.to_str()?;
    let specifier = serde_json::to_string(&format!("./{filename}")).ok()?;
    let mut symbols = functions
        .iter()
        .filter(|function| function.is_exported && !function.is_method && !function.is_nested)
        .map(|function| function.name.as_str())
        .chain(classes.iter().map(|class| class.name.as_str()))
        .filter(|symbol| {
            let mut characters = symbol.chars();
            characters
                .next()
                .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
                && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
                && *symbol != "default"
        })
        .collect::<Vec<_>>();
    symbols.sort_unstable();
    symbols.dedup();

    let runtime_prelude = nuxt_runtime_prelude(context).unwrap_or_default();
    let mut prelude = format!(
        "{runtime_prelude}\
         const __court_jester_target = await import({specifier});\n\
         const __court_jester_exports = __court_jester_target as Record<string, any>;\n"
    );
    for symbol in symbols {
        let property = serde_json::to_string(symbol).ok()?;
        let _ = writeln!(
            prelude,
            "const {symbol} = __court_jester_exports[{property}] ?? __court_jester_exports.default;"
        );
    }
    Some(prelude)
}

fn generated_verifier_source(
    context: &ExecutionContext,
    code: &str,
    language: &Language,
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    coverage: &[FuzzFunctionCoverage],
    verifier: &str,
) -> String {
    let mut full_code = match language {
        Language::TypeScript => {
            generated_typescript_target_import(context, functions, classes, coverage)
                .unwrap_or_else(|| generated_target_source(code, language))
        }
        Language::Python => generated_target_source(code, language),
    };
    full_code.push('\n');
    full_code.push_str(verifier);
    full_code
}

fn differential_probe(
    code: &str,
    function: &FunctionInfo,
    case: &DifferentialCase,
    language: &Language,
) -> String {
    let arguments = case
        .arguments
        .iter()
        .map(|argument| match &argument.binding {
            DifferentialBinding::Positional => argument.value.expression.clone(),
            DifferentialBinding::PythonKeyword(name) => {
                format!("{name}={}", argument.value.expression)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut probe = generated_target_source(code, language);
    match language {
        Language::Python => {
            probe.push_str("\nimport contextlib as _cj_contextlib, io as _cj_io, json as _cj_json, math as _cj_math, re as _cj_re\n");
            probe.push_str("class _CJUnsupportedSnapshot(Exception):\n    pass\n");
            probe.push_str("def _cj_address_bearing(value):\n    return isinstance(value, str) and _cj_re.search(r'\\bat 0x[0-9a-fA-F]+\\b', value) is not None\n");
            probe.push_str("def _cj_stable(value, path, active):\n    if value is None or isinstance(value, bool):\n        return value\n    if isinstance(value, str):\n        if _cj_address_bearing(value):\n            raise _CJUnsupportedSnapshot(path + '_address_bearing_string')\n        return value\n    if isinstance(value, int):\n        if abs(value) > 9007199254740991:\n            raise _CJUnsupportedSnapshot(path + '_integer_outside_json_safe_range')\n        return value\n    if isinstance(value, float):\n        if not _cj_math.isfinite(value):\n            raise _CJUnsupportedSnapshot(path + '_non_finite_number')\n        return value\n    if isinstance(value, (list, tuple)):\n        identity = id(value)\n        if identity in active:\n            raise _CJUnsupportedSnapshot(path + '_cyclic_collection')\n        active.add(identity)\n        try:\n            return [_cj_stable(item, path + '_item', active) for item in value]\n        finally:\n            active.remove(identity)\n    if isinstance(value, dict):\n        if not all(isinstance(key, str) for key in value):\n            raise _CJUnsupportedSnapshot(path + '_non_string_map_key')\n        identity = id(value)\n        if identity in active:\n            raise _CJUnsupportedSnapshot(path + '_cyclic_collection')\n        active.add(identity)\n        try:\n            return {key: _cj_stable(value[key], path + '_' + key, active) for key in sorted(value)}\n        finally:\n            active.remove(identity)\n    raise _CJUnsupportedSnapshot(path + '_unsupported_type_' + type(value).__name__)\n");
            probe.push_str("_cj_stdout = _cj_io.StringIO()\n_cj_envelope = None\n");
            let _ = writeln!(probe, "try:\n    with _cj_contextlib.redirect_stdout(_cj_stdout):\n        _cj_value = {}({arguments})", function.name);
            probe.push_str("    try:\n        _cj_output = _cj_stdout.getvalue()\n        _cj_returned = _cj_stable(_cj_value, 'return', set())\n        _cj_stable(_cj_output, 'stdout', set())\n        _cj_snapshot = {'returned': _cj_returned, 'exception_type': None, 'exception_message': None, 'stdout': _cj_output}\n        _cj_envelope = {'supported': True, 'snapshot': _cj_snapshot}\n    except _CJUnsupportedSnapshot as _cj_unsupported:\n        _cj_envelope = {'supported': False, 'reason': str(_cj_unsupported)}\nexcept Exception as _cj_error:\n    try:\n        _cj_output = _cj_stdout.getvalue()\n        _cj_message = str(_cj_error)\n        _cj_stable(_cj_output, 'stdout', set())\n        _cj_stable(_cj_message, 'exception_message', set())\n        _cj_stable(list(_cj_error.args), 'exception_args', set())\n        _cj_snapshot = {'returned': None, 'exception_type': type(_cj_error).__name__, 'exception_message': _cj_message, 'stdout': _cj_output}\n        _cj_envelope = {'supported': True, 'snapshot': _cj_snapshot}\n    except _CJUnsupportedSnapshot as _cj_unsupported:\n        _cj_envelope = {'supported': False, 'reason': str(_cj_unsupported)}\nprint('__COURT_JESTER_DIFFERENTIAL_JSON__' + _cj_json.dumps(_cj_envelope, allow_nan=False, separators=(',', ':'), sort_keys=True))\n");
        }
        Language::TypeScript => {
            probe.push_str("\nclass _CJUnsupportedSnapshot extends Error {}\n");
            probe.push_str("const _cjAddressBearing = (value: string): boolean => /\\bat 0x[0-9a-f]+\\b/i.test(value);\n");
            probe.push_str("const _cjStable = (value: unknown, path: string, active: Set<object>): unknown => { if (value === null || typeof value === 'boolean') return value; if (typeof value === 'string') { if (_cjAddressBearing(value)) throw new _CJUnsupportedSnapshot(path + '_address_bearing_string'); return value; } if (typeof value === 'number') { if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) throw new _CJUnsupportedSnapshot(path + '_non_json_safe_number'); return value; } if (Array.isArray(value)) { if (active.has(value)) throw new _CJUnsupportedSnapshot(path + '_cyclic_collection'); active.add(value); try { return value.map((item) => _cjStable(item, path + '_item', active)); } finally { active.delete(value); } } if (typeof value === 'object') { const object = value as Record<string, unknown>; const prototype = Object.getPrototypeOf(object); if (prototype !== Object.prototype && prototype !== null) throw new _CJUnsupportedSnapshot(path + '_unsupported_object_prototype'); if (active.has(object)) throw new _CJUnsupportedSnapshot(path + '_cyclic_collection'); if (Object.getOwnPropertySymbols(object).length) throw new _CJUnsupportedSnapshot(path + '_symbol_key'); const descriptors = Object.getOwnPropertyDescriptors(object); active.add(object); try { const stable: Record<string, unknown> = {}; for (const key of Object.keys(object).sort()) { const descriptor = descriptors[key]; if (!descriptor || !('value' in descriptor)) throw new _CJUnsupportedSnapshot(path + '_accessor_property'); stable[key] = _cjStable(descriptor.value, path + '_' + key, active); } return stable; } finally { active.delete(object); } } throw new _CJUnsupportedSnapshot(path + '_unsupported_type_' + typeof value); };\n");
            probe.push_str("const _cjOutput: string[] = []; const _cjLog = console.log; console.log = (...values: unknown[]) => { _cjOutput.push(values.map(String).join(' ')); }; let _cjEnvelope: unknown;\n");
            let _ = writeln!(probe, "try {{ const _cjValue = {}({arguments}); try {{ const _cjStdout = _cjOutput.length ? _cjOutput.join('\\n') + '\\n' : ''; _cjEnvelope = {{ supported: true, snapshot: {{ returned: _cjStable(_cjValue, 'return', new Set()), exception_type: null, exception_message: null, stdout: _cjStable(_cjStdout, 'stdout', new Set()) }} }}; }} catch (_cjUnsupported) {{ if (!(_cjUnsupported instanceof _CJUnsupportedSnapshot)) throw _cjUnsupported; _cjEnvelope = {{ supported: false, reason: _cjUnsupported.message }}; }} }} catch (_cjError) {{ try {{ const _cjStdout = _cjOutput.length ? _cjOutput.join('\\n') + '\\n' : ''; const _cjType = _cjError instanceof Error ? _cjError.constructor.name : 'unknown'; const _cjMessageValue = _cjError instanceof Error ? _cjError.message : _cjStable(_cjError, 'exception_payload', new Set()); const _cjMessage = typeof _cjMessageValue === 'string' ? _cjMessageValue : JSON.stringify(_cjMessageValue); _cjEnvelope = {{ supported: true, snapshot: {{ returned: null, exception_type: _cjType, exception_message: _cjStable(_cjMessage, 'exception_message', new Set()), stdout: _cjStable(_cjStdout, 'stdout', new Set()) }} }}; }} catch (_cjUnsupported) {{ _cjEnvelope = {{ supported: false, reason: _cjUnsupported instanceof Error ? _cjUnsupported.message : 'exception_payload_unsupported' }}; }} }} finally {{ console.log = _cjLog; }} _cjLog('__COURT_JESTER_DIFFERENTIAL_JSON__' + JSON.stringify(_cjEnvelope));", function.name);
        }
    }
    probe
}

fn differential_snapshot(result: &ExecutionResult) -> Result<BehaviorSnapshot, String> {
    if result.timed_out {
        return Err("execution_timed_out".into());
    }
    if result.memory_error {
        return Err("execution_memory_error".into());
    }
    if result.exit_code != Some(0) {
        return Err(format!("execution_exit_{:?}", result.exit_code));
    }
    let marker = "__COURT_JESTER_DIFFERENTIAL_JSON__";
    let lines = result
        .stdout
        .lines()
        .filter(|line| line.starts_with(marker))
        .collect::<Vec<_>>();
    if lines.len() != 1 {
        return Err("snapshot_protocol_marker_count".into());
    }
    let envelope: serde_json::Value = serde_json::from_str(lines[0].trim_start_matches(marker))
        .map_err(|_| "snapshot_protocol_invalid_json".to_string())?;
    if envelope
        .get("supported")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(envelope
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("snapshot_unsupported")
            .to_string());
    }
    serde_json::from_value(
        envelope
            .get("snapshot")
            .cloned()
            .ok_or_else(|| "snapshot_protocol_missing_snapshot".to_string())?,
    )
    .map_err(|_| "snapshot_protocol_invalid_snapshot".to_string())
}

fn differential_binding_failure(snapshot: &BehaviorSnapshot, language: &Language) -> bool {
    if !matches!(language, Language::Python)
        || snapshot.exception_type.as_deref() != Some("TypeError")
    {
        return false;
    }
    let message = snapshot.exception_message.as_deref().unwrap_or_default();
    [
        "required positional argument",
        "required keyword-only argument",
        "unexpected keyword argument",
        "multiple values for argument",
        "positional arguments but",
        "takes no arguments",
    ]
    .iter()
    .any(|fragment| message.contains(fragment))
}

fn embedded_project_sources(
    project_dir: Option<&str>,
    source_file: Option<&str>,
    entry_code: &str,
    language: &Language,
) -> (String, Vec<EmbeddedSource>) {
    let source_path = source_file.map(PathBuf::from);
    let root = project_dir.map(PathBuf::from).or_else(|| {
        source_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
    });
    let relative_entry = match (&root, &source_path) {
        (Some(root), Some(path)) => path
            .strip_prefix(root)
            .ok()
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
        (_, Some(path)) => path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("entry")
            .to_string(),
        _ => match language {
            Language::Python => "entry.py".into(),
            Language::TypeScript => "entry.ts".into(),
        },
    };
    let mut paths = Vec::new();
    if let Some(root) = root.as_deref() {
        fn visit(root: &Path, directory: &Path, language: &Language, paths: &mut Vec<PathBuf>) {
            if paths.len() >= 80 {
                return;
            }
            let Ok(entries) = std::fs::read_dir(directory) else {
                return;
            };
            let mut entries = entries.flatten().collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if paths.len() >= 80 {
                    break;
                }
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if matches!(
                        name.as_ref(),
                        ".git"
                            | "node_modules"
                            | ".venv"
                            | "venv"
                            | "target"
                            | "dist"
                            | "build"
                            | "__pycache__"
                    ) {
                        continue;
                    }
                    visit(root, &path, language, paths);
                } else {
                    let extension = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default();
                    let include = match language {
                        Language::Python => extension == "py" || extension == "json",
                        Language::TypeScript => {
                            matches!(extension, "ts" | "tsx" | "js" | "mjs" | "cjs" | "json")
                        }
                    };
                    if include && path.strip_prefix(root).is_ok() {
                        paths.push(path);
                    }
                }
            }
        }
        visit(root, root, language, &mut paths);
    }
    let mut files = paths
        .into_iter()
        .filter_map(|path| {
            let root = root.as_ref()?;
            let relative_path = path
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let content = if relative_path == relative_entry {
                entry_code.to_string()
            } else {
                std::fs::read_to_string(&path).ok()?
            };
            Some(EmbeddedSource {
                relative_path,
                sha256: stable_digest(&content),
                content,
            })
        })
        .collect::<Vec<_>>();
    if !files
        .iter()
        .any(|source| source.relative_path == relative_entry)
    {
        files.push(EmbeddedSource {
            relative_path: relative_entry.clone(),
            content: entry_code.into(),
            sha256: stable_digest(entry_code),
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    (relative_entry, files)
}

#[allow(clippy::too_many_arguments)]
fn differential_finding(
    source_file: &str,
    function: &FunctionInfo,
    arguments: Vec<ReproValue>,
    candidate_snapshot: &BehaviorSnapshot,
    baseline_snapshot: &BehaviorSnapshot,
    language: &Language,
    relative_entry: String,
    base_files: Vec<EmbeddedSource>,
    candidate_files: Vec<EmbeddedSource>,
) -> VerificationFinding {
    let expectation = ReplayExpectation {
        severity: FindingSeverity::BehavioralRegression,
        oracle_kind: OracleKind::Differential,
        category: FindingCategory::Differential,
    };
    let repro = StructuredRepro {
        kind: ReproKind::Differential,
        function: Some(function.name.clone()),
        arguments: arguments.clone(),
        input_text: None,
        case_label: Some("base_candidate_snapshot".into()),
        snippet: "Differential replay materializes both embedded trees, executes this stored case in separate processes, and compares normalized snapshots.".into(),
        command: None,
        expectation,
        differential: Some(DifferentialRepro {
            relative_entry,
            base_tree_sha256: tree_digest(&base_files),
            candidate_tree_sha256: tree_digest(&candidate_files),
            base_files,
            candidate_files,
            dependency_contract: DependencyContract { language: *language, runtime_identity: "local-trusted".into(), lockfiles: Vec::new(), third_party_modules: Vec::new() },
        }),
    };
    VerificationFinding {
        id: format!(
            "differential:{}:1",
            function
                .name
                .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ),
        severity: FindingSeverity::BehavioralRegression,
        occurrences: 1,
        sample_inputs: vec![ReproCase {
            arguments: arguments.clone(),
            input_text: None,
        }],
        confidence: FindingConfidence::Low,
        category: FindingCategory::Differential,
        error_type: Some("behavioral_regression".into()),
        message: format!("Differential regression in {}", function.name),
        location: FindingLocation {
            source_file: source_file.into(),
            function: function.name.clone(),
            line: function.line,
            invocation_path: InvocationPath::Direct,
        },
        oracle: OracleInfo {
            id: "differential".into(),
            kind: OracleKind::Differential,
            provenance: OracleProvenance::ObservedCall,
            confidence: FindingConfidence::Low,
            expected: Some(serde_json::to_string(baseline_snapshot).unwrap_or_default()),
            actual: Some(serde_json::to_string(candidate_snapshot).unwrap_or_default()),
        },
        input_classification: InputClassification::Valid,
        repro,
        minimization: MinimizationInfo {
            status: MinimizationStatus::NotNeeded,
            attempts: 0,
            original: ReproCase {
                arguments,
                input_text: None,
            },
            minimized: None,
        },
        launch_context: None,
        classification: None,
        suggestion: Some(
            "add an authoritative fixture or test if this divergence is intentional".into(),
        ),
        suppressed: false,
    }
}

fn coverage_counts(entries: &[FuzzFunctionCoverage]) -> serde_json::Value {
    let mut counts = serde_json::Map::new();
    for status in [
        FuzzFunctionStatus::CheckedDirect,
        FuzzFunctionStatus::ReachedDirect,
        FuzzFunctionStatus::ReachedViaFactory,
        FuzzFunctionStatus::ReachedViaAuthoritativeTest,
        FuzzFunctionStatus::CheckedViaFactory,
        FuzzFunctionStatus::CheckedViaCaller,
        FuzzFunctionStatus::CheckedViaAuthoritativeTest,
        FuzzFunctionStatus::SkippedNoFuzzableSurface,
        FuzzFunctionStatus::SkippedUnsupportedType,
        FuzzFunctionStatus::SkippedInternalHelper,
        FuzzFunctionStatus::SkippedMethod,
        FuzzFunctionStatus::SkippedNested,
        FuzzFunctionStatus::SkippedPrivateName,
        FuzzFunctionStatus::SkippedDiffFiltered,
        FuzzFunctionStatus::BlockedModuleLoad,
    ] {
        let key = serde_json::to_value(&status)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "unknown".into());
        let count = entries
            .iter()
            .filter(|entry| entry.status == status)
            .count();
        counts.insert(key, serde_json::Value::from(count));
    }
    serde_json::Value::Object(counts)
}

fn finalize_fuzz_coverage(
    analysis_functions: &[FunctionInfo],
    allowed_functions: &[FunctionInfo],
    planned_coverage: &[FuzzFunctionCoverage],
    module_load_blocked: bool,
) -> Vec<FuzzFunctionCoverage> {
    let allowed: HashSet<(String, usize)> = allowed_functions.iter().map(function_key).collect();
    let mut planned: HashMap<(String, usize), FuzzFunctionCoverage> = planned_coverage
        .iter()
        .cloned()
        .map(|entry| ((entry.function.clone(), entry.line), entry))
        .collect();

    let mut coverage = Vec::with_capacity(analysis_functions.len());
    for func in analysis_functions {
        let key = function_key(func);
        let mut entry = if !allowed.contains(&key) {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedDiffFiltered,
                Some("excluded by diff scoping".into()),
            )
        } else if func.is_method && func.invocation_target.is_none() {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedMethod,
                Some("methods are not fuzzed directly".into()),
            )
        } else if func.is_nested && func.invocation_target.is_none() {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedNested,
                Some(
                    "nested functions are exercised via their parent factory when possible".into(),
                ),
            )
        } else if func.name.starts_with('_') {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedPrivateName,
                Some("underscore-prefixed helpers are skipped".into()),
            )
        } else if let Some(mut planned_entry) = planned.remove(&key) {
            if module_load_blocked
                && matches!(
                    planned_entry.status,
                    FuzzFunctionStatus::CheckedDirect | FuzzFunctionStatus::CheckedViaFactory
                )
            {
                planned_entry.status = FuzzFunctionStatus::BlockedModuleLoad;
                planned_entry.reason =
                    Some("module load failed before the fuzz harness ran".into());
            }
            planned_entry
        } else {
            coverage_entry_for_verify(
                func,
                FuzzFunctionStatus::SkippedUnsupportedType,
                Some("function was not selected for fuzzing".into()),
            )
        };
        // Nested closures can be API-visible through a factory, but the exact
        // authoritative-test contract only requires directly exported surfaces.
        entry.required = func.is_exported && !func.is_nested;
        coverage.push(entry);
    }

    let mut synthetic_entries: Vec<_> = planned.into_values().collect();
    synthetic_entries.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.function.cmp(&right.function))
    });
    coverage.extend(synthetic_entries);

    coverage
}

fn coverage_entry_for_verify(
    func: &FunctionInfo,
    status: FuzzFunctionStatus,
    reason: Option<String>,
) -> FuzzFunctionCoverage {
    FuzzFunctionCoverage {
        function: func.name.clone(),

        line: func.line,
        end_line: func.end_line,
        status,
        required: func.is_exported,
        invocation_path: InvocationPath::Direct,
        is_exported: func.is_exported,
        reason,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuppressionsFile {
    #[serde(default)]
    rules: Vec<SuppressionRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuppressionRule {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    function: Option<String>,
    #[serde(default)]
    severity: Option<FindingSeverity>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct SuppressionContext<'a> {
    source_file: Option<&'a str>,
    stage: &'a str,
    function: Option<&'a str>,
    severity: Option<FindingSeverity>,
    error_type: Option<&'a str>,
    reason: Option<&'a str>,
}

fn parse_suppressions(raw: Option<&str>) -> Result<SuppressionsFile, String> {
    let Some(raw) = raw else {
        return Ok(SuppressionsFile::default());
    };
    let file: SuppressionsFile = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    for (index, rule) in file.rules.iter().enumerate() {
        let selectors = [
            rule.path.as_deref(),
            rule.stage.as_deref(),
            rule.function.as_deref(),
            rule.error_type.as_deref(),
            rule.reason.as_deref(),
        ];
        if selectors.iter().all(Option::is_none) && rule.severity.is_none() {
            return Err(format!("rules[{index}] requires at least one selector"));
        }
        if selectors
            .iter()
            .flatten()
            .any(|value| value.trim().is_empty())
        {
            return Err(format!("rules[{index}] selectors must not be empty"));
        }
        if rule
            .stage
            .as_deref()
            .is_some_and(|stage| !matches!(stage, "execute" | "complexity" | "portability"))
        {
            return Err(format!(
                "rules[{index}].stage must be execute, complexity, or portability"
            ));
        }
    }
    Ok(file)
}

/// Validate suppression data using the same schema and selectors as verification.
pub fn validate_suppressions(raw: &str) -> Result<(), String> {
    parse_suppressions(Some(raw)).map(|_| ())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn suppression_matches(rule: &SuppressionRule, ctx: SuppressionContext<'_>) -> bool {
    if let Some(rule_path) = rule.path.as_deref() {
        let Some(source_file) = ctx.source_file else {
            return false;
        };
        if !normalize_path(source_file).ends_with(normalize_path(rule_path).as_str()) {
            return false;
        }
    }
    if let Some(stage) = rule.stage.as_deref() {
        if stage != ctx.stage {
            return false;
        }
    }
    if let Some(function) = rule.function.as_deref() {
        if Some(function) != ctx.function {
            return false;
        }
    }
    if let Some(severity) = rule.severity {
        if Some(severity) != ctx.severity {
            return false;
        }
    }
    if let Some(error_type) = rule.error_type.as_deref() {
        if Some(error_type) != ctx.error_type {
            return false;
        }
    }
    if let Some(reason) = rule.reason.as_deref() {
        if Some(reason) != ctx.reason {
            return false;
        }
    }
    true
}

fn split_findings(
    findings: Vec<VerificationFinding>,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
) -> (Vec<VerificationFinding>, Vec<VerificationFinding>) {
    let mut active = Vec::new();
    let mut suppressed = Vec::new();
    for mut finding in findings {
        let ctx = SuppressionContext {
            source_file,
            stage: "execute",
            function: Some(finding.location.function.as_str()),
            severity: Some(finding.severity),
            error_type: finding.error_type.as_deref(),
            reason: finding.classification.as_deref(),
        };
        if suppressions
            .rules
            .iter()
            .any(|rule| suppression_matches(rule, ctx))
        {
            finding.suppressed = true;
            suppressed.push(finding);
        } else {
            active.push(finding);
        }
    }
    (active, suppressed)
}

fn split_complexity_violations(
    violations: Vec<ComplexityViolation>,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
    code: &str,
    language: &Language,
) -> (
    Vec<ComplexityViolation>,
    Vec<ComplexityViolation>,
    Vec<String>,
) {
    let mut active = Vec::new();
    let mut suppressed = Vec::new();
    let mut source_directive_functions = Vec::new();

    for violation in violations {
        if analyze::source_directive_suppresses_complexity(code, language, violation.line) {
            source_directive_functions.push(violation.function.clone());
            suppressed.push(violation);
            continue;
        }

        let ctx = SuppressionContext {
            source_file,
            stage: "complexity",
            function: Some(violation.function.as_str()),
            severity: None,
            error_type: None,
            reason: None,
        };
        if suppressions
            .rules
            .iter()
            .any(|rule| suppression_matches(rule, ctx))
        {
            suppressed.push(violation);
        } else {
            active.push(violation);
        }
    }

    source_directive_functions.sort();
    source_directive_functions.dedup();

    (active, suppressed, source_directive_functions)
}

fn portability_reason(stderr: &str) -> &'static str {
    if stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
        || stderr.contains("needs an import attribute of \"type: json\"")
    {
        "err_import_attribute_missing"
    } else if stderr.contains("Cannot find module 'bun'")
        || stderr.contains("Cannot find package 'bun'")
        || stderr.contains("Bun is not defined")
    {
        "bun_runtime_dependency"
    } else if stderr.contains("ERR_MODULE_NOT_FOUND") {
        "err_module_not_found"
    } else {
        "unknown_portability_error"
    }
}

fn collect_portability_imports(stderr: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for prefix in ["Cannot find module '", "Cannot find package '"] {
        for section in stderr.split(prefix).skip(1) {
            if let Some((candidate, _)) = section.split_once('\'') {
                let candidate = candidate.trim();
                if !candidate.is_empty() && !imports.iter().any(|item| item == candidate) {
                    imports.push(candidate.to_string());
                }
            }
        }
    }
    imports
}

fn portability_fix_hint(reason: &str) -> &'static str {
    match reason {
        "err_module_not_found" => {
            "Add explicit Node ESM file extensions for relative imports, or rely on the repo-native runtime when the repo is intentionally Bun-specific."
        }
        "err_import_attribute_missing" => {
            "Add the required import attribute for JSON modules, for example `with { type: \"json\" }` in Node ESM."
        }
        "bun_runtime_dependency" => {
            "The file depends on Bun-only globals or packages. Keep portability advisory-only, or run the repo-native runtime for behavior checks."
        }
        _ => "Review the Node stderr to see which import or runtime assumption blocks strict Node execution.",
    }
}

fn build_portability_detail(
    repo_runtime: &str,
    node_result: &ExecutionResult,
    repo_result: &ExecutionResult,
    suppressions: &SuppressionsFile,
    source_file: Option<&str>,
    suppression_source: Option<&str>,
) -> (serde_json::Value, bool) {
    let reason = portability_reason(&node_result.stderr).to_string();
    let failing_imports = collect_portability_imports(&node_result.stderr);
    let behavior_executed = repo_result.exit_code == Some(0)
        && !repo_result.timed_out
        && !repo_result.memory_error
        && parse_fuzz_outcomes(&repo_result.stdout)
            .iter()
            .any(|outcome| {
                matches!(
                    outcome.status,
                    FuzzOutcomeStatus::Passed | FuzzOutcomeStatus::Crashed
                )
            });
    let suppressed = suppressions.rules.iter().any(|rule| {
        suppression_matches(
            rule,
            SuppressionContext {
                source_file,
                stage: "portability",
                function: None,
                severity: None,
                error_type: None,
                reason: Some(reason.as_str()),
            },
        )
    });

    (
        serde_json::json!({
            "reason": reason,
            "failing_imports": failing_imports,
            "fix_hint": portability_fix_hint(portability_reason(&node_result.stderr)),
            "suppressed": suppressed,
            "suppression_source": suppression_source,
            "repo_runtime": repo_runtime,
            "behavior_executed": behavior_executed,
            "node_result": serde_json::to_value(node_result).unwrap(),
            "repo_result": serde_json::to_value(repo_result).unwrap(),
        }),
        suppressed,
    )
}

#[derive(Debug, Clone)]
struct ObservedArg {
    code: String,
    literal_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct ObservedCall {
    function: String,
    args: Vec<ObservedArg>,
    source_label: String,
}
fn parser_for_source_mode(source_mode: SourceMode) -> Option<Parser> {
    let mut parser = Parser::new();
    let grammar = match source_mode {
        SourceMode::Python => tree_sitter_python::LANGUAGE.into(),
        SourceMode::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceMode::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser.set_language(&grammar).ok()?;
    Some(parser)
}

fn node_text(node: &tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn primitive_literal_value(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<Option<serde_json::Value>> {
    let code = node_text(node, source);
    match language {
        Language::TypeScript => match node.kind() {
            "number" => Some(
                code.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number),
            ),
            "true" => Some(Some(serde_json::Value::Bool(true))),
            "false" => Some(Some(serde_json::Value::Bool(false))),
            "null" => Some(Some(serde_json::Value::Null)),
            "undefined" | "string" => Some(None),
            _ => None,
        },
        Language::Python => match node.kind() {
            "integer" => Some(code.parse::<i64>().ok().map(serde_json::Value::from)),
            "float" => Some(
                code.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number),
            ),
            "true" => Some(Some(serde_json::Value::Bool(true))),
            "false" => Some(Some(serde_json::Value::Bool(false))),
            "none" | "string" => Some(None),
            _ => None,
        },
    }
}

fn is_literal_like_arg(node: &tree_sitter::Node, language: &Language, source: &[u8]) -> bool {
    if primitive_literal_value(node, language, source).is_some() {
        return true;
    }

    match (language, node.kind()) {
        (Language::TypeScript, "array") | (Language::Python, "list" | "tuple" | "set") => {
            let mut cursor = node.walk();
            let all_literal = node.named_children(&mut cursor).all(|child| {
                !matches!(
                    child.kind(),
                    "spread_element" | "list_splat" | "dictionary_splat"
                ) && is_literal_like_arg(&child, language, source)
            });
            all_literal
        }
        (Language::TypeScript, "object") | (Language::Python, "dictionary") => {
            let mut cursor = node.walk();
            let all_literal = node.named_children(&mut cursor).all(|child| {
                if matches!(
                    child.kind(),
                    "spread_element" | "list_splat" | "dictionary_splat"
                ) {
                    return false;
                }
                if child.kind() == "pair" {
                    return child
                        .child_by_field_name("value")
                        .or_else(|| child.named_child(child.named_child_count().saturating_sub(1)))
                        .is_some_and(|value| is_literal_like_arg(&value, language, source));
                }
                false
            });
            all_literal
        }
        (Language::TypeScript, "unary_expression") | (Language::Python, "unary_operator") => {
            let text = node_text(node, source);
            matches!(text.trim().chars().next(), Some('-' | '+'))
                && node.named_child_count() == 1
                && node.named_child(0).is_some_and(|child| {
                    primitive_literal_value(&child, language, source).is_some()
                })
        }
        _ => false,
    }
}

fn parse_literal_arg(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<ObservedArg> {
    let code = node_text(node, source);
    let literal_value = primitive_literal_value(node, language, source)
        .or_else(|| is_literal_like_arg(node, language, source).then_some(None))?;
    Some(ObservedArg {
        code,
        literal_value,
    })
}

fn extract_literal_args(
    arguments_node: tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<Vec<ObservedArg>> {
    let mut args = Vec::new();
    let mut cursor = arguments_node.walk();
    for child in arguments_node.named_children(&mut cursor) {
        match (language, child.kind()) {
            (Language::Python, "keyword_argument")
            | (Language::Python, "list_splat")
            | (Language::TypeScript, "spread_element") => return None,
            _ => {}
        }
        args.push(parse_literal_arg(&child, language, source)?);
    }
    Some(args)
}

fn callee_function_name(
    callee: tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<String> {
    if callee.kind() == "identifier" {
        return Some(node_text(&callee, source));
    }

    match language {
        Language::TypeScript if callee.kind() == "member_expression" => callee
            .child_by_field_name("property")
            .filter(|property| matches!(property.kind(), "property_identifier" | "identifier"))
            .map(|property| node_text(&property, source)),
        Language::Python if callee.kind() == "attribute" => callee
            .child_by_field_name("attribute")
            .filter(|attribute| attribute.kind() == "identifier")
            .map(|attribute| node_text(&attribute, source)),
        _ => None,
    }
}

fn collect_observed_calls_recursive(
    node: tree_sitter::Node,
    language: &Language,
    source: &[u8],
    function_names: &HashSet<String>,
    source_label: &str,
    out: &mut Vec<ObservedCall>,
) {
    let is_call = matches!(
        (language, node.kind()),
        (Language::Python, "call") | (Language::TypeScript, "call_expression")
    );
    if is_call {
        let callee = node.child_by_field_name("function");
        let arguments = node.child_by_field_name("arguments");
        if let (Some(callee), Some(arguments)) = (callee, arguments) {
            if let Some(name) = callee_function_name(callee, language, source) {
                if function_names.contains(&name) {
                    if let Some(args) = extract_literal_args(arguments, language, source) {
                        out.push(ObservedCall {
                            function: name,
                            args,
                            source_label: source_label.to_string(),
                        });
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_observed_calls_recursive(
            child,
            language,
            source,
            function_names,
            source_label,
            out,
        );
    }
}

fn collect_observed_calls(
    code: &str,
    language: &Language,
    source_mode: SourceMode,
    function_names: &HashSet<String>,
    source_label: &str,
) -> Vec<ObservedCall> {
    let Some(mut parser) = parser_for_source_mode(source_mode) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(code, None) else {
        return Vec::new();
    };
    let mut observed = Vec::new();
    collect_observed_calls_recursive(
        tree.root_node(),
        language,
        code.as_bytes(),
        function_names,
        source_label,
        &mut observed,
    );
    observed
}

fn discover_seed_files(source_file: &str, language: &Language) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) else {
        return Vec::new();
    };
    let Some(dir) = source_path.parent() else {
        return Vec::new();
    };
    let parent = dir.parent().unwrap_or(dir);
    let mut candidates = Vec::new();
    match language {
        Language::TypeScript => {
            for name in [format!("{stem}.test.ts"), format!("{stem}.spec.ts")] {
                candidates.push(dir.join(&name));
                candidates.push(dir.join("__tests__").join(&name));
                candidates.push(parent.join("tests").join(&name));
                candidates.push(parent.join("__tests__").join(&name));
            }
        }
        Language::Python => {
            for name in [format!("test_{stem}.py"), format!("{stem}_test.py")] {
                candidates.push(dir.join(&name));
                candidates.push(dir.join("tests").join(&name));
                candidates.push(parent.join("tests").join(&name));
            }
        }
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn json_value_to_literal(value: &serde_json::Value, language: &Language) -> String {
    match language {
        Language::TypeScript => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
        Language::Python => match value {
            serde_json::Value::Null => "None".into(),
            serde_json::Value::Bool(value) => {
                if *value {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::String(value) => {
                serde_json::to_string(value).unwrap_or_else(|_| "''".into())
            }
            serde_json::Value::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|item| json_value_to_literal(item, language))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            serde_json::Value::Object(values) => format!(
                "{{{}}}",
                values
                    .iter()
                    .map(|(key, item)| format!(
                        "{}: {}",
                        serde_json::to_string(key).unwrap_or_else(|_| "''".into()),
                        json_value_to_literal(item, language)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    }
}

fn candidate_fixture_function_name(
    source_file: &str,
    function_names: &HashSet<String>,
) -> Option<String> {
    let stem = Path::new(source_file)
        .file_stem()
        .and_then(|value| value.to_str())?;
    if function_names.contains(stem) {
        return Some(stem.to_string());
    }
    if function_names.len() == 1 {
        return function_names.iter().next().cloned();
    }
    None
}

fn fixture_json_paths(source_file: &str, project_dir: Option<&str>) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) else {
        return Vec::new();
    };
    let Some(source_dir) = source_path.parent() else {
        return Vec::new();
    };
    let parent = source_dir.parent().unwrap_or(source_dir);
    let filename = format!("{stem}.json");
    let mut candidates = vec![
        source_dir.join(&filename),
        source_dir.join("fixtures").join(&filename),
        source_dir.join("examples").join(&filename),
        source_dir.join("tests").join(&filename),
        parent.join("fixtures").join(&filename),
        parent.join("examples").join(&filename),
        parent.join("tests").join(&filename),
    ];

    if let Some(project_dir) = project_dir {
        let root = Path::new(project_dir);
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if !is_ignored_project_seed_dir(&path) {
                        stack.push(path);
                    }
                    continue;
                }
                if path.file_name().and_then(|value| value.to_str()) == Some(filename.as_str()) {
                    candidates.push(path);
                }
            }
        }
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| path.metadata().map(|meta| meta.len()).unwrap_or(0) <= 512 * 1024)
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn display_fixture_path(path: &Path, project_dir: Option<&str>) -> PathBuf {
    let Some(project_dir) = project_dir else {
        return path.to_path_buf();
    };
    let root = Path::new(project_dir);
    let Some(canonical_root) = std::fs::canonicalize(root).ok() else {
        return path.to_path_buf();
    };
    let Some(relative) = path.strip_prefix(&canonical_root).ok() else {
        return path.to_path_buf();
    };
    root.join(relative)
}
fn display_seed_source_label(
    label: &str,
    source_file: Option<&str>,
    project_dir: Option<&str>,
) -> String {
    let path = Path::new(label);
    if !path.is_absolute() {
        return label.to_string();
    }
    let Some(root) = project_dir.map(PathBuf::from).or_else(|| {
        source_file.and_then(|source| {
            Path::new(source)
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
    }) else {
        return label.to_string();
    };
    let display_root = if root.is_absolute() {
        root
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(&root)
    } else {
        root
    };
    let Ok(canonical_root) = std::fs::canonicalize(&display_root) else {
        return label.to_string();
    };
    let Ok(relative) = path.strip_prefix(canonical_root) else {
        return label.to_string();
    };
    display_root.join(relative).to_string_lossy().into_owned()
}

#[derive(Debug, Clone)]
struct JsonFixtureRow {
    function: String,
    args: Vec<serde_json::Value>,
    expected: serde_json::Value,
    source_file: String,
    line: usize,
}

fn json_fixture_rows(
    source_file: &str,
    project_dir: Option<&str>,
    function_names: &HashSet<String>,
) -> Vec<JsonFixtureRow> {
    let Some(function) = candidate_fixture_function_name(source_file, function_names) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    let mut seen_rows = HashSet::new();
    for path in fixture_json_paths(source_file, project_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let serde_json::Value::Array(mut row) = value else {
                continue;
            };
            if row.len() != 2 || !row[0].is_array() {
                continue;
            }
            let expected = row.pop().unwrap_or(serde_json::Value::Null);
            let Some(serde_json::Value::Array(args)) = row.pop() else {
                continue;
            };
            let row_key = format!("{}:{}", function, fixture_row_key(&args, &expected));
            if !seen_rows.insert(row_key) {
                continue;
            }
            rows.push(JsonFixtureRow {
                function: function.clone(),
                args,
                expected,
                source_file: display_fixture_path(&path, project_dir)
                    .to_string_lossy()
                    .to_string(),
                line: line_index + 1,
            });
        }
    }
    rows
}

fn json_value_is_primitive_sortable(value: &serde_json::Value) -> bool {
    value.is_number() || value.is_string() || value.is_boolean()
}

fn json_array_is_sorted_primitive(values: &[serde_json::Value]) -> bool {
    if values.is_empty() {
        return true;
    }
    if values.iter().all(serde_json::Value::is_number) {
        let nums = values
            .iter()
            .filter_map(serde_json::Value::as_f64)
            .collect::<Vec<_>>();
        return nums.len() == values.len()
            && nums
                .windows(2)
                .all(|pair| pair[0].is_finite() && pair[1].is_finite() && pair[0] <= pair[1]);
    }
    if values.iter().all(serde_json::Value::is_string) {
        let strings = values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        return strings.windows(2).all(|pair| pair[0] <= pair[1]);
    }
    if values
        .iter()
        .any(|value| !json_value_is_primitive_sortable(value))
    {
        return false;
    }
    let mut rendered = values
        .iter()
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>();
    let original = rendered.clone();
    rendered.sort();
    original == rendered
}

fn json_multiset_key(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn json_arrays_same_multiset(left: &[serde_json::Value], right: &[serde_json::Value]) -> bool {
    let mut counts: HashMap<String, isize> = HashMap::new();
    for value in left {
        *counts.entry(json_multiset_key(value)).or_default() += 1;
    }
    for value in right {
        let entry = counts.entry(json_multiset_key(value)).or_default();
        *entry -= 1;
    }
    counts.values().all(|count| *count == 0)
}

fn json_sequence_is_palindrome(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => {
            values.len() > 1 && values.iter().eq(values.iter().rev())
        }
        serde_json::Value::String(value) => {
            value.len() > 1 && value.chars().eq(value.chars().rev())
        }
        _ => false,
    }
}

const MIN_FIXTURE_PROPERTY_SUPPORT: usize = 2;

fn fixture_row_key(args: &[serde_json::Value], expected: &serde_json::Value) -> String {
    let args = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
    let expected = serde_json::to_string(expected).unwrap_or_else(|_| "null".into());
    format!("{args}=>{expected}")
}

fn fixture_property_support_count<F>(
    rows: &[(Vec<serde_json::Value>, serde_json::Value)],
    predicate: F,
) -> usize
where
    F: Fn(&[serde_json::Value], &serde_json::Value) -> bool,
{
    let mut seen = HashSet::new();
    for (args, expected) in rows {
        if predicate(args, expected) {
            seen.insert(fixture_row_key(args, expected));
        }
    }
    seen.len()
}

fn fixture_property_has_support<F>(
    rows: &[(Vec<serde_json::Value>, serde_json::Value)],
    predicate: F,
) -> bool
where
    F: Fn(&[serde_json::Value], &serde_json::Value) -> bool,
{
    fixture_property_support_count(rows, predicate) >= MIN_FIXTURE_PROPERTY_SUPPORT
}

fn infer_fixture_properties(rows: &[JsonFixtureRow]) -> HashMap<String, Vec<String>> {
    let mut grouped: HashMap<String, Vec<(Vec<serde_json::Value>, serde_json::Value)>> =
        HashMap::new();
    for row in rows {
        grouped
            .entry(row.function.clone())
            .or_default()
            .push((row.args.clone(), row.expected.clone()));
    }

    let mut inferred = HashMap::new();
    for (function, rows) in grouped {
        if rows.is_empty() {
            continue;
        }
        let mut properties = Vec::new();
        let sorted_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            expected
                .as_array()
                .is_some_and(|values| json_array_is_sorted_primitive(values))
        };
        let nontrivial_sorted_output =
            |args: &[serde_json::Value], expected: &serde_json::Value| {
                sorted_output(args, expected)
                    && expected.as_array().is_some_and(|values| values.len() >= 2)
            };
        if rows
            .iter()
            .all(|(args, expected)| sorted_output(args, expected))
            && fixture_property_has_support(&rows, nontrivial_sorted_output)
        {
            properties.push("sorted".to_string());
        }

        let permutation_output = |args: &[serde_json::Value], expected: &serde_json::Value| {
            let Some(input) = args.first().and_then(|value| value.as_array()) else {
                return false;
            };
            let Some(output) = expected.as_array() else {
                return false;
            };
            json_arrays_same_multiset(input, output)
        };
        let nontrivial_permutation_output =
            |args: &[serde_json::Value], expected: &serde_json::Value| {
                let Some(input) = args.first().and_then(|value| value.as_array()) else {
                    return false;
                };
                let Some(output) = expected.as_array() else {
                    return false;
                };
                input.len() >= 2 && output.len() >= 2 && json_arrays_same_multiset(input, output)
            };
        if rows
            .iter()
            .all(|(args, expected)| permutation_output(args, expected))
            && fixture_property_has_support(&rows, nontrivial_permutation_output)
        {
            properties.push("permutation".to_string());
        }

        let nonnegative_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            expected
                .as_f64()
                .is_some_and(|value| value >= 0.0 && value.is_finite())
        };
        if rows
            .iter()
            .all(|(args, expected)| nonnegative_output(args, expected))
            && fixture_property_has_support(&rows, nonnegative_output)
            && rows.iter().any(|(_, expected)| {
                expected
                    .as_f64()
                    .is_some_and(|value| value > 0.0 && value.is_finite())
            })
        {
            properties.push("nonneg".to_string());
        }

        let palindrome_output = |_: &[serde_json::Value], expected: &serde_json::Value| {
            json_sequence_is_palindrome(expected)
        };
        if function.to_lowercase().contains("palindrome")
            && rows
                .iter()
                .all(|(args, expected)| palindrome_output(args, expected))
            && fixture_property_has_support(&rows, palindrome_output)
        {
            properties.push("palindrome".to_string());
        }
        if !properties.is_empty() {
            inferred.insert(function, properties);
        }
    }
    inferred
}

fn apply_inferred_properties(
    functions: &mut [FunctionInfo],
    inferred: &HashMap<String, Vec<String>>,
) {
    for function in functions {
        let Some(properties) = inferred.get(&function.name) else {
            continue;
        };
        for property in properties {
            if !function
                .declared_properties
                .iter()
                .any(|existing| existing == property)
            {
                function.declared_properties.push(property.clone());
            }
        }
    }
}

const QUERY_NESTED_BRACKETS_PROPERTY: &str = "query_nested_brackets";
const SAME_VALUE_ZERO_PROPERTY: &str = "same_value_zero";
const PEP440_VERSION_ORDERING_PROPERTY: &str = "pep440_version_ordering";
const PEP440_SPECIFIER_MEMBERSHIP_PROPERTY: &str = "pep440_specifier_membership";
const PEP440_FILTER_PRERELEASE_PROPERTY: &str = "pep440_filter_prerelease";
const COOKIE_VALUE_QUOTE_PROPERTY: &str = "cookie_value_quote";
const COOKIE_HEADER_QUOTE_PROPERTY: &str = "cookie_header_quote";
const HTTP_REQUEST_METADATA_PROPERTY: &str = "http_request_metadata";
const HTTP_RESPONSE_HELPERS_PROPERTY: &str = "http_response_helpers";
const HTTP_STATIC_FILE_MIDDLEWARE_PROPERTY: &str = "http_static_file_middleware";

fn push_inferred_property(
    inferred: &mut HashMap<String, Vec<String>>,
    function: &str,
    property: &str,
) {
    let properties = inferred.entry(function.to_string()).or_default();
    if !properties.iter().any(|existing| existing == property) {
        properties.push(property.to_string());
    }
}

fn ts_annotation_is_string_like(type_annotation: Option<&str>) -> bool {
    type_annotation
        .map(str::trim)
        .is_some_and(|value| value == "string" || value == "str")
}

fn ts_annotation_is_structured_or_unknown(type_annotation: Option<&str>) -> bool {
    let Some(value) = type_annotation.map(str::trim) else {
        return true;
    };
    matches!(value, "unknown" | "any" | "object")
        || value.starts_with("Record<")
        || value.starts_with("dict[")
        || value.starts_with("Dict[")
        || value.starts_with('{')
}

fn ts_annotation_is_mapping_like(type_annotation: Option<&str>) -> bool {
    let Some(value) = type_annotation.map(str::trim) else {
        return false;
    };
    value.starts_with("Record<")
        || value.starts_with("dict[")
        || value.starts_with("Dict[")
        || value.starts_with('{')
}

fn function_can_accept_query_nested_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    let query_name_context = lower.contains("query") || lower.contains("urlencoded");
    if !query_name_context {
        return false;
    }

    let first_param_type = func
        .params
        .iter()
        .find(|param| !param.is_variadic())
        .and_then(|param| param.type_annotation.as_deref());
    let return_type = func.return_type.as_deref();

    let parse_like = (lower.contains("parse") || lower.contains("decode"))
        && ts_annotation_is_string_like(first_param_type)
        && ts_annotation_is_structured_or_unknown(return_type);
    let stringify_like = [
        "stringify",
        "serialize",
        "serialise",
        "canonical",
        "canonicalize",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
        && ts_annotation_is_mapping_like(first_param_type)
        && ts_annotation_is_string_like(return_type);

    parse_like || stringify_like
}

fn text_suggests_query_nested_brackets_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let query_context = lower.contains("query")
        || lower.contains("query-string")
        || lower.contains("query string")
        || lower.contains("qs.")
        || lower.contains("urlencoded")
        || lower.contains("url-encoded")
        || lower.contains("urlsearchparams")
        || lower.contains("bracket notation")
        || lower.contains("form parsing")
        || lower.contains("form parser")
        || lower.contains("form body")
        || lower.contains("body parsing")
        || lower.contains("body parser");
    let nested_context = [
        "bracket notation",
        "extended parsing",
        "extended parser",
        "extended urlencoded",
        "nested query",
        "nested queries",
        "nested object",
        "nested objects",
        "nested array",
        "nested arrays",
        "nested form",
        "nested forms",
        "nested collection",
        "nested collections",
        "object-plus-array",
        "larger arrays",
        "arrays of objects",
        "duplicate nested",
        "[] suffix",
        "[tags][]",
        "filter[",
        "%5b",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    query_context && nested_context
}

fn function_can_accept_http_request_metadata_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("request") && (lower.contains("decorate") || lower.contains("metadata"))) {
        return false;
    }
    func.params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| {
            let lower = annotation.to_lowercase();
            lower.contains("request") || lower.contains("req")
        })
        .unwrap_or(false)
}

fn text_suggests_http_request_metadata_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let request_context = lower.contains("request metadata")
        || lower.contains("request introspection")
        || lower.contains("request decoration")
        || lower.contains("request helpers")
        || lower.contains("req.get")
        || lower.contains("req.header")
        || lower.contains("req.xhr");
    let behavior_context = [
        "header lookup",
        "xhr detection",
        "trust proxy",
        "forwarded-proto",
        "x-forwarded-proto",
        "protocol",
        "secure",
        "query-parser request decoration",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    request_context && behavior_context
}

fn function_can_accept_http_response_helpers_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("response") && (lower.contains("decorate") || lower.contains("helper"))) {
        return false;
    }
    func.params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| annotation.to_lowercase().contains("response"))
        .unwrap_or(false)
}

fn text_suggests_http_response_helpers_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let response_context = lower.contains("response header")
        || lower.contains("response helper")
        || lower.contains("response metadata")
        || lower.contains("status helpers")
        || lower.contains("sendstatus")
        || lower.contains("res.location")
        || lower.contains("res.vary");
    let behavior_context = [
        "location",
        "link header",
        "vary",
        "sendstatus",
        "status helper",
        "empty response body",
        "header composition",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    response_context && behavior_context
}

fn function_can_accept_http_static_file_context(func: &FunctionInfo) -> bool {
    let lower = func.name.to_lowercase();
    if !(lower.contains("static") && (lower.contains("middleware") || lower.contains("serve"))) {
        return false;
    }
    let first_param_is_root = func
        .params
        .first()
        .and_then(|param| param.type_annotation.as_deref())
        .map(|annotation| annotation.trim() == "string" || annotation.trim() == "str")
        .unwrap_or(false);
    let returns_handler = func
        .return_type
        .as_deref()
        .map(|annotation| {
            let lower = annotation.to_lowercase();
            lower.contains("handler") || lower.contains("middleware") || lower.contains("function")
        })
        .unwrap_or(false);
    first_param_is_root && returns_handler
}

fn text_suggests_http_static_file_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    let static_context = lower.contains("static-file")
        || lower.contains("static file")
        || lower.contains("static serving")
        || lower.contains("static-file wrapper")
        || lower.contains("static root");
    let file_context = [
        "serve known files",
        "serving a known static file",
        "serving an existing file",
        "serve an existing file",
        "serve known file",
        "static/",
        "hello.txt",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    static_context && file_context
}

fn text_suggests_same_value_zero_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("samevaluezero")
        || lower.contains("same value zero")
        || lower.contains("same_value_zero")
}

fn text_suggests_pep440_context(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("pep 440")
        || lower.contains("pep440")
        || lower.contains("pypa/packaging")
        || lower.contains("packaging.version")
        || lower.contains("packaging specifier")
}

fn text_suggests_pep440_version_ordering(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("version-ordering")
            || lower.contains("version ordering")
            || lower.contains("compare_versions")
            || lower.contains("test_version.py")
            || (lower.contains("dev releases") && lower.contains("post releases")))
}

fn text_suggests_pep440_specifier_membership(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("specifier-set")
            || lower.contains("specifier behavior")
            || lower.contains("allows(version")
            || lower.contains("compatible release")
            || lower.contains("~="))
}

fn text_suggests_pep440_filter_prerelease(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_suggests_pep440_context(text)
        && (lower.contains("specifier.filter")
            || lower.contains("filter_versions")
            || lower.contains("prerelease fallback")
            || lower.contains("only matching candidates"))
}

fn source_suggests_cookie_quote_context(source_file: &str, source_text: Option<&str>) -> bool {
    let path = source_file.replace('\\', "/").to_lowercase();
    if path.contains("cookie") || path.ends_with("/_quote.py") || path.ends_with("/quote.py") {
        return true;
    }
    let Some(text) = source_text else {
        return false;
    };
    let lower = text.to_lowercase();
    lower.contains("cookie") && lower.contains("quote")
}

fn push_context_dir_candidates(dir: &Path, candidates: &mut Vec<PathBuf>) {
    for name in [
        "README.md",
        "readme.md",
        "UPSTREAM_NOTES.md",
        "CONTRACT.md",
        "contract.md",
        "API.md",
        "api.md",
    ] {
        candidates.push(dir.join(name));
    }

    let docs = dir.join("docs");
    for name in ["README.md", "readme.md", "CONTRACT.md", "contract.md"] {
        candidates.push(docs.join(name));
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten().take(40) {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("md") {
                candidates.push(path);
            }
        }
    }
}

fn discover_context_contract_files(source_file: &str, project_dir: Option<&str>) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let mut dirs = Vec::new();
    if let Some(dir) = source_path.parent() {
        dirs.push(dir.to_path_buf());
        if let Some(parent) = dir.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Some(root) = project_dir {
        dirs.push(PathBuf::from(root));
    }

    let mut candidates = Vec::new();
    for dir in dirs {
        push_context_dir_candidates(&dir, &mut candidates);
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| path.metadata().map(|meta| meta.len()).unwrap_or(0) <= 256 * 1024)
        .filter(|path| seen.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn infer_context_properties(
    source_file: &str,
    project_dir: Option<&str>,
    functions: &[FunctionInfo],
) -> HashMap<String, Vec<String>> {
    let query_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_query_nested_context(func))
        .collect();
    let request_metadata_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_request_metadata_context(func))
        .collect();
    let response_helper_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_response_helpers_context(func))
        .collect();
    let static_file_candidates: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|func| function_can_accept_http_static_file_context(func))
        .collect();
    let source_text = std::fs::read_to_string(source_file).ok();
    let has_pep440_candidate = functions.iter().any(|func| {
        let lower = func.name.to_lowercase();
        (lower.contains("compare") && lower.contains("version"))
            || lower == "allows"
            || (lower.contains("filter") && lower.contains("version"))
    });
    if query_candidates.is_empty()
        && request_metadata_candidates.is_empty()
        && response_helper_candidates.is_empty()
        && static_file_candidates.is_empty()
        && !functions.iter().any(|func| {
            func.name
                .to_lowercase()
                .replace(['_', '-'], "")
                .contains("samevaluezero")
        })
        && !has_pep440_candidate
        && !source_suggests_cookie_quote_context(source_file, source_text.as_deref())
    {
        return HashMap::new();
    }

    let mut inferred = HashMap::new();
    if source_suggests_cookie_quote_context(source_file, source_text.as_deref()) {
        for func in functions {
            match func.name.as_str() {
                "format_cookie_value" => {
                    push_inferred_property(&mut inferred, &func.name, COOKIE_VALUE_QUOTE_PROPERTY)
                }
                "build_cookie_header" => {
                    push_inferred_property(&mut inferred, &func.name, COOKIE_HEADER_QUOTE_PROPERTY)
                }
                _ => {}
            }
        }
    }
    for path in discover_context_contract_files(source_file, project_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = text.to_lowercase();
        if text_suggests_query_nested_brackets_contract(&text) {
            for func in &query_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || query_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        QUERY_NESTED_BRACKETS_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_request_metadata_contract(&text) {
            for func in &request_metadata_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || request_metadata_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_REQUEST_METADATA_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_response_helpers_contract(&text) {
            for func in &response_helper_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || response_helper_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_RESPONSE_HELPERS_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_http_static_file_contract(&text) {
            for func in &static_file_candidates {
                let function_mentioned = lower.contains(&func.name.to_lowercase());
                if function_mentioned || static_file_candidates.len() == 1 {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        HTTP_STATIC_FILE_MIDDLEWARE_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_same_value_zero_contract(&text) {
            for func in functions {
                let normalized_name = func.name.to_lowercase().replace(['_', '-'], "");
                if normalized_name.contains("samevaluezero") {
                    push_inferred_property(&mut inferred, &func.name, SAME_VALUE_ZERO_PROPERTY);
                }
            }
        }
        if text_suggests_pep440_version_ordering(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower.contains("compare") && lower.contains("version") {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_VERSION_ORDERING_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_pep440_specifier_membership(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower == "allows" || (lower.contains("allow") && lower.contains("specifier")) {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_SPECIFIER_MEMBERSHIP_PROPERTY,
                    );
                }
            }
        }
        if text_suggests_pep440_filter_prerelease(&text) {
            for func in functions {
                let lower = func.name.to_lowercase();
                if lower.contains("filter") && lower.contains("version") {
                    push_inferred_property(
                        &mut inferred,
                        &func.name,
                        PEP440_FILTER_PRERELEASE_PROPERTY,
                    );
                }
            }
        }
    }
    inferred
}

fn is_ignored_project_seed_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | ".hg"
                    | ".svn"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | "coverage"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
            )
        })
}

fn is_probably_test_seed_file(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.contains("/tests/")
        || normalized.contains("/__tests__/")
        || normalized.contains("/test/")
    {
        return true;
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.starts_with("test_")
                || name.ends_with("_test.py")
                || name.contains(".test.")
                || name.contains(".spec.")
        })
}

fn is_supported_project_seed_file(path: &Path, language: &Language) -> bool {
    match language {
        Language::TypeScript => {
            path.extension().and_then(|value| value.to_str()) == Some("ts")
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with(".d.ts"))
        }
        Language::Python => path.extension().and_then(|value| value.to_str()) == Some("py"),
    }
}

fn discover_project_seed_files(
    source_file: &str,
    project_dir: Option<&str>,
    language: &Language,
) -> Vec<PathBuf> {
    let source_path = Path::new(source_file);
    let root = project_dir
        .map(PathBuf::from)
        .or_else(|| source_path.parent().map(Path::to_path_buf));
    let Some(root) = root else {
        return Vec::new();
    };

    let source_canonical = source_path.canonicalize().ok();
    let mut stack = vec![root];
    let mut candidates = Vec::new();

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_ignored_project_seed_dir(&path) {
                    stack.push(path);
                }
                continue;
            }
            if candidates.len() >= 80 {
                continue;
            }
            if !is_supported_project_seed_file(&path, language) || is_probably_test_seed_file(&path)
            {
                continue;
            }
            if path.metadata().map(|meta| meta.len()).unwrap_or(0) > 256 * 1024 {
                continue;
            }
            if source_canonical
                .as_ref()
                .is_some_and(|source| path.canonicalize().ok().as_ref() == Some(source))
            {
                continue;
            }
            candidates.push(path);
        }
    }

    candidates.sort();
    candidates
}

fn source_mode_for_seed_file(language: &Language, path: Option<&str>) -> SourceMode {
    if matches!(language, Language::Python) {
        return SourceMode::Python;
    }
    match path
        .and_then(|value| Path::new(value).extension())
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("tsx") | Some("jsx") => SourceMode::Tsx,
        _ => SourceMode::TypeScript,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_seed_observations(
    code: &str,
    language: &Language,
    source_mode: SourceMode,
    functions: &[FunctionInfo],
    source_file: Option<&str>,
    project_dir: Option<&str>,
    explicit_test_code: Option<&str>,
    explicit_test_file: Option<&str>,
    auto_seed: bool,
) -> Vec<ObservedCall> {
    let function_names: HashSet<String> = functions.iter().map(|func| func.name.clone()).collect();
    let mut observed = Vec::new();

    let primary_label = source_file.unwrap_or("<source>");
    observed.extend(collect_observed_calls(
        code,
        language,
        source_mode,
        &function_names,
        primary_label,
    ));

    if let Some(test_code) = explicit_test_code {
        observed.extend(collect_observed_calls(
            test_code,
            language,
            source_mode_for_seed_file(language, explicit_test_file),
            &function_names,
            explicit_test_file.unwrap_or("<explicit-test>"),
        ));
    } else if auto_seed {
        if let Some(source_file) = source_file {
            for path in discover_seed_files(source_file, language) {
                if let Ok(test_code) = std::fs::read_to_string(&path) {
                    observed.extend(collect_observed_calls(
                        &test_code,
                        language,
                        source_mode_for_seed_file(language, path.to_str()),
                        &function_names,
                        &path.to_string_lossy(),
                    ));
                }
            }
        }
    }

    if auto_seed {
        if let Some(source_file) = source_file {
            for path in discover_project_seed_files(source_file, project_dir, language) {
                if let Ok(context_code) = std::fs::read_to_string(&path) {
                    observed.extend(collect_observed_calls(
                        &context_code,
                        language,
                        source_mode_for_seed_file(language, path.to_str()),
                        &function_names,
                        &path.to_string_lossy(),
                    ));
                }
            }
        }
    }

    observed
}

fn seed_sources(observed_calls: &[ObservedCall]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for observed in observed_calls {
        if seen.insert(observed.source_label.clone()) {
            sources.push(observed.source_label.clone());
        }
    }
    sources
}

fn classify_type_signature_findings(
    findings: &mut [VerificationFinding],
    observed_calls: &[ObservedCall],
    language: &Language,
) {
    if !matches!(language, Language::TypeScript) {
        return;
    }
    let mut by_function: HashMap<&str, Vec<&ObservedCall>> = HashMap::new();
    for observed in observed_calls {
        by_function
            .entry(observed.function.as_str())
            .or_default()
            .push(observed);
    }
    for finding in findings.iter_mut() {
        if finding.severity != FindingSeverity::Crash || finding.classification.is_some() {
            continue;
        }
        let Some(observed) = by_function.get(finding.location.function.as_str()) else {
            continue;
        };
        for (index, arg) in finding.repro.arguments.iter().enumerate() {
            let Some(failing_number) = arg.json_value.as_ref().and_then(|v| v.as_f64()) else {
                continue;
            };
            let mut observed_numbers = Vec::new();
            let mut saw_non_numeric = false;
            for call in observed {
                match call
                    .args
                    .get(index)
                    .and_then(|item| item.literal_value.as_ref())
                {
                    Some(value) if value.is_number() => {
                        if let Some(number) = value.as_f64() {
                            observed_numbers.push(number);
                        }
                    }
                    Some(_) | None => {
                        saw_non_numeric = true;
                        break;
                    }
                }
            }
            if saw_non_numeric || observed_numbers.is_empty() || observed_numbers.len() > 8 {
                continue;
            }
            if observed_numbers
                .iter()
                .any(|value| (*value - failing_number).abs() < f64::EPSILON)
            {
                continue;
            }
            let mut preview = observed_numbers
                .iter()
                .map(|value| {
                    if value.fract() == 0.0 {
                        format!("{}", *value as i64)
                    } else {
                        value.to_string()
                    }
                })
                .collect::<Vec<_>>();
            preview.sort();
            preview.dedup();
            finding.classification = Some("type_signature_wider_than_usage".into());
            finding.suggestion = Some(format!("Observed static call sites only pass literal values like {} for parameter {}. Consider narrowing the declared type before adding a runtime guard.", preview.join(", "), index + 1));
            break;
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FuzzOutcomeStatus {
    Passed,
    Crashed,
    NoInputsReached,
}

#[derive(Debug, Clone, Serialize)]
struct FuzzOutcome {
    function: String,
    status: FuzzOutcomeStatus,
    pass_count: usize,
    reject_count: usize,
    crash_count: usize,
    total_count: usize,
}

fn parse_leading_usize(raw: &str, suffix: &str) -> Option<usize> {
    raw.trim()
        .strip_suffix(suffix)?
        .trim()
        .parse::<usize>()
        .ok()
}

fn parse_fuzz_outcomes(stdout: &str) -> Vec<FuzzOutcome> {
    let mut outcomes = Vec::new();

    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("FUZZ ") else {
            continue;
        };
        let Some((function, result)) = rest.split_once(": ") else {
            continue;
        };

        if let Some(total) = result
            .strip_prefix("all ")
            .and_then(|value| value.strip_suffix(" inputs rejected (nothing tested)"))
            .and_then(|value| value.parse::<usize>().ok())
        {
            outcomes.push(FuzzOutcome {
                function: function.to_string(),
                status: FuzzOutcomeStatus::NoInputsReached,
                pass_count: 0,
                reject_count: total,
                crash_count: 0,
                total_count: total,
            });
            continue;
        }

        let core = result.split(" [exercises: ").next().unwrap_or(result);
        let total_count = core
            .rsplit_once("(of ")
            .and_then(|(_, tail)| tail.strip_suffix(')'))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let parts: Vec<&str> = core.split(", ").map(|part| part.trim()).collect();
        if parts.len() < 2 {
            continue;
        }

        let pass_count = parse_leading_usize(parts[0], " passed").unwrap_or(0);
        let reject_count = parts
            .get(1)
            .and_then(|part| part.split(" (of ").next())
            .and_then(|part| parse_leading_usize(part, " rejected"))
            .unwrap_or(0);
        let crash_count = parts
            .get(2)
            .and_then(|part| part.split(" (of ").next())
            .and_then(|part| parse_leading_usize(part, " CRASHED"))
            .unwrap_or(0);
        let status = if crash_count > 0 {
            FuzzOutcomeStatus::Crashed
        } else if pass_count > 0 {
            FuzzOutcomeStatus::Passed
        } else {
            FuzzOutcomeStatus::NoInputsReached
        };

        outcomes.push(FuzzOutcome {
            function: function.to_string(),
            status,
            pass_count,
            reject_count,
            crash_count,
            total_count,
        });
    }

    outcomes
}
const MAX_FINDING_INPUT_SAMPLES: usize = 3;

fn finding_repro_case(finding: &VerificationFinding) -> &ReproCase {
    finding
        .minimization
        .minimized
        .as_ref()
        .unwrap_or(&finding.minimization.original)
}

fn coalesce_equivalent_findings(findings: Vec<VerificationFinding>) -> Vec<VerificationFinding> {
    let mut coalesced = Vec::<VerificationFinding>::new();
    let mut fingerprints = HashMap::<String, usize>::new();
    for mut finding in findings {
        let fingerprint = serde_json::json!({
            "source_file": finding.location.source_file,
            "function": finding.location.function,
            "line": finding.location.line,
            "invocation_path": finding.location.invocation_path,
            "severity": finding.severity,
            "confidence": finding.confidence,
            "category": finding.category,
            "oracle_id": finding.oracle.id,
            "oracle_kind": finding.oracle.kind,
            "oracle_provenance": finding.oracle.provenance,
            "input_classification": finding.input_classification,
            "error_type": finding.error_type,
            "message": finding.message,
            "classification": finding.classification,
        })
        .to_string();
        let sample = finding_repro_case(&finding).clone();
        if let Some(index) = fingerprints.get(&fingerprint).copied() {
            let existing = &mut coalesced[index];
            existing.occurrences = existing
                .occurrences
                .saturating_add(finding.occurrences.max(1));
            if existing.sample_inputs.len() < MAX_FINDING_INPUT_SAMPLES
                && !existing.sample_inputs.contains(&sample)
            {
                existing.sample_inputs.push(sample);
            }
            continue;
        }
        finding.occurrences = finding.occurrences.max(1);
        if finding.sample_inputs.is_empty() {
            finding.sample_inputs.push(sample);
        } else {
            finding.sample_inputs.truncate(MAX_FINDING_INPUT_SAMPLES);
        }
        fingerprints.insert(fingerprint, coalesced.len());
        coalesced.push(finding);
    }
    coalesced
}

fn findings_summary(
    findings: &[VerificationFinding],
    suppressed: &[VerificationFinding],
    inferred_gate: InferredOracleGate,
) -> FindingsSummary {
    let mut summary = FindingsSummary {
        total: findings.len() + suppressed.len(),
        occurrences: findings
            .iter()
            .chain(suppressed)
            .map(|finding| finding.occurrences.max(1))
            .sum(),
        gating: 0,
        advisory: suppressed.len(),
        suppressed: suppressed.len(),
        by_severity: BTreeMap::new(),
        by_oracle_kind: BTreeMap::new(),
    };
    for finding in findings {
        let advisory = finding.input_classification != InputClassification::Valid
            || finding.confidence == FindingConfidence::Low
                && inferred_gate == InferredOracleGate::Advisory;
        if advisory {
            summary.advisory += 1;
        } else {
            summary.gating += 1;
        }
        let severity = serde_json::to_string(&finding.severity)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        *summary.by_severity.entry(severity).or_default() += 1;
        let oracle = serde_json::to_string(&finding.oracle.kind)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        *summary.by_oracle_kind.entry(oracle).or_default() += 1;
    }
    summary
}

fn execute_gate_failed(
    gate: ExecuteGate,
    findings: &[VerificationFinding],
    inferred_gate: InferredOracleGate,
) -> bool {
    findings
        .iter()
        .any(|finding| finding_fails_execute_gate(gate, finding, inferred_gate))
}

fn finding_fails_execute_gate(
    gate: ExecuteGate,
    finding: &VerificationFinding,
    inferred_gate: InferredOracleGate,
) -> bool {
    gate != ExecuteGate::None
        && finding.input_classification == InputClassification::Valid
        && !(finding.confidence == FindingConfidence::Low
            && inferred_gate == InferredOracleGate::Advisory)
        && (gate == ExecuteGate::All
            || (gate == ExecuteGate::Crash && finding.severity == FindingSeverity::Crash))
}

fn execute_stage_ok(
    result: &ExecutionResult,
    gate: ExecuteGate,
    inferred_gate: InferredOracleGate,
    active_findings: &[VerificationFinding],
    suppressed_findings: &[VerificationFinding],
    module_load_blocked: bool,
) -> bool {
    if result.timed_out || result.memory_error || result.exit_code.is_none() || module_load_blocked
    {
        return false;
    }
    if active_findings
        .iter()
        .any(|finding| finding.input_classification == InputClassification::Unknown)
    {
        return false;
    }
    if execute_gate_failed(gate, active_findings, inferred_gate) {
        return false;
    }
    !active_findings.is_empty() || !suppressed_findings.is_empty() || result.exit_code == Some(0)
}
fn resolve_verification_contexts(
    opts: &VerifyOptions<'_>,
    language: &Language,
) -> Result<VerificationContext, String> {
    let invocation_dir = std::env::current_dir()
        .map_err(|error| format!("cannot resolve invocation directory: {error}"))?;
    let candidate = ContextRequest {
        invocation_dir: &invocation_dir,
        explicit_project_dir: opts.project_dir.map(Path::new),
        target_file: opts.source_file.map(Path::new),
        test_file: opts.test_source_file.map(Path::new),
        language: *language,
        virtual_file_path: opts.lint_virtual_file_path.map(Path::new),
    };
    let base = (opts.base_code.is_some()
        || opts.base_source_file.is_some()
        || opts.base_project_dir.is_some())
    .then(|| ContextRequest {
        invocation_dir: &invocation_dir,
        explicit_project_dir: opts.base_project_dir.map(Path::new),
        target_file: opts.base_source_file.map(Path::new),
        test_file: None,
        language: *language,
        virtual_file_path: None,
    });
    crate::resolve_verification_context(candidate, base).map_err(|error| error.to_string())
}

fn generated_harness_runtime(mode: SourceMode) -> (HarnessRuntime, &'static str, &'static str) {
    match mode {
        SourceMode::Python => (HarnessRuntime::Python, "python", "py"),
        SourceMode::TypeScript => (HarnessRuntime::NodeScript, "node", "ts"),
        SourceMode::Tsx => (HarnessRuntime::TsxScript, "tsx", "tsx"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_generated_harness<'a>(
    context: &ExecutionContext,
    code: String,
    kind: HarnessKind,
    opts: &VerifyOptions<'a>,
    language: &Language,
    timeout_seconds: f64,
    project_dir: Option<&'a str>,
    source_file: Option<&'a str>,
) -> HarnessExecution {
    let source_mode = context.target_source.mode;
    let (runtime, _runtime_name, extension) = generated_harness_runtime(source_mode);
    let relative_path = context
        .target_source
        .source_file
        .as_deref()
        .and_then(|source| source.strip_prefix(&context.workspace_root).ok())
        .and_then(Path::parent)
        .map(|parent| parent.join(format!(".court-jester-generated-verify.{extension}")))
        .unwrap_or_else(|| PathBuf::from(format!(".court-jester/generated/verify.{extension}")));
    sandbox::execute_harness(
        context,
        HarnessSpec {
            kind,
            runtime,
            test_adapter: None,
            source_mode,
            artifact: HarnessArtifact::Generated {
                code,
                relative_path,
            },
            args: Vec::new(),
            network: opts.network,
        },
        sandbox_options(
            opts,
            language,
            timeout_seconds,
            opts.memory_mb,
            project_dir,
            source_file,
        ),
    )
    .await
}
#[derive(Debug, Deserialize)]
struct LlmPlateauResponse {
    seeds: Vec<LlmSeedProposal>,
}

#[derive(Debug, Deserialize)]
struct LlmSeedProposal {
    function: String,
    arguments: Vec<serde_json::Value>,
}

struct LlmPlateauOutcome {
    corpus: PersistentCorpus,
    proposed_count: usize,
    invalid_count: usize,
    stderr: String,
    duration_ms: u64,
}

fn llm_plateau_command() -> Option<String> {
    std::env::var("COURT_JESTER_LLM_PLATEAU_COMMAND")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn propose_llm_plateau_seeds(
    command: &str,
    language: &Language,
    functions: &[FunctionInfo],
    corpus: &PersistentCorpus,
    project_dir: Option<&str>,
) -> Result<LlmPlateauOutcome, String> {
    let started = Instant::now();
    let prompt = serde_json::json!({
        "protocol_version": 1,
        "task": "Propose high-value argument lists that may reach behavior not represented in the retained corpus. Return JSON only.",
        "language": match language {
            Language::Python => "python",
            Language::TypeScript => "typescript",
        },
        "constraints": {
            "maximum_seeds": 32,
            "response_schema": {
                "seeds": [{
                    "function": "exact exported function name",
                    "arguments": ["JSON value for each positional argument"]
                }]
            }
        },
        "targets": functions.iter().map(|function| serde_json::json!({
            "function": function.name,
            "line": function.line,
            "parameters": function.params.iter().map(|parameter| serde_json::json!({
                "name": parameter.name,
                "type": parameter.type_annotation,
                "optional": parameter.optional,
                "keyword_only": parameter.keyword_only,
                "variadic": parameter.variadic.is_some(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "retained_corpus": corpus,
    });
    let prompt = serde_json::to_vec(&prompt)
        .map_err(|error| format!("failed to serialize LLM plateau prompt: {error}"))?;
    let mut process = tokio::process::Command::new(command);
    process
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .env("COURT_JESTER_LLM_PROTOCOL", "plateau-seeds-v1");
    if let Some(project_dir) = project_dir {
        process.current_dir(project_dir);
    }
    let mut child = process
        .spawn()
        .map_err(|error| format!("failed to launch LLM plateau command `{command}`: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "LLM plateau command stdin was unavailable".to_string())?;
    stdin
        .write_all(&prompt)
        .await
        .map_err(|error| format!("failed to write LLM plateau prompt: {error}"))?;
    drop(stdin);
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| "LLM plateau command timed out after 20 seconds".to_string())?
        .map_err(|error| format!("LLM plateau command failed: {error}"))?;
    if output.stdout.len() > 1_048_576 || output.stderr.len() > 1_048_576 {
        return Err("LLM plateau command output exceeded 1 MiB".into());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "LLM plateau command exited with {}{}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "a signal".into()),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }
    let response: LlmPlateauResponse = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("LLM plateau command returned invalid JSON: {error}"))?;
    let proposed_count = response.seeds.len();
    let mut invalid_count = proposed_count.saturating_sub(32);
    let mut proposed_corpus = PersistentCorpus::new();
    for proposal in response.seeds.into_iter().take(32) {
        let Some(function) = functions
            .iter()
            .find(|function| function.name == proposal.function)
        else {
            invalid_count += 1;
            continue;
        };
        let required = function
            .params
            .iter()
            .filter(|parameter| {
                !parameter.optional
                    && parameter.default_value.is_none()
                    && parameter.variadic.is_none()
            })
            .count();
        let has_variadic = function
            .params
            .iter()
            .any(|parameter| parameter.variadic.is_some());
        let positional_limit = function
            .params
            .iter()
            .filter(|parameter| !parameter.keyword_only && parameter.variadic.is_none())
            .count();
        let has_required_keyword_only = function.params.iter().any(|parameter| {
            parameter.keyword_only && !parameter.optional && parameter.default_value.is_none()
        });
        if has_required_keyword_only
            || proposal.arguments.len() < required
            || (!has_variadic && proposal.arguments.len() > positional_limit)
            || serde_json::to_vec(&proposal.arguments)
                .map(|bytes| bytes.len() > 65_536)
                .unwrap_or(true)
        {
            invalid_count += 1;
            continue;
        }
        let surface_id = format!("{}:{}", function.name, function.line);
        let rows = proposed_corpus.entry(surface_id).or_default();
        if !rows.contains(&proposal.arguments) {
            rows.push(proposal.arguments);
        }
    }
    Ok(LlmPlateauOutcome {
        corpus: proposed_corpus,
        proposed_count,
        invalid_count,
        stderr,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_native_fuzz_harness<'a>(
    context: &ExecutionContext,
    code: String,
    plan: &synthesize::NativeFuzzPlan,
    runs: usize,
    opts: &VerifyOptions<'a>,
    language: &Language,
    timeout_seconds: f64,
    project_dir: Option<&'a str>,
    source_file: Option<&'a str>,
) -> HarnessExecution {
    let source_mode = context.target_source.mode;
    let (runtime, extension, args) = match plan.engine {
        NativeFuzzEngine::Atheris => (
            HarnessRuntime::Python,
            "py",
            vec![HarnessArg::Literal {
                literal: format!("-atheris_runs={runs}"),
            }],
        ),
        NativeFuzzEngine::Jazzer => (
            HarnessRuntime::Jazzer,
            match source_mode {
                SourceMode::Tsx => "tsx",
                _ => "ts",
            },
            vec![
                HarnessArg::Literal {
                    literal: "--".into(),
                },
                HarnessArg::Literal {
                    literal: format!("-runs={runs}"),
                },
            ],
        ),
        NativeFuzzEngine::Off | NativeFuzzEngine::Auto => {
            unreachable!("native fuzz synthesis resolves auto to a concrete engine")
        }
    };
    let relative_path = context
        .target_source
        .source_file
        .as_deref()
        .and_then(|source| source.strip_prefix(&context.workspace_root).ok())
        .and_then(Path::parent)
        .map(|parent| parent.join(format!(".court-jester-native-fuzz.{extension}")))
        .unwrap_or_else(|| {
            PathBuf::from(format!(".court-jester/generated/native-fuzz.{extension}"))
        });
    sandbox::execute_harness(
        context,
        HarnessSpec {
            kind: HarnessKind::Standalone,
            runtime,
            test_adapter: None,
            source_mode,
            artifact: HarnessArtifact::Generated {
                code,
                relative_path,
            },
            args,
            network: opts.network,
        },
        sandbox_options(
            opts,
            language,
            timeout_seconds,
            opts.memory_mb,
            project_dir,
            source_file,
        ),
    )
    .await
}

fn authoritative_harness_runtime(
    language: Language,
    runner: TestRunner,
    test_code: &str,
    context: &ExecutionContext,
) -> (HarnessRuntime, Option<TestAdapter>) {
    match language {
        Language::Python => (HarnessRuntime::Python, None),
        Language::TypeScript => match runner {
            TestRunner::Bun => (HarnessRuntime::BunTest, Some(TestAdapter::BunJunit)),
            TestRunner::RepoNative => (HarnessRuntime::RepoTest, Some(TestAdapter::Opaque)),
            TestRunner::Node => (HarnessRuntime::NodeTest, Some(TestAdapter::NodeTap)),
            TestRunner::Auto
                if typescript_code_imports_vitest(test_code)
                    || context_declares_vitest(context) =>
            {
                (HarnessRuntime::Vitest, Some(TestAdapter::VitestJson))
            }
            TestRunner::Auto => (HarnessRuntime::NodeScript, Some(TestAdapter::Opaque)),
        },
    }
}
fn authoritative_execution_context(
    context: &ExecutionContext,
    project_dir: Option<&str>,
    test_file: Option<&str>,
) -> ExecutionContext {
    let Some(project_dir) = project_dir else {
        return context.clone();
    };
    let project = PathBuf::from(project_dir);
    let original_workspace = &context.workspace_root;
    let map_path = |path: Option<&Path>| {
        path.and_then(|path| {
            path.strip_prefix(original_workspace)
                .ok()
                .map(|relative| project.join(relative))
        })
    };
    let target_source_file = map_path(context.target_source.source_file.as_deref());
    let test_source_file = test_file.map(PathBuf::from).or_else(|| {
        map_path(
            context
                .test_source
                .as_ref()
                .and_then(|source| source.source_file.as_deref()),
        )
    });
    let target_package_root = context
        .target_package_root
        .strip_prefix(original_workspace)
        .ok()
        .map(|relative| project.join(relative))
        .unwrap_or_else(|| project.clone());
    let test_package_root = test_source_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| Some(target_package_root.clone()));
    let mut dependency_roots = Vec::new();
    for root in &context.dependency_roots {
        if let Ok(relative) = root.strip_prefix(original_workspace) {
            let mapped = project.join(relative);
            if !dependency_roots.iter().any(|existing| existing == &mapped) {
                dependency_roots.push(mapped);
            }
        }
    }
    for root in context
        .dependency_roots
        .iter()
        .chain(std::iter::once(original_workspace))
    {
        if !dependency_roots.iter().any(|existing| existing == root) {
            dependency_roots.push(root.clone());
        }
    }
    let test_source = test_source_file.map(|source_file| SourceContext {
        language: context.target_source.language,
        mode: source_mode_for_path(&source_file, context.target_source.language),
        source_file: Some(source_file),
        virtual_file_path: None,
    });
    let mut resolved = context.clone();
    resolved.invocation_dir = project.clone();
    resolved.materialization_source_root = Some(original_workspace.clone());
    resolved.workspace_root = project;
    resolved.target_package_root = target_package_root;
    resolved.test_package_root = test_package_root;
    resolved.dependency_roots = dependency_roots;
    resolved.target_source.source_file = target_source_file;
    resolved.target_source.virtual_file_path = None;
    resolved.test_source = test_source;
    resolved
}

fn source_mode_for_path(path: &Path, language: Language) -> SourceMode {
    if language == Language::Python {
        return SourceMode::Python;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if extension.eq_ignore_ascii_case("tsx") || extension.eq_ignore_ascii_case("jsx") =>
        {
            SourceMode::Tsx
        }
        _ => SourceMode::TypeScript,
    }
}

fn authoritative_artifact_path(
    context: &ExecutionContext,
    project_dir: Option<&str>,
    source_file: Option<&str>,
    mode: SourceMode,
) -> PathBuf {
    let extension = match mode {
        SourceMode::Python => "py",
        SourceMode::TypeScript => "ts",
        SourceMode::Tsx => "tsx",
    };
    project_dir
        .and_then(|project| {
            source_file
                .and_then(|source| Path::new(source).strip_prefix(project).ok())
                .and_then(Path::parent)
                .map(|parent| {
                    parent.join(format!(".court-jester-generated-authoritative.{extension}"))
                })
        })
        .or_else(|| {
            context
                .target_source
                .source_file
                .as_deref()
                .and_then(|source| source.strip_prefix(&context.workspace_root).ok())
                .and_then(Path::parent)
                .map(|parent| {
                    parent.join(format!(".court-jester-generated-authoritative.{extension}"))
                })
        })
        .unwrap_or_else(|| {
            PathBuf::from(format!(".court-jester/generated/authoritative.{extension}"))
        })
}
fn skipped_execute_stage(reason: &str, message: &str) -> VerificationStage {
    VerificationStage {
        name: "execute".into(),
        status: StageStatus::Skipped,
        duration_ms: 0,
        detail: Some(serde_json::json!({
            "skipped": true,
            "reason": reason,
            "generated_cases": 0,
            "valid_invocations": 0,
            "evaluated_oracles": 0,
            "no_inputs_reached": 0,
            "findings": [],
            "suppressed_findings": [],
        })),
        message: Some(message.into()),
    }
}

struct AuthoritativeTestOutcome {
    result: ExecutionResult,
    overlay: InstrumentationOverlay,
    entered_surfaces: HashSet<String>,
    has_non_target_blocker: bool,
    has_assertion_failure: bool,
    test_ok: bool,
    covered_required: usize,
    duration_ms: u64,
    selected_test_runner: TestRunner,
}

pub struct EntrypointProbeOptions<'a> {
    pub source_file: &'a str,
    pub test_source_file: &'a str,
    pub project_dir: Option<&'a str>,
    pub test_runner: TestRunner,
    pub timeout_seconds: f64,
    pub memory_mb: u64,
    pub runtime_profile: RuntimeProfile,
    pub python_docker_image: &'a str,
    pub typescript_docker_image: &'a str,
}

/// Execute only the selected authoritative entrypoint through the selected runtime
/// test adapter. This is an opt-in readiness probe, not fuzzing or a coverage claim.
pub async fn probe_authoritative_entrypoint(
    code: &str,
    tests: &str,
    language: &Language,
    probe: EntrypointProbeOptions<'_>,
) -> Result<VerificationStage, String> {
    if !probe.timeout_seconds.is_finite() || probe.timeout_seconds <= 0.0 || probe.memory_mb == 0 {
        return Err("entrypoint probe requires finite positive timeout and memory limits".into());
    }
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(probe.test_source_file),
        test_runner: probe.test_runner,
        tests_only: true,
        test_quality_max_mutants: None,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: probe.project_dir,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: false,
        source_file: Some(probe.source_file),
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::None,
        coverage_gate: CoverageGate::None,
        inferred_oracle_gate: InferredOracleGate::Advisory,
        runtime_profile: probe.runtime_profile,
        memory_mb: probe.memory_mb,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
        python_docker_image: probe.python_docker_image,
        typescript_docker_image: probe.typescript_docker_image,
    };
    let context = resolve_verification_contexts(&opts, language)?;
    let nonce = tempfile::Builder::new()
        .prefix("doctor-probe-")
        .tempfile()
        .map_err(|error| error.to_string())?;
    let marker = format!(
        "__COURT_JESTER_ENTRYPOINT_LOADED__:{}",
        nonce.path().file_name().unwrap().to_string_lossy()
    );
    let literal = serde_json::to_string(&marker).map_err(|error| error.to_string())?;
    let instrumented = match language {
        Language::Python => format!("{code}\nimport builtins as _court_jester_probe_builtins\n_court_jester_probe_builtins.print({literal})\n"),
        Language::TypeScript => format!("{code}\nglobalThis.console.log({literal});\n"),
    };
    let source = context
        .candidate
        .target_source
        .source_file
        .as_ref()
        .and_then(|path| path.to_str());
    let test = context
        .candidate
        .test_source
        .as_ref()
        .and_then(|source| source.source_file.as_ref())
        .and_then(|path| path.to_str());
    let outcome = run_authoritative_test(
        &instrumented,
        tests,
        &[],
        language,
        &context,
        &opts,
        source,
        test,
        probe.timeout_seconds,
    )
    .await;
    let loaded = outcome.result.stdout.lines().any(|line| {
        authoritative_output_line(line, language, outcome.selected_test_runner) == marker
    });
    let mut stage = authoritative_test_stage(&outcome, &opts);
    let detail = stage.detail.as_mut().unwrap();
    detail["target_module_loaded"] = serde_json::Value::Bool(loaded);
    if stage.status == StageStatus::Passed && !loaded {
        stage.status = StageStatus::Inconclusive;
        let message = "Entrypoint exited successfully without evidence that the selected target module loaded";
        stage.message = Some(message.into());
        detail["diagnostic"] = serde_json::to_value(FailureDiagnostic {
            domain: FailureDomain::VerifierHarness,
            kind: FailureKind::Instrumentation,
            component: DiagnosticComponent::Instrumentation,
            impact: DiagnosticImpact::Blocking,
            message: message.into(),
            process: None,
            limits: None,
        })
        .unwrap();
    }
    Ok(stage)
}

fn authoritative_test_stage(
    outcome: &AuthoritativeTestOutcome,
    opts: &VerifyOptions<'_>,
) -> VerificationStage {
    let mut detail = serde_json::to_value(&outcome.result).unwrap();
    detail["assertion_failure"] = serde_json::Value::Bool(outcome.has_assertion_failure);
    detail["non_target_blocking"] = serde_json::Value::Bool(outcome.has_non_target_blocker);
    detail["instrumentation_overlay"] = serde_json::to_value(&outcome.overlay).unwrap();
    detail["target_entered_surfaces"] = serde_json::to_value(&outcome.entered_surfaces).unwrap();
    detail["authoritative_test_covered_surfaces"] =
        serde_json::Value::from(outcome.covered_required);
    detail["tests_only"] = serde_json::Value::Bool(opts.tests_only);
    detail["test_runner_requested"] = serde_json::to_value(opts.test_runner).unwrap();
    detail["test_runner_selected"] = serde_json::to_value(outcome.selected_test_runner).unwrap();
    sanitize_report_value(&mut detail);
    VerificationStage {
        name: "test".into(),
        status: if !outcome.overlay.supported || outcome.has_non_target_blocker {
            StageStatus::Inconclusive
        } else if outcome.test_ok {
            StageStatus::Passed
        } else {
            StageStatus::Failed
        },
        duration_ms: outcome.duration_ms,
        detail: Some(detail),
        message: if !outcome.overlay.supported {
            outcome.overlay.reason.clone()
        } else if outcome.test_ok {
            None
        } else {
            Some(sanitize_report_text(&outcome.result.stderr))
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_authoritative_test(
    code: &str,
    tests: &str,
    required_functions: &[&FunctionInfo],
    language: &Language,
    verification_context: &VerificationContext,
    opts: &VerifyOptions<'_>,
    candidate_source_file: Option<&str>,
    candidate_test_source_file: Option<&str>,
    timeout_seconds: f64,
) -> AuthoritativeTestOutcome {
    let runner_probe = if test_code_has_imports(tests, language) {
        tests.to_string()
    } else {
        format!("{code}\n\n{tests}")
    };
    let selected_test_runner = match language {
        Language::TypeScript
            if opts.test_runner == TestRunner::Auto
                && typescript_code_imports_node_test(&runner_probe) =>
        {
            TestRunner::Node
        }
        Language::TypeScript => match opts.test_runner {
            TestRunner::Auto if sandbox::typescript_code_requires_bun_runtime(&runner_probe) => {
                TestRunner::Bun
            }
            other => other,
        },
        Language::Python => TestRunner::Auto,
    };
    let project_dir = verification_context
        .candidate
        .workspace_root
        .to_string_lossy()
        .into_owned();
    let prepared = prepare_authoritative_test(
        code,
        tests,
        required_functions,
        language,
        verification_context.candidate.target_source.mode,
        selected_test_runner,
        Some(project_dir.as_str()),
        candidate_source_file,
        candidate_test_source_file,
    );
    let execution_project = prepared.project_dir.as_deref();
    let execution_source = prepared.source_file.as_deref();
    let start = Instant::now();
    let result = if !prepared.overlay.supported {
        err_execution_result(
            prepared
                .overlay
                .reason
                .as_deref()
                .unwrap_or("authoritative-test instrumentation is unsupported"),
        )
    } else {
        let harness_context = authoritative_execution_context(
            &verification_context.candidate,
            execution_project,
            execution_source,
        );
        let source_mode = harness_context
            .test_source
            .as_ref()
            .map(|source| source.mode)
            .unwrap_or(harness_context.target_source.mode);
        let (runtime, test_adapter) = authoritative_harness_runtime(
            *language,
            selected_test_runner,
            &runner_probe,
            &harness_context,
        );
        let artifact = if prepared._root.is_some() {
            execution_source
                .and_then(|source| {
                    Path::new(source)
                        .strip_prefix(&harness_context.workspace_root)
                        .ok()
                        .map(Path::to_path_buf)
                })
                .map(|relative_path| HarnessArtifact::Existing { relative_path })
                .unwrap_or_else(|| HarnessArtifact::Generated {
                    code: prepared.code.clone(),
                    relative_path: authoritative_artifact_path(
                        &harness_context,
                        execution_project,
                        execution_source,
                        source_mode,
                    ),
                })
        } else if let Some(relative_path) = execution_source.and_then(|source| {
            let source = Path::new(source);
            source
                .is_file()
                .then(|| source.strip_prefix(&harness_context.workspace_root).ok())
                .flatten()
                .map(Path::to_path_buf)
        }) {
            HarnessArtifact::Existing { relative_path }
        } else {
            HarnessArtifact::Generated {
                code: prepared.code.clone(),
                relative_path: authoritative_artifact_path(
                    &harness_context,
                    execution_project,
                    execution_source,
                    source_mode,
                ),
            }
        };
        let mut limits = sandbox_options(
            opts,
            language,
            timeout_seconds,
            opts.memory_mb,
            execution_project,
            execution_source,
        );
        limits.instrumentation_target = prepared
            .instrumented_source
            .as_ref()
            .and(candidate_source_file);
        limits.instrumented_source = prepared.instrumented_source.as_deref();
        sandbox::execute_harness(
            &harness_context,
            HarnessSpec {
                kind: HarnessKind::AuthoritativeTest,
                runtime,
                test_adapter,
                source_mode,
                artifact,
                args: Vec::new(),
                network: opts.network,
            },
            limits,
        )
        .await
        .process
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let test_output = format!("{}\n{}", result.stdout, result.stderr);
    let entered_surfaces = parse_target_entered_lines(
        test_output
            .lines()
            .map(|line| authoritative_output_line(line, language, selected_test_runner)),
    );
    let has_non_target_blocker = has_non_target_blocking_diagnostic(&result.diagnostics);
    let has_assertion_failure = !has_non_target_blocker
        && (test_output.contains("Assertion failed")
            || test_output.contains("AssertionError")
            || result.diagnostics.iter().any(|diagnostic| {
                diagnostic.kind == FailureKind::AssertionFailure
                    && diagnostic.component == DiagnosticComponent::AuthoritativeTestRunner
            }));
    let test_ok = result.exit_code == Some(0)
        && !result.timed_out
        && !result.memory_error
        && !has_assertion_failure;
    let covered_required = required_functions
        .iter()
        .filter(|function| {
            entered_surfaces.contains(&format!("{}:{}", function.name, function.line))
        })
        .count();
    AuthoritativeTestOutcome {
        result,
        overlay: prepared.overlay,
        entered_surfaces,
        has_non_target_blocker,
        has_assertion_failure,
        test_ok,
        covered_required,
        duration_ms,
        selected_test_runner,
    }
}

fn test_quality_functions(
    analysis: &AnalysisResult,
    source_file: Option<&str>,
    diff_text: Option<&str>,
) -> Vec<FunctionInfo> {
    let candidates = if let Some(diff_text) = diff_text {
        let ranges = source_file
            .map(|path| diff::parse_changed_lines_for_file(diff_text, path))
            .unwrap_or_else(|| diff::parse_changed_lines(diff_text));
        analyze::filter_changed_functions(analysis, &ranges)
    } else {
        analysis.functions.clone()
    };
    candidates
        .into_iter()
        .filter(|function| {
            function.is_exported
                && !function.is_nested
                && (!function.is_method
                    || function
                        .invocation_target
                        .as_deref()
                        .is_some_and(|target| !target.trim().is_empty()))
        })
        .collect()
}

pub fn test_quality_candidate_count(
    code: &str,
    language: &Language,
    source_mode: SourceMode,
    source_file: Option<&str>,
    diff: Option<&str>,
) -> Result<usize, String> {
    let context = SourceContext {
        language: *language,
        mode: source_mode,
        source_file: source_file.map(PathBuf::from),
        virtual_file_path: None,
    };
    let analysis = analyze::analyze_with_context(code, &context);
    if analysis.parse_error {
        let message = analysis
            .parse_diagnostics
            .first()
            .map(|diagnostic| diagnostic.message.as_str())
            .unwrap_or("test-quality source analysis failed");
        return Err(sanitize_report_text(message));
    }
    let functions = test_quality_functions(&analysis, source_file, diff);
    let required_functions = functions.iter().collect::<Vec<_>>();
    test_quality::plan_mutations(
        code,
        *language,
        source_mode,
        &required_functions,
        usize::MAX,
    )
    .map(|planned| planned.len())
}

fn skipped_test_quality_stage(max_mutants: usize, reason: impl Into<String>) -> VerificationStage {
    VerificationStage {
        name: "test_quality".into(),
        status: StageStatus::Skipped,
        duration_ms: 0,
        detail: Some(serde_json::json!({
            "experimental": false,
            "mode": "advisory",
            "max_mutants": max_mutants,
            "baseline_eligible": false,
            "planning_error": null,
            "counts": {
                "planned": 0,
                "killed": 0,
                "survived": 0,
                "invalid": 0,
                "blocked": 0,
                "no_coverage": 0,
            },
            "mutants": [],
            "coupling_findings": [],
            "coupling_error": null,
        })),
        message: Some(reason.into()),
    }
}

fn ensure_test_quality_stage(
    stages: &mut Vec<VerificationStage>,
    max_mutants: Option<usize>,
    reason: &str,
) {
    if let Some(max_mutants) = max_mutants {
        if !stages.iter().any(|stage| stage.name == "test_quality") {
            stages.push(skipped_test_quality_stage(max_mutants, reason));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_test_quality_stage(
    code: &str,
    tests: &str,
    required_functions: &[&FunctionInfo],
    language: &Language,
    verification_context: &VerificationContext,
    opts: &VerifyOptions<'_>,
    candidate_source_file: Option<&str>,
    candidate_test_source_file: Option<&str>,
    baseline: &AuthoritativeTestOutcome,
    max_mutants: usize,
) -> VerificationStage {
    let started = Instant::now();
    let test_source_mode = candidate_test_source_file
        .map(|path| source_mode_for_path(Path::new(path), *language))
        .unwrap_or_else(|| SourceMode::for_language(language));
    let coupling_result = test_quality::analyze_coupling(
        code,
        tests,
        *language,
        test_source_mode,
        candidate_source_file,
        candidate_test_source_file,
    );
    let (coupling_findings, coupling_error) = match coupling_result {
        Ok(findings) => (findings, None),
        Err(error) => (Vec::new(), Some(sanitize_report_text(&error))),
    };
    let baseline_eligible = baseline.overlay.supported
        && baseline.test_ok
        && !baseline.has_non_target_blocker
        && baseline.covered_required == required_functions.len()
        && !required_functions.is_empty();
    let unsupported_overlay_reason = (!baseline.overlay.supported).then(|| {
        baseline
            .overlay
            .reason
            .clone()
            .unwrap_or_else(|| "authoritative-test instrumentation is unsupported".into())
    });
    let planning_limit = if max_mutants == 0 { 1 } else { max_mutants };
    let planned = match test_quality::plan_mutations(
        code,
        *language,
        verification_context.candidate.target_source.mode,
        required_functions,
        planning_limit,
    ) {
        Ok(planned) => planned,
        Err(error) => {
            let error = sanitize_report_text(&error);
            return VerificationStage {
                name: "test_quality".into(),
                status: StageStatus::Advisory,
                duration_ms: started.elapsed().as_millis() as u64,
                detail: Some(serde_json::json!({
                    "experimental": false,
                    "mode": "advisory",
                    "max_mutants": max_mutants,
                    "baseline_eligible": baseline_eligible,
                    "planning_error": error,
                    "counts": {
                        "planned": 0,
                        "killed": 0,
                        "survived": 0,
                        "invalid": 0,
                        "blocked": 0,
                        "no_coverage": 0,
                    },
                    "mutants": [],
                    "coupling_findings": coupling_findings,
                    "coupling_error": coupling_error,
                })),
                message: Some("Test-quality mutation planning failed".into()),
            };
        }
    };
    if planned.is_empty() {
        return VerificationStage {
            name: "test_quality".into(),
            status: if !baseline.overlay.supported
                || baseline.has_non_target_blocker
                || !coupling_findings.is_empty()
                || coupling_error.is_some()
            {
                StageStatus::Advisory
            } else {
                StageStatus::Skipped
            },
            duration_ms: started.elapsed().as_millis() as u64,
            detail: Some(serde_json::json!({
                "experimental": false,
                "mode": "advisory",
                "max_mutants": max_mutants,
                "baseline_eligible": baseline_eligible,
                "planning_error": null,
                "counts": {
                    "planned": 0,
                    "killed": 0,
                    "survived": 0,
                    "invalid": 0,
                    "blocked": 0,
                    "no_coverage": 0,
                },
                "mutants": [],
                "coupling_findings": coupling_findings,
                "coupling_error": coupling_error,
            })),
            message: Some("no bounded mutation candidates were available".into()),
        };
    }
    if max_mutants == 0 && !planned.is_empty() {
        return VerificationStage {
            name: "test_quality".into(),
            status: if !baseline.overlay.supported
                || baseline.has_non_target_blocker
                || !coupling_findings.is_empty()
                || coupling_error.is_some()
            {
                StageStatus::Advisory
            } else {
                StageStatus::Skipped
            },
            duration_ms: started.elapsed().as_millis() as u64,
            detail: Some(serde_json::json!({
                "experimental": false,
                "mode": "advisory",
                "max_mutants": 0,
                "baseline_eligible": baseline_eligible,
                "planning_error": null,
                "counts": {
                    "planned": 0,
                    "killed": 0,
                    "survived": 0,
                    "invalid": 0,
                    "blocked": 0,
                    "no_coverage": 0,
                },
                "mutants": [],
                "coupling_findings": coupling_findings,
                "coupling_error": coupling_error,
            })),
            message: Some("global CI mutation budget was exhausted before this file".into()),
        };
    }
    if !baseline_eligible {
        let status = if !baseline.overlay.supported
            || baseline.has_non_target_blocker
            || !coupling_findings.is_empty()
            || coupling_error.is_some()
        {
            StageStatus::Advisory
        } else {
            StageStatus::Skipped
        };
        return VerificationStage {
            name: "test_quality".into(),
            status,
            duration_ms: started.elapsed().as_millis() as u64,
            detail: Some(serde_json::json!({
                "experimental": false,
                "mode": "advisory",
                "max_mutants": max_mutants,
                "baseline_eligible": false,
                "planning_error": null,
                "counts": {
                    "planned": 0,
                    "killed": 0,
                    "survived": 0,
                    "invalid": 0,
                    "blocked": 0,
                    "no_coverage": 0,
                },
                "mutants": [],
                "coupling_findings": coupling_findings,
                "coupling_error": coupling_error,
            })),
            message: Some(if let Some(reason) = unsupported_overlay_reason {
                reason
            } else if baseline.has_non_target_blocker {
                "Mutation campaign skipped because the clean baseline was blocked by non-target test infrastructure"
                    .into()
            } else {
                "Mutation campaign skipped because the baseline test did not pass cleanly with complete target entry"
                    .into()
            }),
        };
    }

    let planned_count = planned.len();
    let mut killed = 0usize;
    let mut survived = 0usize;
    let mut invalid = 0usize;
    let mut blocked = 0usize;
    let mut no_coverage = 0usize;
    let mut observations = Vec::with_capacity(planned_count);
    for candidate in planned {
        let validated = match test_quality::validate_mutation(
            code,
            &candidate,
            &verification_context.candidate.target_source,
            required_functions,
        ) {
            Ok(validated) => validated,
            Err(error) => {
                invalid += 1;
                observations.push(serde_json::json!({
                    "mutation": candidate,
                    "outcome": "invalid",
                    "validation_kind": error.kind,
                    "reason": sanitize_report_text(&error.message),
                }));
                continue;
            }
        };
        let mutated_code = validated.code;
        let mutant_functions = validated.required_functions.iter().collect::<Vec<_>>();
        let outcome = run_authoritative_test(
            &mutated_code,
            tests,
            &mutant_functions,
            language,
            verification_context,
            opts,
            candidate_source_file,
            candidate_test_source_file,
            test_timeout(),
        )
        .await;
        let entered_mutated_surface = outcome.entered_surfaces.contains(&candidate.surface_id);
        let (classification, reason) =
            if !outcome.overlay.supported || outcome.has_non_target_blocker {
                blocked += 1;
                (
                    "blocked",
                    outcome
                        .overlay
                        .reason
                        .clone()
                        .unwrap_or_else(|| "test infrastructure blocked mutant execution".into()),
                )
            } else if !entered_mutated_surface {
                no_coverage += 1;
                (
                    "no_coverage",
                    "authoritative test did not enter the mutated surface".into(),
                )
            } else if outcome.test_ok {
                survived += 1;
                (
                    "survived",
                    format!(
                        "test passed after changing `{}` to `{}`; exercise the {}",
                        candidate.original, candidate.replacement, candidate.witness
                    ),
                )
            } else {
                killed += 1;
                (
                    "killed",
                    "authoritative test failed after entering the mutated surface".into(),
                )
            };
        observations.push(serde_json::json!({
            "mutation": candidate,
            "outcome": classification,
            "reason": reason,
            "entered_mutated_surface": entered_mutated_surface,
            "test_status": if outcome.has_non_target_blocker {
                "inconclusive"
            } else if outcome.test_ok {
                "passed"
            } else {
                "failed"
            },
            "exit_code": outcome.result.exit_code,
            "timed_out": outcome.result.timed_out,
            "memory_error": outcome.result.memory_error,
            "assertion_failure": outcome.has_assertion_failure,
            "failure_excerpt": clipped_test_failure(&outcome.result),
            "duration_ms": outcome.duration_ms,
        }));
    }

    let status = if survived > 0
        || !coupling_findings.is_empty()
        || invalid > 0
        || blocked > 0
        || no_coverage > 0
    {
        StageStatus::Advisory
    } else if planned_count > 0 && killed == planned_count {
        StageStatus::Passed
    } else {
        StageStatus::Skipped
    };
    let message = if survived > 0 {
        Some(format!(
            "{survived} behavior-changing mutant{} survived the authoritative test",
            if survived == 1 { "" } else { "s" }
        ))
    } else if !coupling_findings.is_empty() {
        Some(format!(
            "{} implementation-coupling finding{} require review",
            coupling_findings.len(),
            if coupling_findings.len() == 1 {
                ""
            } else {
                "s"
            }
        ))
    } else if invalid > 0 || blocked > 0 || no_coverage > 0 {
        Some("Some mutants could not be judged by the authoritative test".into())
    } else {
        None
    };
    VerificationStage {
        name: "test_quality".into(),
        status,
        duration_ms: started.elapsed().as_millis() as u64,
        detail: Some(serde_json::json!({
            "experimental": false,
            "mode": "advisory",
            "max_mutants": max_mutants,
            "baseline_eligible": true,
            "planning_error": null,
            "counts": {
                "planned": planned_count,
                "killed": killed,
                "survived": survived,
                "invalid": invalid,
                "blocked": blocked,
                "no_coverage": no_coverage,
            },
            "mutants": observations,
            "coupling_findings": coupling_findings,
            "coupling_error": coupling_error,
        })),
        message,
    }
}

/// Run the full verification pipeline: parse → complexity → lint → synthesize+execute → test.
pub async fn verify(
    code: &str,
    language: &Language,
    opts: VerifyOptions<'_>,
) -> VerificationReport {
    let mut stages = vec![];
    let suppressions = match parse_suppressions(opts.suppressions) {
        Ok(suppressions) => suppressions,
        Err(error) => {
            stages.push(VerificationStage {
                name: "configuration".into(),
                status: StageStatus::Inconclusive,
                duration_ms: 0,
                detail: Some(serde_json::json!({
                    "diagnostic": FailureDiagnostic {
                        domain: FailureDomain::Environment,
                        kind: FailureKind::InvalidConfiguration,
                        component: DiagnosticComponent::Configuration,
                        impact: DiagnosticImpact::Blocking,
                        message: error,
                        process: None,
                        limits: None,
                    },
                    "configuration_kind": "suppressions",
                    "suppression_source": opts.suppression_source,
                })),
                message: Some("Invalid suppression configuration; verification did not run".into()),
            });
            if !opts.tests_only {
                stages.push(skipped_execute_stage(
                    "invalid_suppressions",
                    "Execution skipped because suppression configuration is invalid",
                ));
            }
            ensure_test_quality_stage(
                &mut stages,
                opts.test_quality_max_mutants,
                "Mutation campaign skipped because suppression configuration is invalid",
            );
            return finalize_report(
                build_report(stages, opts.coverage_gate, code, opts.source_file),
                opts.output_dir,
                opts.source_file,
                language,
                opts.report_level,
            );
        }
    };
    let verification_context = match resolve_verification_contexts(&opts, language) {
        Ok(context) => context,
        Err(error) => {
            stages.push(VerificationStage {
                name: "context".into(),
                status: StageStatus::Inconclusive,
                duration_ms: 0,
                detail: Some(serde_json::json!({
                    "diagnostic": {
                        "domain": "environment",
                        "kind": "context_resolution",
                        "message": error,
                    }
                })),
                message: Some("Unable to resolve source or project context".into()),
            });
            if !opts.tests_only {
                stages.push(skipped_execute_stage(
                    "context_unavailable",
                    "Execution skipped because verification context could not be resolved",
                ));
            }
            ensure_test_quality_stage(
                &mut stages,
                opts.test_quality_max_mutants,
                "Mutation campaign skipped because verification context was unavailable",
            );
            return finalize_report(
                build_report(stages, opts.coverage_gate, code, opts.source_file),
                opts.output_dir,
                opts.source_file,
                language,
                opts.report_level,
            );
        }
    };
    // All subsequent stages consume the resolved context rather than reinterpreting
    // the caller's possibly-relative paths. Keep owned spellings alive for APIs
    // whose compatibility boundary still accepts borrowed strings.
    let candidate_project_dir_owned = verification_context
        .candidate
        .workspace_root
        .to_string_lossy()
        .into_owned();
    let candidate_source_file_owned = verification_context
        .candidate
        .target_source
        .source_file
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let explicit_test_source_file_owned = verification_context
        .candidate
        .test_source
        .as_ref()
        .and_then(|source| source.source_file.as_ref())
        .map(|path| path.to_string_lossy().into_owned());
    let discovered_test_source_file_owned = (opts.test_code.is_none() && opts.auto_seed)
        .then(|| {
            candidate_source_file_owned
                .as_deref()
                .and_then(|source| discover_seed_files(source, language).into_iter().next())
                .map(|path| path.to_string_lossy().into_owned())
        })
        .flatten();
    let candidate_test_source_file_owned = explicit_test_source_file_owned
        .clone()
        .or(discovered_test_source_file_owned);
    let discovered_test_code_owned = (opts.test_code.is_none())
        .then(|| {
            candidate_test_source_file_owned
                .as_deref()
                .and_then(|path| std::fs::read_to_string(path).ok())
        })
        .flatten();
    let effective_test_code = opts.test_code.or(discovered_test_code_owned.as_deref());
    let candidate_virtual_file_path_owned = verification_context
        .candidate
        .target_source
        .virtual_file_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let base_project_dir_owned = verification_context
        .base
        .as_ref()
        .map(|context| context.workspace_root.to_string_lossy().into_owned());
    let base_source_file_owned = verification_context
        .base
        .as_ref()
        .and_then(|context| context.target_source.source_file.as_ref())
        .map(|path| path.to_string_lossy().into_owned());
    let candidate_embedded_project_dir = candidate_source_file_owned
        .as_ref()
        .map(|_| candidate_project_dir_owned.as_str());
    let base_embedded_project_dir = base_source_file_owned
        .as_ref()
        .and(base_project_dir_owned.as_deref());

    let start = Instant::now();
    let analysis =
        analyze::analyze_with_context(code, &verification_context.candidate.target_source);
    let parse_ms = start.elapsed().as_millis() as u64;

    if analysis.parse_error {
        let unsupported = analysis
            .parse_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "unsupported");
        let message = analysis
            .parse_diagnostics
            .first()
            .map(|diagnostic| {
                format!(
                    "{} at {}:{}",
                    diagnostic.message, diagnostic.start_line, diagnostic.start_column
                )
            })
            .unwrap_or_else(|| "Code contains syntax errors".into());
        stages.push(VerificationStage {
            name: "parse".into(),
            status: if unsupported {
                StageStatus::Inconclusive
            } else {
                StageStatus::Failed
            },
            duration_ms: parse_ms,
            detail: Some(serde_json::to_value(&analysis).unwrap()),
            message: Some(message),
        });
        if !opts.tests_only {
            stages.push(skipped_execute_stage(
                if unsupported {
                    "analysis_depth_unsupported"
                } else {
                    "parse_failed"
                },
                if unsupported {
                    "Execution skipped because source nesting exceeds the analysis safety limit"
                } else {
                    "Execution skipped because the source did not parse"
                },
            ));
        }
        ensure_test_quality_stage(
            &mut stages,
            opts.test_quality_max_mutants,
            "Mutation campaign skipped because the source did not parse",
        );
        return finalize_report(
            build_report(stages, opts.coverage_gate, code, opts.source_file),
            opts.output_dir,
            opts.source_file,
            language,
            opts.report_level,
        );
    }

    stages.push(VerificationStage {
        name: "parse".into(),
        status: StageStatus::Passed,
        duration_ms: parse_ms,
        detail: Some(serde_json::to_value(&analysis).unwrap()),
        message: None,
    });

    // Stage 2: Complexity threshold (optional)
    if let Some(threshold) = opts.complexity_threshold {
        let start = Instant::now();
        let (functions_checked, diff_scoped) = if let Some(diff_str) = opts.diff {
            let changed_ranges = candidate_source_file_owned
                .as_deref()
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            (
                analyze::filter_changed_functions(&analysis, &changed_ranges),
                true,
            )
        } else {
            (analysis.functions.clone(), false)
        };
        let (violations, suppressed_violations, source_directive_functions) =
            split_complexity_violations(
                analyze::check_complexity_threshold_for_functions_with_metric(
                    &functions_checked,
                    threshold,
                    opts.complexity_metric,
                ),
                &suppressions,
                candidate_source_file_owned.as_deref(),
                code,
                language,
            );
        let complexity_ms = start.elapsed().as_millis() as u64;
        let complexity_ok = violations.is_empty();
        stages.push(VerificationStage {
            name: "complexity".into(),
            status: if complexity_ok { StageStatus::Passed } else { StageStatus::Failed },
            duration_ms: complexity_ms,
            detail: Some(serde_json::json!({
                "violations": serde_json::to_value(&violations).unwrap(),
                "suppressed_violations": serde_json::to_value(&suppressed_violations).unwrap(),
                "threshold": threshold,
                "metric": serde_json::to_value(opts.complexity_metric).unwrap(),
                "checked_functions": functions_checked.len(),
                "diff_scoped": diff_scoped,
                "complexity_ok": complexity_ok,
                "suppression_source": opts.suppression_source,
                "source_directive_functions": serde_json::to_value(&source_directive_functions).unwrap(),
                "source_directive_suppression_count": source_directive_functions.len(),
            })),
            message: if complexity_ok {
                None
            } else {
                Some(format!(
                    "{} function(s) exceed complexity threshold {}",
                    violations.len(),
                    threshold,
                ))
            },
        });
    }

    // Stage 3: Lint — informational unless the lint runner itself errors.
    let start = Instant::now();
    let mut lint_result = lint::lint_with_options(
        code,
        language,
        lint::LintOptions {
            source_file: candidate_source_file_owned.as_deref(),
            project_dir: Some(candidate_project_dir_owned.as_str()),
            config_path: opts.lint_config_path,
            virtual_file_path: candidate_virtual_file_path_owned.as_deref(),
        },
    )
    .await;
    let lint_ms = start.elapsed().as_millis() as u64;

    // Filter out false positives only when linting anonymous inline snippets.
    if candidate_source_file_owned.is_none() && candidate_virtual_file_path_owned.is_none() {
        lint_result.diagnostics.retain(|d| {
            !matches!(
                d.rule.as_str(),
                "lint/correctness/noUnusedVariables" | "F401" | "F841"
            )
        });
    }

    let lint_runner_failed = lint_result.runner_failed;

    stages.push(VerificationStage {
        name: "lint".into(),
        status: StageStatus::Advisory,
        duration_ms: lint_ms,
        detail: Some(serde_json::to_value(&lint_result).unwrap()),
        message: if lint_runner_failed {
            lint_result.error.clone()
        } else {
            None
        },
    });
    let project_adapter = project_adapter_contract(&verification_context.candidate);
    let defer_generated_execution_to_project_runner =
        project_adapter.kind == ProjectAdapterKind::Nuxt && effective_test_code.is_some();

    if opts.tests_only && effective_test_code.is_none() {
        stages.push(VerificationStage {
            name: "test".into(),
            status: StageStatus::Inconclusive,
            duration_ms: 0,
            detail: None,
            message: Some("tests_only mode requires an authoritative test".into()),
        });
        ensure_test_quality_stage(
            &mut stages,
            opts.test_quality_max_mutants,
            "Mutation campaign skipped because no authoritative test was available",
        );
        return finalize_report(
            build_report(stages, opts.coverage_gate, code, opts.source_file),
            opts.output_dir,
            opts.source_file,
            language,
            opts.report_level,
        );
    }

    // Stage 4: Synthesize + Execute
    if !opts.tests_only && !analysis.functions.is_empty() {
        // Determine which functions to fuzz
        let mut functions_to_fuzz: Vec<FunctionInfo> = if let Some(diff_str) = opts.diff {
            let changed_ranges = candidate_source_file_owned
                .as_deref()
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            analyze::filter_changed_functions(&analysis, &changed_ranges)
        } else {
            analysis.functions.clone()
        };
        // A diff selects execution owners, not fragments of their declaration
        // context. An edited factory still needs unchanged nested signatures
        // to exercise its returned actions. Keep those declarations available
        // without admitting unrelated top-level functions to the campaign.
        let nested_context = analysis
            .functions
            .iter()
            .filter(|nested| {
                nested.is_nested
                    && functions_to_fuzz.iter().any(|owner| {
                        !owner.is_nested
                            && owner.line <= nested.line
                            && owner.end_line >= nested.end_line
                    })
                    && !functions_to_fuzz.iter().any(|selected| {
                        selected.name == nested.name && selected.line == nested.line
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        functions_to_fuzz.extend(nested_context);
        let mut fixture_rows = Vec::new();
        let mut inferred_fixture_properties: HashMap<String, Vec<String>> = HashMap::new();
        let mut inferred_context_properties: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_classes = analysis.classes.clone();
        let mut all_aliases = analysis.aliases.clone();
        if opts.auto_seed {
            if let Some(source_file) = candidate_source_file_owned.as_deref() {
                let function_names: HashSet<String> = functions_to_fuzz
                    .iter()
                    .map(|func| func.name.clone())
                    .collect();
                let fixture_project_dir_owned = opts.project_dir.map(|project| {
                    let path = Path::new(project);
                    if path.is_absolute() {
                        project.to_string()
                    } else {
                        verification_context
                            .candidate
                            .invocation_dir
                            .join(path)
                            .to_string_lossy()
                            .into_owned()
                    }
                });
                let project_dir = fixture_project_dir_owned
                    .as_deref()
                    .or(Some(candidate_project_dir_owned.as_str()));
                fixture_rows = json_fixture_rows(source_file, project_dir, &function_names);
                inferred_fixture_properties = infer_fixture_properties(&fixture_rows);
                inferred_context_properties =
                    infer_context_properties(source_file, project_dir, &functions_to_fuzz);
                apply_inferred_properties(&mut functions_to_fuzz, &inferred_fixture_properties);
                apply_inferred_properties(&mut functions_to_fuzz, &inferred_context_properties);
                let referenced_names =
                    analyze::referenced_type_names_for_functions(&functions_to_fuzz);
                let imported = analyze::resolve_imported_types_for_names(
                    &analysis,
                    source_file,
                    language,
                    &referenced_names,
                );
                all_classes.extend(imported.classes);
                all_aliases.extend(imported.aliases);
            }
        }
        let observed_calls = collect_seed_observations(
            code,
            language,
            verification_context.candidate.target_source.mode,
            &functions_to_fuzz,
            candidate_source_file_owned.as_deref(),
            Some(candidate_project_dir_owned.as_str()),
            effective_test_code,
            candidate_test_source_file_owned.as_deref(),
            opts.auto_seed,
        );
        let observed_calls = observed_calls
            .into_iter()
            .map(|mut observed| {
                observed.source_label = display_seed_source_label(
                    &observed.source_label,
                    opts.source_file,
                    opts.project_dir,
                );
                observed
            })
            .collect::<Vec<_>>();
        let mut seed_sources = seed_sources(&observed_calls);
        for row in &fixture_rows {
            if !seed_sources.contains(&row.source_file) {
                seed_sources.push(row.source_file.clone());
            }
        }
        let surface_ids: HashMap<&str, String> = functions_to_fuzz
            .iter()
            .map(|function| {
                (
                    function.name.as_str(),
                    format!("{}:{}", function.name, function.line),
                )
            })
            .collect();
        let caller_examples = observed_calls
            .iter()
            .filter_map(|observed| {
                let target_surface_id = surface_ids.get(observed.function.as_str())?.clone();
                Some(CallerExample {
                    caller: observed.source_label.clone(),
                    target_surface_id,
                    source_file: observed.source_label.clone(),
                    line: 0,
                    arguments: PlannedArguments {
                        positional: observed
                            .args
                            .iter()
                            .map(|argument| DomainLiteral {
                                expression: argument.code.clone(),
                                json_value: argument.literal_value.clone(),
                            })
                            .collect(),
                        named: BTreeMap::new(),
                    },
                    evidence: CallerEvidence::StaticSyntax,
                })
            })
            .collect::<Vec<_>>();
        let fixture_examples = fixture_rows
            .iter()
            .filter_map(|row| {
                let target_surface_id = surface_ids.get(row.function.as_str())?.clone();
                Some(FixtureExample {
                    target_surface_id,
                    source_file: row.source_file.clone(),
                    line: row.line,
                    arguments: PlannedArguments {
                        positional: row
                            .args
                            .iter()
                            .map(|argument| DomainLiteral {
                                expression: json_value_to_literal(argument, language),
                                json_value: Some(argument.clone()),
                            })
                            .collect(),
                        named: BTreeMap::new(),
                    },
                    expected: Some(DomainLiteral {
                        expression: json_value_to_literal(&row.expected, language),
                        json_value: Some(row.expected.clone()),
                    }),
                })
            })
            .collect::<Vec<_>>();
        let mut inferred_properties = Vec::new();
        for (symbol, properties) in &inferred_fixture_properties {
            let Some(surface_id) = surface_ids.get(symbol.as_str()) else {
                continue;
            };
            let fixture_source = fixture_rows.iter().find(|row| row.function == *symbol);
            for property in properties {
                inferred_properties.push(InferredProperty {
                    target_surface_id: surface_id.clone(),
                    contract_id: property.clone(),
                    source_file: fixture_source.map(|row| row.source_file.clone()),
                    line: fixture_source.map(|row| row.line),
                    evidence: CallerEvidence::AuthoritativeFixture,
                });
            }
        }
        for (symbol, properties) in &inferred_context_properties {
            let Some(surface_id) = surface_ids.get(symbol.as_str()) else {
                continue;
            };
            for property in properties {
                inferred_properties.push(InferredProperty {
                    target_surface_id: surface_id.clone(),
                    contract_id: property.clone(),
                    source_file: candidate_source_file_owned.clone(),
                    line: None,
                    evidence: CallerEvidence::StaticSyntax,
                });
            }
        }
        let mut verification_plan = domain::build_verification_plan(
            &functions_to_fuzz,
            &all_classes,
            &all_aliases,
            language,
            &caller_examples,
            &fixture_examples,
            &inferred_properties,
        );
        let context_dependent_surfaces = if matches!(language, Language::TypeScript) {
            react_hook_surface_ids(code, &functions_to_fuzz)
        } else {
            HashSet::new()
        };
        exclude_context_dependent_surfaces(&mut verification_plan, &context_dependent_surfaces);
        let corpus_path = persistent_corpus_path(
            opts.output_dir,
            candidate_source_file_owned.as_deref(),
            language,
        );
        let persistent_corpus = corpus_path
            .as_deref()
            .map(read_persistent_corpus)
            .unwrap_or_default();
        let corpus_loaded_count = persistent_corpus.values().map(Vec::len).sum::<usize>();
        verification_plan.inputs.extend(corpus_inputs(
            &persistent_corpus,
            &functions_to_fuzz,
            language,
            candidate_source_file_owned.as_deref(),
        ));
        let mut corpus_retained_count = corpus_loaded_count;
        let seed_input_count = verification_plan
            .inputs
            .iter()
            .filter(|input| {
                input.sources.iter().any(|source| {
                    matches!(
                        source.kind,
                        DomainSourceKind::ObservedCall | DomainSourceKind::JsonFixture
                    )
                })
            })
            .count();
        let seeded_function_count = verification_plan
            .inputs
            .iter()
            .filter(|input| {
                input.sources.iter().any(|source| {
                    matches!(
                        source.kind,
                        DomainSourceKind::ObservedCall | DomainSourceKind::JsonFixture
                    )
                })
            })
            .map(|input| input.surface_id.as_str())
            .collect::<HashSet<_>>()
            .len();

        let synth_start = Instant::now();
        let fuzz_plan = synthesize::synthesize_plan_for_verification(
            &functions_to_fuzz,
            &all_classes,
            &all_aliases,
            language,
            &verification_plan,
        );
        let coverage_ms = synth_start.elapsed().as_millis() as u64;
        let native_config = native_fuzz_config();
        let mut module_load_blocked = false;
        let execution_plans = surface_execution_plans(
            &analysis.functions,
            &project_adapter,
            &fuzz_plan.coverage,
            opts.tests_only,
            effective_test_code.is_some(),
        );
        stages.push(VerificationStage {
            name: "project_adapter".into(),
            status: StageStatus::Passed,
            duration_ms: 0,
            detail: Some(serde_json::json!({
                "adapter": &project_adapter,
                "surfaces": execution_plans,
            })),
            message: None,
        });
        if defer_generated_execution_to_project_runner {
            let mut coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                false,
            );
            apply_context_dependent_coverage(&mut coverage, &context_dependent_surfaces);
            for function in &mut coverage {
                if matches!(
                    function.status,
                    FuzzFunctionStatus::CheckedDirect
                        | FuzzFunctionStatus::ReachedViaFactory
                        | FuzzFunctionStatus::CheckedViaFactory
                        | FuzzFunctionStatus::CheckedViaCaller
                ) {
                    function.status = FuzzFunctionStatus::SkippedNoFuzzableSurface;
                    function.reason =
                        Some("surface delegated to the Nuxt project test runner".into());
                }
            }
            stages.push(VerificationStage {
                name: "coverage".into(),
                status: StageStatus::Inconclusive,
                duration_ms: coverage_ms,
                detail: Some(serde_json::json!({
                    "functions": serde_json::to_value(&coverage).unwrap(),
                    "counts": coverage_counts(&coverage),
                    "diff_scoped": opts.diff.is_some(),
                    "seed_input_count": seed_input_count,
                    "seeded_functions": seeded_function_count,
                    "seed_sources": seed_sources,
                    "inferred_fixture_properties": inferred_fixture_properties,
                    "inferred_context_properties": inferred_context_properties,
                    "auto_seed": opts.auto_seed,
                    "verification_plan": &verification_plan,
                    "corpus_loaded": corpus_loaded_count,
                    "corpus_retained": corpus_retained_count,
                })),
                message: Some(
                    "generated execution deferred to the Nuxt project test runner".into(),
                ),
            });
            stages.push(skipped_execute_stage(
                "project_runner_selected",
                "Generated execution skipped because the Nuxt project runner owns this surface",
            ));
        } else if !fuzz_plan.code.is_empty() || native_config.engine != NativeFuzzEngine::Off {
            let full_code = generated_verifier_source(
                &verification_context.candidate,
                code,
                language,
                &functions_to_fuzz,
                &analysis.classes,
                &fuzz_plan.coverage,
                &fuzz_plan.code,
            );
            let execute_timeout = execute_timeout_for(language);

            let start = Instant::now();
            let (_, runtime_name, _) =
                generated_harness_runtime(verification_context.candidate.target_source.mode);
            let mut exec_runtime = Some(runtime_name.to_string());
            let harness_execution = if fuzz_plan.code.is_empty() {
                // Native engines own their input decoders. A missing ordinary
                // campaign must not prevent an independently requested engine
                // from running, nor count as a successful generated invocation.
                exec_runtime = None;
                HarnessExecution {
                    process: ExecutionResult {
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: None,
                        duration_ms: 0,
                        timed_out: false,
                        memory_error: false,
                        termination: None,
                        diagnostics: Vec::new(),
                    },
                    diagnostics: Vec::new(),
                }
            } else {
                execute_generated_harness(
                    &verification_context.candidate,
                    full_code,
                    HarnessKind::GeneratedVerifier,
                    &opts,
                    language,
                    execute_timeout,
                    Some(candidate_project_dir_owned.as_str()),
                    candidate_source_file_owned.as_deref(),
                )
                .await
            };
            let mut harness_diagnostics = harness_execution.diagnostics;
            let mut exec_result = harness_execution.process;
            let mut generated_failures = parse_findings(&exec_result.stdout).unwrap_or_default();
            let mut corpus_update = parse_corpus(&exec_result.stdout);
            corpus_retained_count = persist_corpus(corpus_path.as_deref(), &corpus_update);
            let nuxt_runtime_blocker = take_nuxt_runtime_failures(
                &verification_context.candidate,
                &mut generated_failures,
                &exec_result.stderr,
            );
            if let Some(blocker) = &nuxt_runtime_blocker {
                harness_diagnostics.retain(|diagnostic| {
                    !matches!(
                        diagnostic.kind,
                        FailureKind::HarnessProtocol | FailureKind::NonzeroExit
                    )
                });
                exec_result.diagnostics.retain(|diagnostic| {
                    !matches!(
                        diagnostic.kind,
                        FailureKind::HarnessProtocol | FailureKind::NonzeroExit
                    )
                });
                exec_result.stdout = normalize_nuxt_runtime_stdout(
                    &exec_result.stdout,
                    &generated_failures,
                    blocker,
                );
                let diagnostic = blocker.diagnostic();
                exec_result.diagnostics.push(diagnostic.clone());
                harness_diagnostics.push(diagnostic);
            }
            let exec_ms = start.elapsed().as_millis() as u64;
            let launch_runtime = match exec_runtime.as_deref() {
                Some("bun") | Some("bun-test") => HarnessRuntime::BunScript,
                Some("tsx") | Some("tsx-script") => HarnessRuntime::TsxScript,
                Some("vitest") => HarnessRuntime::Vitest,
                Some("jest") => HarnessRuntime::Jest,
                Some("node") | Some("node-script") => HarnessRuntime::NodeScript,
                Some("python") | Some("python3") => HarnessRuntime::Python,
                _ => match language {
                    Language::Python => HarnessRuntime::Python,
                    Language::TypeScript => HarnessRuntime::NodeScript,
                },
            };
            let launch_context = ReproLaunchContext {
                limits: ExecutionLimits {
                    timeout_seconds: execute_timeout,
                    memory_mb: opts.memory_mb,
                    runtime_profile: opts.runtime_profile,
                    network_policy: opts.network,
                },
                source_mode: verification_context.candidate.target_source.mode,
                runtime: launch_runtime,
                base_source_mode: verification_context
                    .base
                    .as_ref()
                    .map(|context| context.target_source.mode),
                base_runtime: None,
                harness_args: opts.harness_args.clone(),
                docker_image: (opts.runtime_profile == RuntimeProfile::Isolated).then(|| {
                    match language {
                        Language::Python => opts.python_docker_image.to_string(),
                        Language::TypeScript => opts.typescript_docker_image.to_string(),
                    }
                }),
            };
            module_load_blocked = nuxt_runtime_blocker
                .as_ref()
                .is_some_and(|blocker| blocker.blocked_before_harness)
                || (matches!(language, Language::TypeScript)
                    && is_typescript_module_load_error(&exec_result.stderr)
                    && !exec_result
                        .stdout
                        .lines()
                        .any(|line| line.starts_with("FUZZ ")));
            let mut coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
            apply_context_dependent_coverage(&mut coverage, &context_dependent_surfaces);
            apply_runtime_coverage_proof(&mut coverage, &exec_result);
            if let Some(blocker) = &nuxt_runtime_blocker {
                apply_nuxt_runtime_coverage(&mut coverage, blocker);
            }
            stages.push(VerificationStage {
                name: "coverage".into(),
                status: StageStatus::Passed,
                duration_ms: coverage_ms,
                detail: Some(serde_json::json!({
                    "functions": serde_json::to_value(&coverage).unwrap(),
                    "counts": coverage_counts(&coverage),
                    "diff_scoped": opts.diff.is_some(),
                    "seed_input_count": seed_input_count,
                    "seeded_functions": seeded_function_count,
                    "seed_sources": seed_sources,
                    "inferred_fixture_properties": inferred_fixture_properties,
                    "inferred_context_properties": inferred_context_properties,
                    "auto_seed": opts.auto_seed,
                    "verification_plan": &verification_plan,
                    "corpus_loaded": corpus_loaded_count,
                    "corpus_retained": corpus_retained_count,
                    "corpus_novel": corpus_retained_count.saturating_sub(corpus_loaded_count),
                })),
                message: None,
            });

            let (differential_findings, differential_detail) = if let Some(base_code) =
                opts.base_code
            {
                let baseline_analysis = verification_context
                    .base
                    .as_ref()
                    .map(|context| analyze::analyze_with_context(base_code, &context.target_source))
                    .expect("base context is resolved whenever base code is present");
                if baseline_analysis.parse_error {
                    (
                        Vec::new(),
                        serde_json::json!({
                            "enabled": false,
                            "reason": "baseline_parse_error",
                            "units": []
                        }),
                    )
                } else {
                    let (relative_entry, candidate_files) = embedded_project_sources(
                        candidate_embedded_project_dir,
                        candidate_source_file_owned.as_deref(),
                        code,
                        language,
                    );
                    let (base_relative_entry, base_files) = embedded_project_sources(
                        base_embedded_project_dir,
                        base_source_file_owned.as_deref(),
                        base_code,
                        language,
                    );
                    if relative_entry != base_relative_entry {
                        (
                            Vec::new(),
                            serde_json::json!({
                                "enabled": false,
                                "reason": "baseline_entry_path_mismatch",
                                "candidate_entry": relative_entry,
                                "base_entry": base_relative_entry,
                                "units": []
                            }),
                        )
                    } else {
                        let baseline_by_name = baseline_analysis
                            .functions
                            .iter()
                            .map(|function| (function.name.as_str(), function))
                            .collect::<HashMap<_, _>>();
                        let mut findings = Vec::new();
                        let mut units = Vec::new();
                        let mut differential_diagnostics = Vec::new();
                        let base_context = verification_context
                            .base
                            .as_ref()
                            .expect("base context is resolved whenever base code is present");
                        for candidate_function in analysis.functions.iter().filter(|function| {
                            function.is_exported && !function.is_method && !function.is_nested
                        }) {
                            let Some(baseline_function) = baseline_by_name
                                .get(candidate_function.name.as_str())
                                .copied()
                            else {
                                units.push(serde_json::json!({ "surface": format!("{}:{}", candidate_function.name, candidate_function.line), "status": "disabled", "reason": "missing_base_surface" }));
                                continue;
                            };
                            if !compatible_surface(candidate_function, baseline_function) {
                                units.push(serde_json::json!({ "surface": format!("{}:{}", candidate_function.name, candidate_function.line), "status": "disabled", "reason": "incompatible_signature" }));
                                continue;
                            }
                            let Some(differential_case) =
                                differential_case(candidate_function, language)
                            else {
                                units.push(serde_json::json!({ "surface": format!("{}:{}", candidate_function.name, candidate_function.line), "status": "disabled", "reason": "no_deterministic_valid_case" }));
                                continue;
                            };
                            let candidate_probe = differential_probe(
                                code,
                                candidate_function,
                                &differential_case,
                                language,
                            );
                            let baseline_probe = differential_probe(
                                base_code,
                                baseline_function,
                                &differential_case,
                                language,
                            );
                            let candidate_execution = execute_generated_harness(
                                &verification_context.candidate,
                                candidate_probe,
                                HarnessKind::Standalone,
                                &opts,
                                language,
                                execute_timeout,
                                Some(candidate_project_dir_owned.as_str()),
                                candidate_source_file_owned.as_deref(),
                            )
                            .await;
                            differential_diagnostics.extend(candidate_execution.diagnostics);
                            let candidate_result = candidate_execution.process;
                            let baseline_execution = execute_generated_harness(
                                base_context,
                                baseline_probe,
                                HarnessKind::Standalone,
                                &opts,
                                language,
                                execute_timeout,
                                base_project_dir_owned.as_deref(),
                                base_source_file_owned.as_deref(),
                            )
                            .await;
                            differential_diagnostics.extend(baseline_execution.diagnostics);
                            let baseline_result = baseline_execution.process;
                            let surface =
                                format!("{}:{}", candidate_function.name, candidate_function.line);
                            match (
                                differential_snapshot(&candidate_result),
                                differential_snapshot(&baseline_result),
                            ) {
                                (Ok(candidate_snapshot), Ok(baseline_snapshot))
                                    if differential_binding_failure(
                                        &candidate_snapshot,
                                        language,
                                    ) || differential_binding_failure(
                                        &baseline_snapshot,
                                        language,
                                    ) =>
                                {
                                    units.push(serde_json::json!({ "surface": surface, "status": "disabled", "reason": "invalid_generated_invocation:binding_exception" }));
                                }
                                (Ok(candidate_snapshot), Ok(baseline_snapshot))
                                    if candidate_snapshot == baseline_snapshot
                                        && candidate_snapshot.exception_type.is_some() =>
                                {
                                    units.push(serde_json::json!({ "surface": surface, "status": "disabled", "reason": "invalid_generated_invocation:identical_runtime_exception" }));
                                }
                                (Ok(candidate_snapshot), Ok(baseline_snapshot))
                                    if candidate_snapshot == baseline_snapshot =>
                                {
                                    units.push(serde_json::json!({ "surface": surface, "status": "equal" }));
                                }
                                (Ok(candidate_snapshot), Ok(baseline_snapshot)) => {
                                    findings.push(differential_finding(
                                        candidate_source_file_owned
                                            .as_deref()
                                            .unwrap_or("<inline>"),
                                        candidate_function,
                                        differential_case.repro_arguments(),
                                        &candidate_snapshot,
                                        &baseline_snapshot,
                                        language,
                                        relative_entry.clone(),
                                        base_files.clone(),
                                        candidate_files.clone(),
                                    ));
                                    units.push(serde_json::json!({ "surface": surface, "status": "different" }));
                                }
                                (Err(candidate_reason), Err(baseline_reason)) => {
                                    units.push(serde_json::json!({ "surface": surface, "status": "disabled", "reason": format!("unsupported_snapshot:candidate={candidate_reason};baseline={baseline_reason}") }));
                                }
                                (Err(candidate_reason), _) => {
                                    units.push(serde_json::json!({ "surface": surface, "status": "disabled", "reason": format!("unsupported_snapshot:candidate={candidate_reason}") }));
                                }
                                (_, Err(baseline_reason)) => {
                                    units.push(serde_json::json!({ "surface": surface, "status": "disabled", "reason": format!("unsupported_snapshot:baseline={baseline_reason}") }));
                                }
                            }
                        }
                        (
                            findings,
                            serde_json::json!({
                                "enabled": true,
                                "relative_entry": relative_entry,
                                "units": units,
                                "diagnostics": differential_diagnostics,
                            }),
                        )
                    }
                }
            } else {
                (
                    Vec::new(),
                    serde_json::json!({ "enabled": false, "reason": "no_baseline", "units": [] }),
                )
            };

            if opts.base_code.is_some() {
                stages.push(VerificationStage {
                    name: "differential".into(),
                    status: StageStatus::Advisory,
                    duration_ms: 0,
                    detail: Some(serde_json::json!({
                        "findings": &differential_findings,
                        "comparison": &differential_detail,
                    })),
                    message: None,
                });
            }

            if let Some(command) = llm_plateau_command() {
                let plateaued = corpus_loaded_count > 0
                    && corpus_retained_count <= corpus_loaded_count
                    && generated_failures.is_empty();
                if !plateaued {
                    stages.push(VerificationStage {
                        name: "llm_plateau_escape".into(),
                        status: StageStatus::Skipped,
                        duration_ms: 0,
                        detail: Some(serde_json::json!({
                            "reason": if corpus_loaded_count == 0 {
                                "insufficient_corpus_history"
                            } else if !generated_failures.is_empty() {
                                "behavioral_failure_already_found"
                            } else {
                                "corpus_progressing"
                            },
                            "corpus_loaded": corpus_loaded_count,
                            "corpus_retained": corpus_retained_count,
                        })),
                        message: None,
                    });
                } else {
                    match propose_llm_plateau_seeds(
                        &command,
                        language,
                        &functions_to_fuzz,
                        &persistent_corpus,
                        Some(candidate_project_dir_owned.as_str()),
                    )
                    .await
                    {
                        Err(error) => stages.push(VerificationStage {
                            name: "llm_plateau_escape".into(),
                            status: StageStatus::Advisory,
                            duration_ms: 0,
                            detail: Some(serde_json::json!({
                                "reason": "proposal_command_failed",
                                "error": error,
                            })),
                            message: Some(
                                "LLM plateau escape was requested but did not produce usable seeds"
                                    .into(),
                            ),
                        }),
                        Ok(mut proposal) => {
                            for (surface_id, retained_rows) in &persistent_corpus {
                                if let Some(rows) = proposal.corpus.get_mut(surface_id) {
                                    rows.retain(|row| !retained_rows.contains(row));
                                }
                            }
                            proposal.corpus.retain(|_, rows| !rows.is_empty());
                            if proposal.corpus.is_empty() {
                                stages.push(VerificationStage {
                                    name: "llm_plateau_escape".into(),
                                    status: StageStatus::Advisory,
                                    duration_ms: proposal.duration_ms,
                                    detail: Some(serde_json::json!({
                                        "reason": "no_novel_valid_seeds",
                                        "proposed": proposal.proposed_count,
                                        "invalid": proposal.invalid_count,
                                        "stderr": proposal.stderr,
                                    })),
                                    message: Some(
                                        "LLM plateau escape produced no novel valid seed".into(),
                                    ),
                                });
                            } else {
                                let accepted_count =
                                    proposal.corpus.values().map(Vec::len).sum::<usize>();
                                let mut plateau_plan = verification_plan.clone();
                                let mut plateau_inputs = corpus_inputs(
                                    &proposal.corpus,
                                    &functions_to_fuzz,
                                    language,
                                    candidate_source_file_owned.as_deref(),
                                );
                                plateau_inputs.append(&mut plateau_plan.inputs);
                                plateau_plan.inputs = plateau_inputs;
                                let plateau_fuzz_plan =
                                    synthesize::synthesize_plan_for_verification(
                                        &functions_to_fuzz,
                                        &all_classes,
                                        &all_aliases,
                                        language,
                                        &plateau_plan,
                                    );
                                let plateau_code = generated_verifier_source(
                                    &verification_context.candidate,
                                    code,
                                    language,
                                    &functions_to_fuzz,
                                    &all_classes,
                                    &plateau_fuzz_plan.coverage,
                                    &plateau_fuzz_plan.code,
                                );
                                let plateau_execution = execute_generated_harness(
                                    &verification_context.candidate,
                                    plateau_code,
                                    HarnessKind::GeneratedVerifier,
                                    &opts,
                                    language,
                                    execute_timeout,
                                    Some(candidate_project_dir_owned.as_str()),
                                    candidate_source_file_owned.as_deref(),
                                )
                                .await;
                                let plateau_process = plateau_execution.process;
                                let plateau_findings =
                                    parse_findings(&plateau_process.stdout).unwrap_or_default();
                                let plateau_corpus = parse_corpus(&plateau_process.stdout);
                                for (surface_id, rows) in proposal
                                    .corpus
                                    .into_iter()
                                    .chain(plateau_corpus.into_iter())
                                {
                                    let retained = corpus_update.entry(surface_id).or_default();
                                    for row in rows {
                                        if !retained.contains(&row) {
                                            retained.push(row);
                                        }
                                    }
                                }
                                corpus_retained_count =
                                    persist_corpus(corpus_path.as_deref(), &corpus_update);
                                let execution_ok = plateau_process.exit_code == Some(0)
                                    && !plateau_process.timed_out
                                    && !plateau_process.memory_error;
                                let gating_finding_count = plateau_findings
                                    .iter()
                                    .filter(|finding| {
                                        finding_fails_execute_gate(
                                            opts.execute_gate,
                                            finding,
                                            opts.inferred_oracle_gate,
                                        )
                                    })
                                    .count();
                                let unknown_finding_count = plateau_findings
                                    .iter()
                                    .filter(|finding| {
                                        finding.input_classification == InputClassification::Unknown
                                    })
                                    .count();
                                let status = if gating_finding_count > 0 {
                                    StageStatus::Failed
                                } else if execute_stage_ok(
                                    &plateau_process,
                                    opts.execute_gate,
                                    opts.inferred_oracle_gate,
                                    &plateau_findings,
                                    &[],
                                    false,
                                ) {
                                    StageStatus::Passed
                                } else {
                                    StageStatus::Inconclusive
                                };
                                let finding_count = plateau_findings.len();
                                generated_failures.extend(plateau_findings);
                                stages.push(VerificationStage {
                                    name: "llm_plateau_escape".into(),
                                    status,
                                    duration_ms: proposal.duration_ms + plateau_process.duration_ms,
                                    detail: Some(serde_json::json!({
                                        "proposed": proposal.proposed_count,
                                        "accepted": accepted_count,
                                        "invalid": proposal.invalid_count,
                                        "finding_count": finding_count,
                                        "gating_finding_count": gating_finding_count,
                                        "unknown_finding_count": unknown_finding_count,
                                        "corpus_retained": corpus_retained_count,
                                        "stderr": proposal.stderr,
                                        "execution": plateau_process,
                                    })),
                                    message: (!execution_ok && finding_count == 0).then(|| {
                                        "LLM-proposed seeds could not be executed authoritatively"
                                            .into()
                                    }),
                                });
                            }
                        }
                    }
                }
            }

            let mut native_findings = Vec::new();
            if native_config.engine != NativeFuzzEngine::Off {
                let selected_refs = functions_to_fuzz.iter().collect::<Vec<_>>();
                if let Some(native_plan) = synthesize::synthesize_native_fuzz(
                    language,
                    &selected_refs,
                    native_config.engine,
                ) {
                    if opts.runtime_profile == RuntimeProfile::Isolated {
                        stages.push(VerificationStage {
                            name: "native_fuzz".into(),
                            status: StageStatus::Inconclusive,
                            duration_ms: 0,
                            detail: Some(serde_json::json!({
                                "engine": native_plan.engine,
                                "runs": native_config.runs,
                                "target_count": native_plan.target_count,
                                "reason": "native_engine_requires_local_trusted_profile",
                            })),
                            message: Some(
                                "Optional native fuzz engines require the local-trusted runtime profile"
                                    .into(),
                            ),
                        });
                    } else {
                        let mut native_code = match language {
                            Language::TypeScript => generated_typescript_target_import(
                                &verification_context.candidate,
                                &functions_to_fuzz,
                                &analysis.classes,
                                &fuzz_plan.coverage,
                            )
                            .unwrap_or_else(|| generated_target_source(code, language)),
                            Language::Python => generated_target_source(code, language),
                        };
                        native_code.push('\n');
                        native_code.push_str(&native_plan.code);
                        let native_start = Instant::now();
                        let native_execution = execute_native_fuzz_harness(
                            &verification_context.candidate,
                            native_code,
                            &native_plan,
                            native_config.runs,
                            &opts,
                            language,
                            execute_timeout.max(30.0),
                            Some(candidate_project_dir_owned.as_str()),
                            candidate_source_file_owned.as_deref(),
                        )
                        .await;
                        let native_ms = native_start.elapsed().as_millis() as u64;
                        let native_output = format!(
                            "{}\n{}",
                            native_execution.process.stdout, native_execution.process.stderr
                        );
                        native_findings = parse_native_findings_with_plan(
                            &native_output,
                            language,
                            Some(&verification_plan),
                        );
                        // Stage reduction and aggregate reduction must use the
                        // same suppression contract. Keep the observations for
                        // reporting, but never count suppressed ones as blockers.
                        let (active_native, suppressed_native) = split_findings(
                            native_findings,
                            &suppressions,
                            candidate_source_file_owned.as_deref(),
                        );
                        let native_suppressed_count = suppressed_native.len();
                        native_findings =
                            active_native.into_iter().chain(suppressed_native).collect();
                        let native_gating_count = native_findings
                            .iter()
                            .filter(|finding| !finding.suppressed)
                            .filter(|finding| {
                                finding_fails_execute_gate(
                                    opts.execute_gate,
                                    finding,
                                    opts.inferred_oracle_gate,
                                )
                            })
                            .count();
                        let native_unknown_count = native_findings
                            .iter()
                            .filter(|finding| {
                                !finding.suppressed
                                    && finding.input_classification == InputClassification::Unknown
                            })
                            .count();
                        let unavailable = native_engine_unavailable(&native_output);
                        let succeeded = native_execution.process.exit_code == Some(0)
                            && !native_execution.process.timed_out
                            && !native_execution.process.memory_error;
                        let (status, message) = if unavailable
                            && native_config.engine == NativeFuzzEngine::Auto
                        {
                            (
                                StageStatus::Skipped,
                                Some("No compatible native fuzz engine is installed".into()),
                            )
                        } else if unavailable {
                            (
                                StageStatus::Inconclusive,
                                Some(format!(
                                    "Requested native fuzz engine {:?} is unavailable",
                                    native_plan.engine
                                )),
                            )
                        } else if native_gating_count > 0 {
                            (StageStatus::Failed, Some(format!("Native engine observed {native_gating_count} admitted-input failure(s)")))
                        } else if native_unknown_count > 0 {
                            (
                                StageStatus::Inconclusive,
                                Some(format!(
                                    "{} observed {} exception input(s) without admission evidence",
                                    match native_plan.engine {
                                        NativeFuzzEngine::Atheris => "Atheris",
                                        NativeFuzzEngine::Jazzer => "Jazzer.js",
                                        NativeFuzzEngine::Off | NativeFuzzEngine::Auto => {
                                            "Native engine"
                                        }
                                    },
                                    native_unknown_count
                                )),
                            )
                        } else if native_suppressed_count > 0
                            && native_suppressed_count == native_findings.len()
                            && !native_execution.process.timed_out
                            && !native_execution.process.memory_error
                            && native_execution.process.exit_code.is_some()
                        {
                            (
                                StageStatus::Advisory,
                                Some(
                                    "Native observations retained under execute suppressions"
                                        .into(),
                                ),
                            )
                        } else if succeeded {
                            (StageStatus::Passed, None)
                        } else {
                            (
                                StageStatus::Inconclusive,
                                Some(
                                    "Native fuzz engine exited without a replayable finding".into(),
                                ),
                            )
                        };
                        stages.push(VerificationStage {
                            name: "native_fuzz".into(),
                            status,
                            duration_ms: native_ms,
                            detail: Some(serde_json::json!({
                                "engine": native_plan.engine,
                                "runs": native_config.runs,
                                "target_count": native_plan.target_count,
                                "execution": native_execution.process,
                                "native_findings": &native_findings,
                                "unknown_finding_count": native_unknown_count,
                                "suppressed_finding_count": native_suppressed_count,
                                "gating_finding_count": native_gating_count,
                                "diagnostics": native_execution.diagnostics,
                            })),
                            message,
                        });
                    }
                } else {
                    stages.push(VerificationStage {
                        name: "native_fuzz".into(),
                        status: StageStatus::Skipped,
                        duration_ms: 0,
                        detail: Some(serde_json::json!({
                            "engine": native_config.engine,
                            "runs": native_config.runs,
                            "reason": "no_supported_native_targets",
                        })),
                        message: Some(
                            "No selected function has a native-engine-compatible signature".into(),
                        ),
                    });
                }
            }

            let mut failures = generated_failures;
            failures.extend(differential_findings);
            failures.extend(native_findings);
            for finding in &mut failures {
                finding.launch_context = Some(launch_context.clone());
            }
            classify_type_signature_findings(&mut failures, &observed_calls, language);
            for finding in &mut failures {
                if finding.location.source_file.is_empty() {
                    finding.location.source_file = candidate_source_file_owned
                        .as_deref()
                        .unwrap_or("<inline>")
                        .to_string();
                }
                if finding.location.line == 0 {
                    if let Some(function) = analysis
                        .functions
                        .iter()
                        .find(|function| function.name == finding.location.function)
                    {
                        finding.location.line = function.line;
                    }
                }
            }
            let mut failures = coalesce_equivalent_findings(failures);
            let mut finding_ordinals: HashMap<String, usize> = HashMap::new();
            for finding in &mut failures {
                let symbol: String = finding
                    .location
                    .function
                    .chars()
                    .map(|character| {
                        if character.is_ascii_alphanumeric() {
                            character
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let ordinal = finding_ordinals.entry(symbol.clone()).or_default();
                *ordinal += 1;
                finding.id = format!("execute:{symbol}:{}", *ordinal);
            }
            let (failures, suppressed_failures) = split_findings(
                failures,
                &suppressions,
                candidate_source_file_owned.as_deref(),
            );
            let summary =
                findings_summary(&failures, &suppressed_failures, opts.inferred_oracle_gate);
            let unclassified_exceptions = failures
                .iter()
                .filter(|finding| finding.input_classification == InputClassification::Unknown)
                .map(|finding| finding.occurrences.max(1))
                .sum::<usize>();
            if unclassified_exceptions > 0 {
                harness_diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::VerifierHarness,
                    kind: FailureKind::AmbiguousGeneratedInput,
                    component: DiagnosticComponent::FuzzHarness,
                    impact: DiagnosticImpact::Blocking,
                    message: "exception observed without evidence establishing whether the input should be accepted or rejected; add a contract or authoritative test".into(),
                    process: None, limits: None,
                });
                if let Some(finding) = failures.iter().find(|finding| {
                    finding_fails_execute_gate(
                        opts.execute_gate,
                        finding,
                        opts.inferred_oracle_gate,
                    )
                }) {
                    harness_diagnostics.push(FailureDiagnostic {
                        domain: FailureDomain::TargetCode,
                        kind: if finding.category == FindingCategory::Property {
                            FailureKind::AssertionFailure
                        } else {
                            FailureKind::TargetException
                        },
                        component: DiagnosticComponent::Target,
                        impact: DiagnosticImpact::Gating,
                        message: finding.message.clone(),
                        process: None,
                        limits: None,
                    });
                }
            }
            let fuzz_outcomes = parse_fuzz_outcomes(&exec_result.stdout);
            let functions_with_valid_invocations = fuzz_outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome.status,
                        FuzzOutcomeStatus::Passed | FuzzOutcomeStatus::Crashed
                    ) && !nuxt_runtime_blocker
                        .as_ref()
                        .is_some_and(|blocker| blocker.blocks_surface(&outcome.function))
                })
                .count();
            let invocation_events = sandbox::parse_harness_events(&exec_result.stdout).ok();
            let surface_evidence = invocation_events
                .as_ref()
                .into_iter()
                .flat_map(|events| events.surfaces.iter())
                .filter(|(surface, _)| {
                    !nuxt_runtime_blocker.as_ref().is_some_and(|blocker| {
                        blocker.blocks_surface(
                            surface
                                .rsplit_once(':')
                                .map_or(surface.as_str(), |(name, _)| name),
                        )
                    })
                })
                .map(|(_, evidence)| evidence)
                .collect::<Vec<_>>();
            let valid_invocations = surface_evidence
                .iter()
                .map(|evidence| evidence.valid_completed)
                .sum::<usize>();
            let evaluated_oracles = surface_evidence
                .iter()
                .map(|evidence| evidence.passed_oracles + evidence.failed_oracles)
                .sum::<usize>();
            let no_inputs_reached = fuzz_outcomes
                .iter()
                .filter(|outcome| outcome.status == FuzzOutcomeStatus::NoInputsReached)
                .count();
            let exec_ok = execute_stage_ok(
                &exec_result,
                opts.execute_gate,
                opts.inferred_oracle_gate,
                &failures,
                &suppressed_failures,
                module_load_blocked,
            ) && no_inputs_reached == 0;
            let harness_event_detail = {
                let combined = format!("{}\n{}", exec_result.stdout, exec_result.stderr);
                if combined.contains(sandbox::HARNESS_EVENT_SENTINEL) {
                    match sandbox::parse_harness_events(&combined) {
                        Ok(summary) => serde_json::json!({
                            "records": summary.records.len(),
                            "completed_units": summary.completed_units,
                            "runner_started": summary.runner_started,
                            "target_resolved": summary.target_resolved,
                            "target_ready": summary.target_ready,
                            "harness_completed": summary.harness_completed,
                            "surfaces": summary.surfaces,
                        }),
                        Err(error) => {
                            let diagnostic = FailureDiagnostic {
                                domain: FailureDomain::VerifierHarness,
                                kind: FailureKind::HarnessProtocol,
                                component: DiagnosticComponent::FuzzHarness,
                                impact: DiagnosticImpact::Blocking,
                                message: error,
                                process: exec_result.termination.clone(),
                                limits: Some(ExecutionLimits {
                                    timeout_seconds: execute_timeout,
                                    memory_mb: opts.memory_mb,
                                    runtime_profile: opts.runtime_profile,
                                    network_policy: opts.network,
                                }),
                            };
                            serde_json::json!({
                                "diagnostics": [diagnostic],
                            })
                        }
                    }
                } else {
                    serde_json::Value::Null
                }
            };
            let portability_stage = if matches!(language, Language::TypeScript)
                && candidate_source_file_owned.is_some()
                && verification_context.candidate.target_source.mode == SourceMode::TypeScript
            {
                let portability_context = &verification_context.candidate;
                let relative_path = portability_context
                    .target_source
                    .source_file
                    .as_deref()
                    .and_then(|source| {
                        Path::new(source)
                            .strip_prefix(&portability_context.workspace_root)
                            .ok()
                            .map(Path::to_path_buf)
                    });
                let node_result = if let Some(relative_path) = relative_path {
                    sandbox::execute_harness(
                        portability_context,
                        HarnessSpec {
                            kind: HarnessKind::PortabilityProbe,
                            runtime: HarnessRuntime::NodeScript,
                            test_adapter: None,
                            source_mode: SourceMode::TypeScript,
                            artifact: HarnessArtifact::Existing { relative_path },
                            args: Vec::new(),
                            network: opts.network,
                        },
                        sandbox_options(
                            &opts,
                            language,
                            execute_timeout,
                            opts.memory_mb,
                            Some(candidate_project_dir_owned.as_str()),
                            candidate_source_file_owned.as_deref(),
                        ),
                    )
                    .await
                    .process
                } else {
                    ExecutionResult {
                        stdout: String::new(),
                        stderr: "TypeScript portability source is outside the workspace".into(),
                        exit_code: None,
                        duration_ms: 0,
                        timed_out: false,
                        memory_error: false,
                        termination: Some(ProcessTermination {
                            kind: ProcessTerminationKind::LaunchFailed,
                            exit_code: None,
                            signal: None,
                            signal_name: None,
                        }),
                        diagnostics: vec![],
                    }
                };
                if is_typescript_portability_error(&node_result.stderr) {
                    let repo_runtime = opts
                        .project_dir
                        .map(Path::new)
                        .filter(|project| {
                            project.join("bun.lock").is_file()
                                || project.join("bun.lockb").is_file()
                        })
                        .map(|_| "bun")
                        .unwrap_or("node");
                    let (detail, suppressed) = build_portability_detail(
                        repo_runtime,
                        &node_result,
                        &exec_result,
                        &suppressions,
                        candidate_source_file_owned.as_deref(),
                        opts.suppression_source,
                    );
                    Some(VerificationStage {
                        name: "portability".into(),
                        status: if suppressed {
                            StageStatus::Passed
                        } else {
                            StageStatus::Advisory
                        },
                        duration_ms: 0,
                        detail: Some(detail),
                        message: Some(
                            "Node portability check reported a repo-runtime compatibility warning"
                                .into(),
                        ),
                    })
                } else {
                    None
                }
            } else {
                None
            };
            if portability_stage.as_ref().is_some_and(|stage| {
                stage
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.get("repo_runtime"))
                    .and_then(|value| value.as_str())
                    == Some("bun")
            }) {
                exec_runtime = Some("bun".into());
            }
            if let Some(stage) = portability_stage {
                stages.push(stage);
            }
            let environment_setup = nuxt_runtime_blocker.as_ref().map(|blocker| {
                serde_json::json!({
                    "framework": "nuxt",
                    "classification": "missing_nuxt_auto_import_runtime",
                    "missing_globals": blocker.missing_globals,
                    "affected_findings": blocker.affected_findings,
                })
            });
            let mut detail = serde_json::json!({
                "execution": exec_result,
                "generated_campaign_ran": !fuzz_plan.code.is_empty(),
                "runtime": exec_runtime,
                "module_load_blocked": module_load_blocked,
                "environment_setup": environment_setup,
                "valid_invocations": valid_invocations,
                "functions_with_valid_invocations": functions_with_valid_invocations,
                "evaluated_oracles": evaluated_oracles,
                "unclassified_exceptions": unclassified_exceptions,
                "no_inputs_reached": no_inputs_reached,
                "findings": failures,
                "suppressed_findings": suppressed_failures,
                "findings_summary": summary,
                "verification_plan": &verification_plan,
                "seed_input_count": seed_input_count,
                "seeded_functions": seeded_function_count,
                "seed_sources": &seed_sources,
                "inferred_fixture_properties": &inferred_fixture_properties,
                "inferred_context_properties": &inferred_context_properties,
                "differential": differential_detail,
                "harness_events": harness_event_detail,
                "diagnostics": harness_diagnostics,
            });
            if let Some(diagnostics) = detail
                .get("harness_events")
                .and_then(|value| value.get("diagnostics"))
                .cloned()
            {
                detail["diagnostics"] = diagnostics;
            }
            stages.push(VerificationStage {
                name: "execute".into(),
                status: if exec_ok {
                    StageStatus::Passed
                } else if execute_gate_failed(
                    opts.execute_gate,
                    &failures,
                    opts.inferred_oracle_gate,
                ) {
                    StageStatus::Failed
                } else {
                    StageStatus::Inconclusive
                },
                duration_ms: exec_ms,
                detail: Some(detail),
                message: if exec_ok {
                    None
                } else if let Some(blocker) = &nuxt_runtime_blocker {
                    Some(blocker.diagnostic().message)
                } else {
                    Some(exec_result.stderr.clone())
                },
            });
        } else {
            let mut coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
            apply_context_dependent_coverage(&mut coverage, &context_dependent_surfaces);
            stages.push(VerificationStage {
                name: "coverage".into(),
                status: StageStatus::Passed,
                duration_ms: coverage_ms,
                detail: Some(serde_json::json!({
                    "functions": serde_json::to_value(&coverage).unwrap(),
                    "counts": coverage_counts(&coverage),
                    "diff_scoped": opts.diff.is_some(),
                    "seed_input_count": seed_input_count,
                    "seeded_functions": seeded_function_count,
                    "seed_sources": seed_sources,
                    "inferred_fixture_properties": inferred_fixture_properties,
                    "inferred_context_properties": inferred_context_properties,
                    "auto_seed": opts.auto_seed,
                    "verification_plan": &verification_plan,
                })),
                message: None,
            });
            stages.push(skipped_execute_stage(
                "no_fuzzable_targets",
                "Execution skipped because no fuzzable targets produced runnable cases",
            ));
        }
    } else if !opts.tests_only {
        stages.push(skipped_execute_stage(
            "no_analyzed_functions",
            "Execution skipped because no functions were available to fuzz",
        ));
    }

    // Stage 5: Test (if test_code provided) — this IS authoritative
    if let Some(tests) = effective_test_code {
        let required_candidates =
            test_quality_functions(&analysis, candidate_source_file_owned.as_deref(), opts.diff);
        let required_functions = required_candidates.iter().collect::<Vec<_>>();
        let baseline_test = run_authoritative_test(
            code,
            tests,
            &required_functions,
            language,
            &verification_context,
            &opts,
            candidate_source_file_owned.as_deref(),
            candidate_test_source_file_owned.as_deref(),
            test_timeout(),
        )
        .await;
        let entered_surfaces = &baseline_test.entered_surfaces;
        let test_ok = baseline_test.test_ok;
        let covered_required = baseline_test.covered_required;

        stages.push(authoritative_test_stage(&baseline_test, &opts));

        if !opts.tests_only
            && baseline_test.overlay.supported
            && (test_ok || !entered_surfaces.is_empty())
        {
            let authoritative_source = candidate_test_source_file_owned
                .as_deref()
                .unwrap_or("<inline>")
                .to_string();
            if let Some(coverage_stage) = stages.iter_mut().find(|stage| stage.name == "coverage") {
                if let Some(detail) = coverage_stage.detail.as_mut() {
                    if let Some(functions_value) = detail.get("functions").cloned() {
                        if let Ok(mut coverage_functions) =
                            serde_json::from_value::<Vec<FuzzFunctionCoverage>>(functions_value)
                        {
                            for function in &mut coverage_functions {
                                let surface_id = format!("{}:{}", function.function, function.line);
                                if !function.required || !entered_surfaces.contains(&surface_id) {
                                    continue;
                                }
                                if test_ok {
                                    function.status =
                                        FuzzFunctionStatus::CheckedViaAuthoritativeTest;
                                    function.reason = None;
                                } else if !matches!(
                                    function.status,
                                    FuzzFunctionStatus::CheckedDirect
                                        | FuzzFunctionStatus::CheckedViaFactory
                                        | FuzzFunctionStatus::CheckedViaCaller
                                        | FuzzFunctionStatus::CheckedViaAuthoritativeTest
                                ) {
                                    function.status =
                                        FuzzFunctionStatus::ReachedViaAuthoritativeTest;
                                    function.reason = Some(
                                        "authoritative test reached the exact surface before test completion"
                                            .into(),
                                    );
                                } else {
                                    continue;
                                }
                                function.invocation_path = InvocationPath::AuthoritativeTest {
                                    source_file: authoritative_source.clone(),
                                };
                            }
                            let all_required_checked = coverage_functions
                                .iter()
                                .filter(|function| function.required)
                                .all(|function| {
                                    matches!(
                                        function.status,
                                        FuzzFunctionStatus::CheckedDirect
                                            | FuzzFunctionStatus::CheckedViaAuthoritativeTest
                                    )
                                });
                            detail["counts"] =
                                serde_json::to_value(coverage_counts(&coverage_functions)).unwrap();
                            detail["functions"] =
                                serde_json::to_value(&coverage_functions).unwrap();
                            detail["authoritative_test_source"] =
                                serde_json::Value::String(authoritative_source);
                            if all_required_checked {
                                coverage_stage.status = StageStatus::Passed;
                                coverage_stage.message = None;
                            }
                        }
                    }
                }
            }
        }

        if opts.tests_only {
            let authoritative_source = candidate_test_source_file_owned
                .as_deref()
                .unwrap_or("<inline>")
                .to_string();
            let coverage_functions = required_functions
                .iter()
                .map(|function| {
                    let surface_id = format!("{}:{}", function.name, function.line);
                    let reached =
                        baseline_test.overlay.supported && entered_surfaces.contains(&surface_id);
                    let checked = reached && test_ok;
                    let status = if checked {
                        FuzzFunctionStatus::CheckedViaAuthoritativeTest
                    } else if reached {
                        FuzzFunctionStatus::ReachedViaAuthoritativeTest
                    } else {
                        FuzzFunctionStatus::SkippedNoFuzzableSurface
                    };
                    let reason = if checked {
                        None
                    } else if reached {
                        Some(
                            "authoritative test reached the exact surface before test completion"
                                .into(),
                        )
                    } else if !baseline_test.overlay.supported {
                        Some(baseline_test.overlay.reason.clone().unwrap_or_else(|| {
                            "authoritative-test instrumentation is unsupported".into()
                        }))
                    } else if !test_ok {
                        Some("authoritative test did not complete successfully".into())
                    } else {
                        Some(
                            "authoritative test did not emit the exact target_entered surface id"
                                .into(),
                        )
                    };
                    FuzzFunctionCoverage {
                        function: function.name.clone(),
                        line: function.line,
                        end_line: function.end_line,
                        status,
                        required: true,
                        invocation_path: InvocationPath::AuthoritativeTest {
                            source_file: authoritative_source.clone(),
                        },
                        is_exported: true,
                        reason,
                    }
                })
                .collect::<Vec<_>>();
            let all_required_checked = !coverage_functions.is_empty()
                && coverage_functions.iter().all(|function| {
                    function.status == FuzzFunctionStatus::CheckedViaAuthoritativeTest
                });
            let coverage_message = if !baseline_test.overlay.supported {
                baseline_test.overlay.reason.clone()
            } else if coverage_functions.is_empty() {
                Some("tests-only mode selected no required exported surfaces".into())
            } else if !all_required_checked {
                Some(format!("authoritative test reached {covered_required} of {} required exported surfaces", coverage_functions.len()))
            } else {
                None
            };
            stages.push(VerificationStage {
                name: "coverage".into(),
                status: if all_required_checked {
                    StageStatus::Passed
                } else {
                    StageStatus::Inconclusive
                },
                duration_ms: 0,
                detail: Some(serde_json::json!({
                    "functions": coverage_functions,
                    "counts": coverage_counts(&coverage_functions),
                    "tests_only": true,
                    "required_surface_count": required_functions.len(),
                    "observed_required_surface_count": covered_required,
                })),
                message: coverage_message,
            });
        }
        if opts.test_code.is_some() {
            if let Some(max_mutants) = opts.test_quality_max_mutants {
                stages.push(
                    run_test_quality_stage(
                        code,
                        tests,
                        &required_functions,
                        language,
                        &verification_context,
                        &opts,
                        candidate_source_file_owned.as_deref(),
                        candidate_test_source_file_owned.as_deref(),
                        &baseline_test,
                        max_mutants,
                    )
                    .await,
                );
            }
        }
    }

    ensure_test_quality_stage(
        &mut stages,
        opts.test_quality_max_mutants,
        "Mutation campaign skipped because no explicit authoritative test entrypoint was available",
    );
    finalize_report(
        build_report(stages, opts.coverage_gate, code, opts.source_file),
        opts.output_dir,
        opts.source_file,
        language,
        opts.report_level,
    )
}

fn finalize_report(
    mut report: VerificationReport,
    output_dir: Option<&str>,
    source_file: Option<&str>,
    language: &Language,
    report_level: ReportLevel,
) -> VerificationReport {
    if let Some(dir) = output_dir {
        report.report_path = write_report(dir, &report, source_file, language, report_level);
    }
    report
}

const NATIVE_FINDING_MARKER: &str = "__COURT_JESTER_NATIVE_FINDING__";

#[derive(Debug, Deserialize)]
struct NativeFindingRecord {
    #[serde(default)]
    protocol_version: Option<u32>,
    #[serde(default)]
    argument_snapshots: Option<Vec<ReproValue>>,
    #[serde(default)]
    replay_snippet: Option<String>,
    function: String,
    line: usize,
    #[serde(default)]
    arguments: Vec<serde_json::Value>,
    input: String,
    #[serde(default)]
    error_type: Option<String>,
    message: String,
}

#[cfg(test)]
fn parse_native_findings(output: &str, language: &Language) -> Vec<VerificationFinding> {
    parse_native_findings_with_plan(output, language, None)
}

fn native_input_classification(
    function: &str,
    line: usize,
    arguments: &[ReproValue],
    plan: &VerificationPlan,
) -> InputClassification {
    let surfaces = plan
        .surfaces
        .iter()
        .filter(|surface| surface.symbol == function && surface.line == line)
        .collect::<Vec<_>>();
    let [surface] = surfaces.as_slice() else {
        return InputClassification::Unknown;
    };
    let domains = plan
        .parameter_domains
        .iter()
        .filter(|domain| domain.surface_id == surface.id)
        .cloned()
        .collect::<Vec<_>>();
    if domains.is_empty()
        || domains
            .iter()
            .any(|domain| !domain.closed || domain.variadic.is_some())
    {
        return InputClassification::Unknown;
    }
    let Some(slots) = arguments
        .iter()
        .map(|value| {
            Some(PlannedArgumentSlot::Single(DomainLiteral {
                expression: value.expression.clone(),
                json_value: Some(value.json_value.clone()?),
            }))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return InputClassification::Unknown;
    };
    let Ok(arguments) = domain::bind_argument_slots(&domains, PlannedArgumentSlots { slots })
    else {
        return InputClassification::Unknown;
    };
    domain::classify_input(&arguments, &domains)
}

fn parse_native_findings_with_plan(
    output: &str,
    language: &Language,
    plan: Option<&VerificationPlan>,
) -> Vec<VerificationFinding> {
    output
        .lines()
        .filter_map(|line| line.find(NATIVE_FINDING_MARKER).map(|index| &line[index..]))
        .filter_map(|line| {
            serde_json::from_str::<NativeFindingRecord>(&line[NATIVE_FINDING_MARKER.len()..]).ok()
        })
        .filter_map(|record| {
            let arguments = match record.protocol_version {
                Some(2) => {
                    let snapshots = record.argument_snapshots?;
                    if snapshots.iter().any(|value| value.expression.trim().is_empty()) {
                        return None;
                    }
                    snapshots
                }
                None | Some(1) => record
                .arguments
                .iter()
                .map(|value| ReproValue {
                    expression: serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
                    json_value: Some(value.clone()),
                })
                .collect::<Vec<_>>(),
                Some(_) => return None,
            };
            let original = ReproCase {
                arguments: arguments.clone(),
                input_text: Some(record.input.clone()),
            };
            let input_classification = if record.protocol_version == Some(2) {
                plan.map(|plan| native_input_classification(&record.function, record.line, &arguments, plan)).unwrap_or(InputClassification::Unknown)
            } else { InputClassification::Unknown };
            let confidence = if input_classification == InputClassification::Valid { FindingConfidence::High } else { FindingConfidence::Low };
            let expectation = ReplayExpectation {
                severity: FindingSeverity::Crash,
                oracle_kind: OracleKind::RuntimeContract,
                category: FindingCategory::Exception,
            };
            Some(VerificationFinding {
                id: format!("native:{}", record.function),
                severity: FindingSeverity::Crash,
                confidence,
                category: FindingCategory::Exception,
                occurrences: 1,
                sample_inputs: vec![original.clone()],
                location: FindingLocation {
                    source_file: String::new(),
                    function: record.function.clone(),
                    line: record.line,
                    invocation_path: InvocationPath::Direct,
                },
                oracle: OracleInfo {
                    id: format!("native_runtime:{}", record.function),
                    kind: OracleKind::RuntimeContract,
                    provenance: OracleProvenance::ObservedCall,
                    confidence,
                    expected: None,
                    actual: Some(record.message.clone()),
                },
                input_classification,
                repro: StructuredRepro {
                    kind: ReproKind::FunctionCall,
                    function: Some(record.function.clone()),
                    arguments,
                    input_text: Some(record.input),
                    case_label: Some("native_coverage_guided".into()),
                    snippet: record.replay_snippet.filter(|snippet| record.protocol_version == Some(2) && !snippet.trim().is_empty()).unwrap_or_else(|| match language {
                        Language::Python => "raise RuntimeError('Court Jester native observation has no recorded replay contract')".into(),
                        Language::TypeScript => "throw new Error('Court Jester native observation has no recorded replay contract');".into(),
                    }),
                    command: None,
                    expectation,
                    differential: None,
                },
                minimization: MinimizationInfo {
                    status: MinimizationStatus::NotNeeded,
                    attempts: 0,
                    original: original.clone(),
                    minimized: None,
                },
                launch_context: None,
                error_type: record.error_type,
                message: record.message,
                classification: Some("native_coverage_guided".into()),
                suggestion: None,
                suppressed: false,
            })
        })
        .collect()
}

fn native_engine_unavailable(output: &str) -> bool {
    [
        "No module named 'atheris'",
        "No module named atheris",
        "required runtime 'jazzer' is unavailable",
        "Cannot find package '@jazzer.js",
        "Cannot find module '@jazzer.js",
    ]
    .iter()
    .any(|message| output.contains(message))
}

/// independently framed result.
pub fn parse_findings(stdout: &str) -> Option<Vec<VerificationFinding>> {
    if stdout.contains(sandbox::HARNESS_EVENT_SENTINEL) {
        if let Ok(summary) = sandbox::parse_harness_events(stdout) {
            if !summary.findings.is_empty() {
                return Some(summary.findings);
            }
        }
    }

    let marker = "__COURT_JESTER_FINDINGS_JSON__";
    let idx = stdout.rfind(marker)?;
    let json_str = stdout[idx + marker.len()..].trim();
    serde_json::from_str(json_str).ok()
}

fn findings_from_stages(stages: &[VerificationStage]) -> Vec<VerificationFinding> {
    let mut findings = Vec::new();
    for stage in stages {
        // Differential findings are also attached to execute so the existing
        // report summary/gating path sees them. Do not surface the dedicated
        // diagnostic-stage copy as a second replay finding.
        if stage.name == "differential" {
            continue;
        }
        let Some(detail) = stage.detail.as_ref() else {
            continue;
        };
        for key in ["findings", "suppressed_findings"] {
            let Some(items) = detail.get(key).and_then(|value| value.as_array()) else {
                continue;
            };
            for item in items {
                if let Ok(finding) = serde_json::from_value::<VerificationFinding>(item.clone()) {
                    findings.push(finding);
                }
            }
        }
    }
    findings
}

fn repair_priority(finding: &VerificationFinding) -> u8 {
    if finding.suppressed {
        return u8::MAX;
    }
    match (
        finding.severity,
        finding.confidence,
        finding.oracle.kind,
        finding.input_classification,
    ) {
        (_, FindingConfidence::Authoritative, _, _) | (_, _, OracleKind::AuthoritativeTest, _) => 0,
        (FindingSeverity::Crash, FindingConfidence::High, _, InputClassification::Valid) => 1,
        (_, _, OracleKind::DeclaredProperty | OracleKind::TypeContract, _) => 2,
        (_, _, OracleKind::SeedRegression | OracleKind::Differential, _) => 3,
        (_, _, OracleKind::GenericProperty, _) => 4,
        (_, FindingConfidence::Low, OracleKind::InferredSemantic, _) => 5,
        (FindingSeverity::Infrastructure, _, _, _) => 6,
        _ => 7,
    }
}

/// Build the stable, agent-facing repair view from a verification report.
pub fn repair_summary(report: &VerificationReport, language: &Language) -> RepairSummary {
    let findings = findings_from_stages(&report.stages);
    let primary_finding = findings
        .iter()
        .filter(|finding| !finding.suppressed)
        .min_by_key(|finding| repair_priority(finding))
        .cloned();
    let recommended_action = match report.verdict {
        VerificationVerdict::Pass => "none",
        VerificationVerdict::Fail => {
            if report.diagnostics.iter().any(|diagnostic| {
                diagnostic.domain == FailureDomain::TargetCode
                    && diagnostic.impact == DiagnosticImpact::Gating
            }) {
                "repair"
            } else {
                "inspect_environment"
            }
        }
        VerificationVerdict::Inconclusive => {
            let non_target_blocker = report.diagnostics.iter().any(|diagnostic| {
                diagnostic.impact == DiagnosticImpact::Blocking
                    && diagnostic.domain != FailureDomain::TargetCode
                    && !matches!(
                        diagnostic.kind,
                        FailureKind::ContractViolation
                            | FailureKind::AmbiguousGeneratedInput
                            | FailureKind::InvalidGeneratedInput
                    )
            });
            let infrastructure = non_target_blocker
                || findings.iter().any(|finding| {
                    !finding.suppressed && finding.severity == FindingSeverity::Infrastructure
                });
            if infrastructure {
                "inspect_environment"
            } else if report.summary.coverage.required
                > report.summary.coverage.behaviorally_checked
                || report.summary.coverage.no_inputs_reached > 0
            {
                "add_contract_or_test"
            } else {
                "inspect_environment"
            }
        }
    }
    .to_string();
    RepairSummary {
        schema_version: report.schema_version,
        tool: report.tool.clone(),
        candidate: report.candidate.clone(),
        meta: ReportMeta {
            source_file: report.candidate.source_file.clone(),
            language: match language {
                Language::Python => "python",
                Language::TypeScript => "typescript",
            }
            .into(),
            timestamp: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            duration_ms: report.stages.iter().map(|stage| stage.duration_ms).sum(),
        },
        verdict: report.verdict,
        strength: report.strength,
        summary: report.summary.clone(),
        recommended_action,
        primary_finding,
        findings,
        coverage: report.summary.coverage.clone(),
        diagnostics: report.diagnostics.clone(),
        diagnostics_summary: report.diagnostics_summary.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_event_transport_decoding_is_adapter_scoped() {
        let event = r#"{"event":"target_entered","surface_id":"eligible:1"}"#;
        let tap = format!("  # {event}");
        assert!(parse_target_entered_events(&tap).is_empty());
        for (language, runner, expected) in [
            (Language::TypeScript, TestRunner::Node, 1),
            (Language::TypeScript, TestRunner::Bun, 0),
            (Language::Python, TestRunner::Auto, 0),
        ] {
            let decoded = authoritative_output_line(&tap, &language, runner);
            assert_eq!(
                parse_target_entered_lines(std::iter::once(decoded)).len(),
                expected
            );
        }
        for line in [
            format!("ok 1 - {event}"),
            format!("log {event}"),
            "# {\"event\":\"target_entered\",\"surface_id\":42}".into(),
        ] {
            let decoded = authoritative_output_line(&line, &Language::TypeScript, TestRunner::Node);
            assert!(parse_target_entered_lines(std::iter::once(decoded)).is_empty());
        }
    }

    #[test]
    fn native_admission_requires_complete_matching_bound_values() {
        let analysis = analyze::analyze(
            "def inspect(*, value: bool):\n    return value\n",
            &Language::Python,
        );
        let plan = domain::build_verification_plan(
            &analysis.functions,
            &analysis.classes,
            &analysis.aliases,
            &Language::Python,
            &[],
            &[],
            &[],
        );
        let argument = ReproValue {
            expression: "False".into(),
            json_value: Some(serde_json::json!(false)),
        };
        assert_eq!(
            native_input_classification("inspect", 1, std::slice::from_ref(&argument), &plan),
            InputClassification::Valid
        );
        assert_eq!(
            native_input_classification("inspect", 2, std::slice::from_ref(&argument), &plan),
            InputClassification::Unknown
        );
        assert_eq!(
            native_input_classification("inspect", 1, &[], &plan),
            InputClassification::Unknown
        );
        assert_eq!(
            native_input_classification("inspect", 1, &[argument.clone(), argument.clone()], &plan),
            InputClassification::Unknown
        );
        let absent = ReproValue {
            expression: "False".into(),
            json_value: None,
        };
        assert_eq!(
            native_input_classification("inspect", 1, &[absent], &plan),
            InputClassification::Unknown
        );
        let invalid = ReproValue {
            expression: "'false'".into(),
            json_value: Some(serde_json::json!("false")),
        };
        assert_eq!(
            native_input_classification("inspect", 1, &[invalid], &plan),
            InputClassification::Invalid
        );
    }

    #[test]
    fn native_protocol_requires_versioned_snapshots_before_accepting_replay() {
        let legacy = serde_json::json!({
            "function": "inspect", "line": 1, "arguments": [3], "input": "00",
            "error_type": "ValueError", "message": "failure",
            "replay_snippet": "unversioned replay must not run"
        });
        let parse = |record: &serde_json::Value| {
            parse_native_findings(
                &format!("{NATIVE_FINDING_MARKER}{record}"),
                &Language::Python,
            )
        };
        let findings = parse(&legacy);
        assert_eq!(findings.len(), 1);
        assert!(findings[0]
            .repro
            .snippet
            .contains("no recorded replay contract"));
        assert_eq!(
            findings[0].input_classification,
            InputClassification::Unknown
        );

        let mut versioned = legacy.clone();
        versioned["protocol_version"] = 2.into();
        assert!(parse(&versioned).is_empty());
        versioned["argument_snapshots"] = serde_json::json!([{"expression": "bytearray(b'abc')"}]);
        versioned["replay_snippet"] = "recorded replay".into();
        let findings = parse(&versioned);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].repro.arguments[0].expression,
            "bytearray(b'abc')"
        );
        assert!(findings[0].repro.arguments[0].json_value.is_none());
        assert_eq!(findings[0].repro.snippet, "recorded replay");
        assert_eq!(
            findings[0].input_classification,
            InputClassification::Unknown
        );
        versioned["protocol_version"] = 99.into();
        assert!(parse(&versioned).is_empty());
        versioned["protocol_version"] = 2.into();
        versioned["argument_snapshots"][0]["expression"] = " ".into();
        assert!(parse(&versioned).is_empty());
        versioned["argument_snapshots"][0]["expression"] = 3.into();
        assert!(parse(&versioned).is_empty());
    }

    #[test]
    fn candidate_count_uses_runtime_surface_eligibility() {
        let code = "export function publicFn(value: number) { return value < 1; }\nfunction internalFn(value: number) { return value > 1; }\n";
        let count = test_quality_candidate_count(
            code,
            &Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/source.ts"),
            None,
        )
        .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn same_line_class_methods_are_instrumented_by_qualified_identity() {
        let code = "export class A { same(value: number) { return value < 0; } } export class B { same(value: number) { return value > 0; } }";
        let analysis = analyze::analyze(code, &Language::TypeScript);
        let functions = analysis
            .functions
            .iter()
            .filter(|function| matches!(function.name.as_str(), "A#same" | "B#same"))
            .collect::<Vec<_>>();
        assert_eq!(functions.len(), 2, "{:#?}", analysis.functions);
        let instrumented = instrument_source_for_surfaces(
            code,
            &functions,
            &Language::TypeScript,
            SourceMode::TypeScript,
        )
        .unwrap();
        assert_eq!(instrumented.matches("A#same:1").count(), 1);
        assert_eq!(instrumented.matches("B#same:1").count(), 1);
    }

    #[test]
    fn typescript_surface_events_bypass_captured_console_error() {
        let code = "function parseSort(value) { return value.trim(); }\n";
        let analysis = analyze::analyze(code, &Language::TypeScript);
        let functions = analysis.functions.iter().collect::<Vec<_>>();
        let instrumented = instrument_source_for_surfaces(
            code,
            &functions,
            &Language::TypeScript,
            SourceMode::TypeScript,
        )
        .unwrap();
        let script =
            format!("console.error = () => {{}};\n{instrumented}\nparseSort('name:asc');\n");
        let output = std::process::Command::new("node")
            .args(["--input-type=module", "--eval", &script])
            .output()
            .expect("Node must execute the instrumented target");

        assert!(output.status.success(), "{output:#?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            parse_target_entered_events(&stderr).contains("parseSort:1"),
            "surface events must remain observable when a test runner captures console.error: {stderr:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn authoritative_overlay_copies_regular_workspace_files() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n",
        )
        .unwrap();

        mirror_test_overlay(source.path(), destination.path(), Path::new(""), &[]).unwrap();

        let lockfile = destination.path().join("pnpm-lock.yaml");
        assert!(lockfile.is_file());
        assert!(
            !std::fs::symlink_metadata(lockfile)
                .unwrap()
                .file_type()
                .is_symlink(),
            "regular workspace files must remain regular in the generated overlay"
        );
    }
}
