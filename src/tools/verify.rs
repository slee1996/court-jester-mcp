use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

use crate::tools::{analyze, diff, domain, lint, sandbox, synthesize};
use crate::types::*;

pub struct VerifyOptions<'a> {
    pub test_code: Option<&'a str>,
    pub test_source_file: Option<&'a str>,
    pub test_runner: TestRunner,
    pub tests_only: bool,
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
    let mut entered = HashSet::new();
    for line in stderr.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
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

fn apply_runtime_coverage_proof(coverage: &mut [FuzzFunctionCoverage], stderr: &str) {
    let entered = parse_target_entered_events(stderr);
    for item in coverage {
        let proved = entered.iter().any(|surface| {
            surface == &item.function
                || surface.starts_with(&format!("{}:", item.function))
                || surface.contains(&format!("().{}", item.function))
        });
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

    fn matching_target<'a>(
        targets: &'a [InstrumentationTarget<'_>],
        line: usize,
        syntax_name: &str,
        qualified_name_allowed: bool,
    ) -> Option<&'a InstrumentationTarget<'a>> {
        targets.iter().find(|target| {
            target.line == line
                && (target.analyzer_name == syntax_name
                    || (qualified_name_allowed
                        && unqualified_name(target.analyzer_name) == syntax_name))
        })
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
                insertions.push((body.start_byte() + 1, format!("\nconsole.error(JSON.stringify({{event: 'target_entered', surface_id: '{}'}}));", target.surface_id)));
                instrumented.insert(target.surface_id.clone());
            }
            Language::TypeScript if callable.kind() == "arrow_function" => {
                insertions.push((body.start_byte(), format!("{{ console.error(JSON.stringify({{event: 'target_entered', surface_id: '{}'}})); return ", target.surface_id)));
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
                    matching_target(targets, line, name, false),
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
                if let (Some(target), Some(body)) = (
                    matching_target(targets, line, name, true),
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
                if let Some(target) = matching_target(targets, line, name, qualified_name_allowed) {
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
        symlink(entry.path(), destination_root.join(&relative)).map_err(|error| {
            format!(
                "failed to link instrumentation overlay '{}': {error}",
                relative.display()
            )
        })?;
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
            overlay,
        };
    };
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
        overlay,
    }
}

fn function_key(func: &FunctionInfo) -> (String, usize) {
    (func.name.clone(), func.line)
}

fn stable_digest(value: &str) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bytes = value.as_bytes();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let padded_len = (bytes.len() + 9).div_ceil(64) * 64;
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    padded.resize(padded_len - 8, 0);
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes(chunk[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
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
    let mut probe = String::from(code);
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

fn tree_digest(files: &[EmbeddedSource]) -> String {
    if files.len() == 1 {
        return stable_digest(&files[0].content);
    }
    let mut entries = files
        .iter()
        .map(|source| format!("{}\n{}", source.relative_path, source.content))
        .collect::<Vec<_>>();
    entries.sort();
    stable_digest(&entries.join("\n"))
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
        FuzzFunctionStatus::ReachedViaFactory,
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
        let entry = if !allowed.contains(&key) {
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
fn coverage_has_gap(coverage: &CoverageSummary, gate: CoverageGate) -> bool {
    gate == CoverageGate::ChangedExports && coverage.required > coverage.behaviorally_checked
}

/// Compute the schema-v3 verdict and evidence strength from typed stage/evidence data.
pub fn final_verdict(
    stages: &[VerificationStage],
    coverage: &CoverageSummary,
    gate: CoverageGate,
    evidence: &VerificationEvidence,
) -> (VerificationVerdict, VerificationStrength) {
    let parse_failed = stages
        .iter()
        .any(|stage| stage.name == "parse" && stage.status == StageStatus::Failed);
    let strength = if parse_failed {
        VerificationStrength::ParseOnly
    } else if evidence.authoritative_test_completed {
        VerificationStrength::AuthoritativeTests
    } else if evidence.evaluated_oracles > 0 {
        VerificationStrength::PropertyChecked
    } else if evidence.valid_invocations > 0 {
        VerificationStrength::RuntimeSmoke
    } else if evidence.static_checks_completed {
        VerificationStrength::StaticChecked
    } else if evidence.parsed {
        VerificationStrength::ParseOnly
    } else {
        VerificationStrength::None
    };
    // Typed causes outrank the lossy stage status: a gating target cause
    // remains a failure even when the process also reported a resource or
    // harness termination, while a blocking non-target cause is inconclusive.
    let diagnostics = diagnostics_from_stages(stages);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.impact == DiagnosticImpact::Gating)
    {
        return (VerificationVerdict::Fail, strength);
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.impact == DiagnosticImpact::Blocking)
    {
        return (VerificationVerdict::Inconclusive, strength);
    }
    if stages
        .iter()
        .any(|stage| stage.status == StageStatus::Failed)
    {
        return (VerificationVerdict::Fail, strength);
    }
    if coverage_has_gap(coverage, gate)
        || (!evidence.authoritative_test_completed
            && evidence.valid_invocations == 0
            && evidence.evaluated_oracles == 0)
        || stages
            .iter()
            .any(|stage| stage.status == StageStatus::Inconclusive)
    {
        return (VerificationVerdict::Inconclusive, strength);
    }
    (VerificationVerdict::Pass, strength)
}

fn is_typescript_portability_error(stderr: &str) -> bool {
    stderr.contains("ERR_MODULE_NOT_FOUND")
        || stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
        || stderr.contains("Cannot find module 'bun'")
        || stderr.contains("Cannot find package 'bun'")
        || stderr.contains("Bun is not defined")
        || stderr.contains("needs an import attribute of \"type: json\"")
}

fn is_typescript_module_load_error(stderr: &str) -> bool {
    is_typescript_portability_error(stderr)
        || stderr.contains("Cannot find module")
        || stderr.contains("Cannot find package")
        || stderr.contains("The requested module")
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SuppressionsFile {
    #[serde(default)]
    rules: Vec<SuppressionRule>,
}

#[derive(Debug, Clone, Deserialize)]
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

fn parse_suppressions(raw: Option<&str>) -> SuppressionsFile {
    raw.and_then(|value| serde_json::from_str::<SuppressionsFile>(value).ok())
        .unwrap_or_default()
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
        } else {
            FuzzOutcomeStatus::Passed
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
fn findings_summary(
    findings: &[VerificationFinding],
    suppressed: usize,
    inferred_gate: InferredOracleGate,
) -> FindingsSummary {
    let mut summary = FindingsSummary {
        total: findings.len() + suppressed,
        gating: 0,
        advisory: suppressed,
        suppressed,
        by_severity: BTreeMap::new(),
        by_oracle_kind: BTreeMap::new(),
    };
    for finding in findings {
        let advisory = finding.confidence == FindingConfidence::Low
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
    if !active_findings.is_empty()
        && active_findings.iter().all(|finding| {
            finding.message.starts_with("Comparator")
                && finding.minimization.status == MinimizationStatus::Failed
        })
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

fn authoritative_harness_runtime(
    language: Language,
    runner: TestRunner,
) -> (HarnessRuntime, Option<TestAdapter>) {
    match language {
        Language::Python => (HarnessRuntime::Python, None),
        Language::TypeScript => match runner {
            TestRunner::Bun => (HarnessRuntime::BunTest, Some(TestAdapter::BunJunit)),
            TestRunner::RepoNative => (HarnessRuntime::RepoTest, Some(TestAdapter::Opaque)),
            TestRunner::Node => (HarnessRuntime::NodeScript, Some(TestAdapter::NodeTap)),
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
    let mut dependency_roots = vec![project.clone()];
    for root in &context.dependency_roots {
        if let Ok(relative) = root.strip_prefix(original_workspace) {
            let mapped = project.join(relative);
            if !dependency_roots.iter().any(|existing| existing == &mapped) {
                dependency_roots.push(mapped);
            }
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

/// Run the full verification pipeline: parse → complexity → lint → synthesize+execute → test.
pub async fn verify(
    code: &str,
    language: &Language,
    opts: VerifyOptions<'_>,
) -> VerificationReport {
    let mut stages = vec![];
    let suppressions = parse_suppressions(opts.suppressions);
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
            return finalize_report(
                build_report(stages, opts.coverage_gate),
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
    let candidate_test_source_file_owned = verification_context
        .candidate
        .test_source
        .as_ref()
        .and_then(|source| source.source_file.as_ref())
        .map(|path| path.to_string_lossy().into_owned());
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
            status: StageStatus::Failed,
            duration_ms: parse_ms,
            detail: Some(serde_json::to_value(&analysis).unwrap()),
            message: Some(message),
        });
        return finalize_report(
            build_report(stages, opts.coverage_gate),
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

    if opts.tests_only && opts.test_code.is_none() {
        stages.push(VerificationStage {
            name: "test".into(),
            status: StageStatus::Inconclusive,
            duration_ms: 0,
            detail: None,
            message: Some("tests_only mode requires an authoritative test".into()),
        });
        return finalize_report(
            build_report(stages, opts.coverage_gate),
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
            opts.test_code,
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
        let verification_plan = domain::build_verification_plan(
            &functions_to_fuzz,
            &all_classes,
            &all_aliases,
            language,
            &caller_examples,
            &fixture_examples,
            &inferred_properties,
        );
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
        let mut module_load_blocked = false;
        if !fuzz_plan.code.is_empty() {
            let full_code = format!("{code}\n{}", fuzz_plan.code);
            let execute_timeout = execute_timeout_for(language);

            let start = Instant::now();
            let (_, runtime_name, _) =
                generated_harness_runtime(verification_context.candidate.target_source.mode);
            let mut exec_runtime = Some(runtime_name.to_string());
            let harness_execution = execute_generated_harness(
                &verification_context.candidate,
                full_code,
                HarnessKind::GeneratedVerifier,
                &opts,
                language,
                execute_timeout,
                Some(candidate_project_dir_owned.as_str()),
                candidate_source_file_owned.as_deref(),
            )
            .await;
            let harness_diagnostics = harness_execution.diagnostics;
            let exec_result = harness_execution.process;
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
            module_load_blocked = matches!(language, Language::TypeScript)
                && is_typescript_module_load_error(&exec_result.stderr)
                && !exec_result
                    .stdout
                    .lines()
                    .any(|line| line.starts_with("FUZZ "));
            let mut coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
            apply_runtime_coverage_proof(&mut coverage, &exec_result.stderr);
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

            let mut failures = parse_findings(&exec_result.stdout).unwrap_or_default();
            failures.extend(differential_findings);
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
            let mut seen_findings = HashSet::new();
            failures.retain(|finding| {
                let replay_case = finding
                    .minimization
                    .minimized
                    .as_ref()
                    .unwrap_or(&finding.minimization.original);
                let key = serde_json::json!({
                    "function": finding.location.function,
                    "severity": finding.severity,
                    "category": finding.category,
                    "oracle": finding.oracle.id,
                    "case": replay_case,
                })
                .to_string();
                seen_findings.insert(key)
            });
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
            let summary = findings_summary(
                &failures,
                suppressed_failures.len(),
                opts.inferred_oracle_gate,
            );
            let fuzz_outcomes = parse_fuzz_outcomes(&exec_result.stdout);
            let valid_invocations = fuzz_outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome.status,
                        FuzzOutcomeStatus::Passed | FuzzOutcomeStatus::Crashed
                    )
                })
                .count();
            let completed_functions: HashSet<&str> = fuzz_outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome.status,
                        FuzzOutcomeStatus::Passed | FuzzOutcomeStatus::Crashed
                    )
                })
                .map(|outcome| outcome.function.as_str())
                .collect();
            let evaluated_oracles = functions_to_fuzz
                .iter()
                .filter(|func| {
                    completed_functions.contains(func.name.as_str())
                        && func
                            .return_type
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                })
                .count();
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
            );
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
                            kind: HarnessKind::Standalone,
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
            let mut detail = serde_json::json!({
                "execution": exec_result,
                "runtime": exec_runtime,
                "module_load_blocked": module_load_blocked,
                "valid_invocations": valid_invocations,
                "evaluated_oracles": evaluated_oracles,
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
                    ExecuteGate::Crash,
                    &failures,
                    opts.inferred_oracle_gate,
                ) || (execute_gate_failed(
                    opts.execute_gate,
                    &failures,
                    opts.inferred_oracle_gate,
                ) && !failures
                    .iter()
                    .filter(|finding| {
                        finding_fails_execute_gate(
                            opts.execute_gate,
                            finding,
                            opts.inferred_oracle_gate,
                        )
                    })
                    .all(|finding| {
                        finding.message.starts_with("Comparator")
                            && finding.minimization.status == MinimizationStatus::Failed
                    }))
                {
                    StageStatus::Failed
                } else {
                    StageStatus::Inconclusive
                },
                duration_ms: exec_ms,
                detail: Some(detail),
                message: if exec_ok {
                    None
                } else {
                    Some(exec_result.stderr.clone())
                },
            });
        } else {
            let coverage = finalize_fuzz_coverage(
                &analysis.functions,
                &functions_to_fuzz,
                &fuzz_plan.coverage,
                module_load_blocked,
            );
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
        }
    }

    // Stage 5: Test (if test_code provided) — this IS authoritative
    if let Some(tests) = opts.test_code {
        let required_candidates = if let Some(diff_text) = opts.diff {
            let ranges = candidate_source_file_owned
                .as_deref()
                .map(|path| diff::parse_changed_lines_for_file(diff_text, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_text));
            analyze::filter_changed_functions(&analysis, &ranges)
        } else {
            analysis.functions.clone()
        };
        let required_functions = required_candidates
            .iter()
            .filter(|function| {
                function.is_exported
                    && !function.is_nested
                    && (!function.is_method
                        || function
                            .invocation_target
                            .as_deref()
                            .is_some_and(|target| !target.trim().is_empty()))
            })
            .collect::<Vec<_>>();
        let runner_probe = if test_code_has_imports(tests, language) {
            tests.to_string()
        } else {
            format!("{code}\n\n{tests}")
        };
        let selected_test_runner = match language {
            Language::TypeScript => match opts.test_runner {
                TestRunner::Auto
                    if sandbox::typescript_code_requires_bun_runtime(&runner_probe) =>
                {
                    TestRunner::Bun
                }
                other => other,
            },
            Language::Python => TestRunner::Auto,
        };
        let prepared = prepare_authoritative_test(
            code,
            tests,
            &required_functions,
            language,
            verification_context.candidate.target_source.mode,
            selected_test_runner,
            Some(candidate_project_dir_owned.as_str()),
            candidate_source_file_owned.as_deref(),
            candidate_test_source_file_owned.as_deref(),
        );
        let execution_project = prepared.project_dir.as_deref();
        let execution_source = prepared.source_file.as_deref();
        let start = Instant::now();
        let test_result = if !prepared.overlay.supported {
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
            let source_mode = harness_context.target_source.mode;
            let (runtime, test_adapter) =
                authoritative_harness_runtime(*language, selected_test_runner);
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
                sandbox_options(
                    &opts,
                    language,
                    test_timeout(),
                    opts.memory_mb,
                    execution_project,
                    execution_source,
                ),
            )
            .await
            .process
        };
        let test_ms = start.elapsed().as_millis() as u64;
        let test_output = format!("{}\n{}", test_result.stdout, test_result.stderr);
        let entered_surfaces = parse_target_entered_events(&test_output);
        let has_assertion_failure =
            test_output.contains("Assertion failed") || test_output.contains("AssertionError");
        let test_ok = test_result.exit_code == Some(0)
            && !test_result.timed_out
            && !test_result.memory_error
            && !has_assertion_failure;
        let covered_required = required_functions
            .iter()
            .filter(|function| {
                entered_surfaces.contains(&format!("{}:{}", function.name, function.line))
            })
            .count();

        let mut test_detail = serde_json::to_value(&test_result).unwrap();
        test_detail["assertion_failure"] = serde_json::Value::Bool(has_assertion_failure);
        test_detail["instrumentation_overlay"] = serde_json::to_value(&prepared.overlay).unwrap();
        test_detail["target_entered_surfaces"] = serde_json::to_value(&entered_surfaces).unwrap();
        test_detail["authoritative_test_covered_surfaces"] =
            serde_json::Value::from(covered_required);
        test_detail["tests_only"] = serde_json::Value::Bool(opts.tests_only);
        test_detail["test_runner_requested"] = serde_json::to_value(opts.test_runner).unwrap();
        test_detail["test_runner_selected"] = serde_json::to_value(selected_test_runner).unwrap();
        stages.push(VerificationStage {
            name: "test".into(),
            status: if !prepared.overlay.supported {
                StageStatus::Inconclusive
            } else if test_ok {
                StageStatus::Passed
            } else {
                StageStatus::Failed
            },
            duration_ms: test_ms,
            detail: Some(test_detail),
            message: if !prepared.overlay.supported {
                prepared.overlay.reason.clone()
            } else if test_ok {
                None
            } else {
                Some(test_result.stderr.clone())
            },
        });

        if opts.tests_only {
            let authoritative_source = candidate_test_source_file_owned
                .as_deref()
                .unwrap_or("<inline>")
                .to_string();
            let coverage_functions = required_functions.iter().map(|function| {
                let surface_id = format!("{}:{}", function.name, function.line);
                let checked = prepared.overlay.supported && test_ok && entered_surfaces.contains(&surface_id);
                FuzzFunctionCoverage {
                    function: function.name.clone(),
                    line: function.line,
                    end_line: function.end_line,
                    status: if checked { FuzzFunctionStatus::CheckedViaAuthoritativeTest } else { FuzzFunctionStatus::SkippedNoFuzzableSurface },
                    required: true,
                    invocation_path: InvocationPath::AuthoritativeTest { source_file: authoritative_source.clone() },
                    is_exported: true,
                    reason: (!checked).then(|| if !prepared.overlay.supported {
                        prepared.overlay.reason.clone().unwrap_or_else(|| "authoritative-test instrumentation is unsupported".into())
                    } else if !test_ok {
                        "authoritative test did not complete successfully".into()
                    } else {
                        "authoritative test did not emit the exact target_entered surface id".into()
                    }),
                }
            }).collect::<Vec<_>>();
            let all_required_checked = !coverage_functions.is_empty()
                && coverage_functions.iter().all(|function| {
                    function.status == FuzzFunctionStatus::CheckedViaAuthoritativeTest
                });
            let coverage_message = if !prepared.overlay.supported {
                prepared.overlay.reason.clone()
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
    }

    finalize_report(
        build_report(stages, opts.coverage_gate),
        opts.output_dir,
        opts.source_file,
        language,
        opts.report_level,
    )
}

fn diagnostic_from_termination(
    termination: &ProcessTermination,
    message: String,
) -> FailureDiagnostic {
    let (domain, kind) = match termination.kind {
        ProcessTerminationKind::TimedOut => (FailureDomain::Resource, FailureKind::Timeout),
        ProcessTerminationKind::MemoryLimit => (FailureDomain::Resource, FailureKind::MemoryLimit),
        ProcessTerminationKind::Signaled => (FailureDomain::Environment, FailureKind::Signal),
        ProcessTerminationKind::LaunchFailed => {
            (FailureDomain::Environment, FailureKind::LauncherFailure)
        }
        ProcessTerminationKind::WaitFailed => {
            (FailureDomain::Environment, FailureKind::ToolFailure)
        }
        ProcessTerminationKind::Exited => (FailureDomain::Environment, FailureKind::NonzeroExit),
    };
    FailureDiagnostic {
        domain,
        kind,
        component: DiagnosticComponent::Sandbox,
        impact: DiagnosticImpact::Blocking,
        message,
        process: Some(termination.clone()),
        limits: None,
    }
}

fn diagnostic_from_stage(
    stage: &VerificationStage,
    coverage_gate: CoverageGate,
) -> Option<FailureDiagnostic> {
    let detail = stage.detail.as_ref();
    let message = stage
        .message
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{} stage did not complete successfully", stage.name));

    let simple = |domain, kind, component, impact| FailureDiagnostic {
        domain,
        kind,
        component,
        impact,
        message: message.clone(),
        process: None,
        limits: None,
    };

    match stage.name.as_str() {
        "context" => Some(simple(
            FailureDomain::Environment,
            FailureKind::ContextResolution,
            DiagnosticComponent::ModuleLoader,
            DiagnosticImpact::Blocking,
        )),
        "parse" => {
            let parse_message = detail
                .and_then(|value| value.get("parse_diagnostics"))
                .and_then(|value| value.as_array())
                .and_then(|values| values.first())
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
                .map(|value| format!("{value} ({}).", message))
                .unwrap_or(message);
            Some(FailureDiagnostic {
                domain: FailureDomain::TargetCode,
                kind: FailureKind::SyntaxError,
                component: DiagnosticComponent::Target,
                impact: DiagnosticImpact::Gating,
                message: parse_message,
                process: None,
                limits: None,
            })
        }
        "complexity" => Some(simple(
            FailureDomain::TargetCode,
            FailureKind::ComplexityThreshold,
            DiagnosticComponent::Target,
            DiagnosticImpact::Gating,
        )),
        "lint" => {
            let runner_failed = detail
                .and_then(|value| value.get("runner_failed"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if runner_failed || stage.status != StageStatus::Advisory {
                Some(simple(
                    FailureDomain::Environment,
                    if runner_failed {
                        FailureKind::ToolFailure
                    } else {
                        FailureKind::LauncherFailure
                    },
                    DiagnosticComponent::LintRunner,
                    DiagnosticImpact::Advisory,
                ))
            } else {
                None
            }
        }
        "coverage" => Some(simple(
            FailureDomain::VerifierHarness,
            FailureKind::ContractViolation,
            DiagnosticComponent::FuzzHarness,
            if coverage_gate == CoverageGate::ChangedExports {
                DiagnosticImpact::Blocking
            } else {
                DiagnosticImpact::Advisory
            },
        )),
        "portability" => Some(simple(
            FailureDomain::Environment,
            FailureKind::ToolFailure,
            DiagnosticComponent::Sandbox,
            DiagnosticImpact::Blocking,
        )),
        "differential" => Some(simple(
            FailureDomain::VerifierHarness,
            FailureKind::AmbiguousGeneratedInput,
            DiagnosticComponent::DifferentialRunner,
            DiagnosticImpact::Advisory,
        )),
        "execute" | "test" => {
            let is_test = stage.name == "test";
            let assertion_failure = is_test
                && (message.contains("Assertion failed")
                    || message.contains("AssertionError")
                    || detail
                        .and_then(|value| value.get("assertion_failure"))
                        .and_then(|value| value.as_bool())
                        == Some(true));
            let has_target_finding = detail
                .and_then(|value| value.get("findings"))
                .and_then(|value| value.as_array())
                .is_some_and(|findings| !findings.is_empty())
                || detail
                    .and_then(|value| value.get("suppressed_findings"))
                    .and_then(|value| value.as_array())
                    .is_some_and(|findings| !findings.is_empty());
            let execution = detail.and_then(|value| value.get("execution")).or(detail);
            let module_load_blocked = detail
                .and_then(|value| value.get("module_load_blocked"))
                .and_then(|value| value.as_bool())
                == Some(true)
                || execution
                    .and_then(|value| value.get("stderr"))
                    .and_then(|value| value.as_str())
                    .is_some_and(is_typescript_module_load_error);
            if module_load_blocked {
                return Some(simple(
                    FailureDomain::Environment,
                    FailureKind::ModuleLoad,
                    DiagnosticComponent::ModuleLoader,
                    DiagnosticImpact::Blocking,
                ));
            }
            if let Some(termination) = execution
                .and_then(|value| value.get("termination"))
                .and_then(|value| serde_json::from_value::<ProcessTermination>(value.clone()).ok())
            {
                // A target finding is authoritative and must not be replaced by
                // a generic nonzero-exit diagnostic. Resource/process causes are
                // retained alongside that finding.
                if !assertion_failure
                    && !(has_target_finding
                        && termination.kind == ProcessTerminationKind::Exited
                        && termination.exit_code != Some(0))
                {
                    return Some(diagnostic_from_termination(&termination, message));
                }
            }

            let overlay_unsupported = detail
                .and_then(|value| value.get("instrumentation_overlay"))
                .and_then(|value| value.get("supported"))
                .and_then(|value| value.as_bool())
                == Some(false);
            if overlay_unsupported {
                return Some(simple(
                    FailureDomain::VerifierHarness,
                    FailureKind::Instrumentation,
                    DiagnosticComponent::Instrumentation,
                    DiagnosticImpact::Blocking,
                ));
            }

            if assertion_failure {
                return Some(simple(
                    FailureDomain::TargetCode,
                    FailureKind::AssertionFailure,
                    DiagnosticComponent::AuthoritativeTestRunner,
                    DiagnosticImpact::Gating,
                ));
            }
            if has_target_finding {
                return Some(simple(
                    FailureDomain::TargetCode,
                    if is_test {
                        FailureKind::AssertionFailure
                    } else {
                        FailureKind::TargetException
                    },
                    if is_test {
                        DiagnosticComponent::AuthoritativeTestRunner
                    } else {
                        DiagnosticComponent::Target
                    },
                    DiagnosticImpact::Gating,
                ));
            }
            if !has_target_finding {
                return Some(simple(
                    if is_test {
                        FailureDomain::Environment
                    } else {
                        FailureDomain::VerifierHarness
                    },
                    if is_test {
                        FailureKind::ToolFailure
                    } else {
                        FailureKind::HarnessProtocol
                    },
                    if is_test {
                        DiagnosticComponent::AuthoritativeTestRunner
                    } else {
                        DiagnosticComponent::FuzzHarness
                    },
                    DiagnosticImpact::Blocking,
                ));
            }
            None
        }
        _ => Some(simple(
            FailureDomain::Environment,
            FailureKind::ToolFailure,
            DiagnosticComponent::Sandbox,
            DiagnosticImpact::Blocking,
        )),
    }
}

fn append_unique_diagnostic(
    diagnostics: &mut Vec<FailureDiagnostic>,
    diagnostic: FailureDiagnostic,
) {
    let key = serde_json::to_string(&diagnostic).unwrap_or_else(|_| diagnostic.message.clone());
    if !diagnostics.iter().any(|existing| {
        serde_json::to_string(existing)
            .map(|value| value == key)
            .unwrap_or(false)
    }) {
        diagnostics.push(diagnostic);
    }
}

fn diagnostics_from_stage_detail(detail: Option<&serde_json::Value>) -> Vec<FailureDiagnostic> {
    let mut diagnostics = Vec::new();
    let Some(detail) = detail else {
        return diagnostics;
    };
    let target_cause = detail
        .get("findings")
        .and_then(|value| value.as_array())
        .is_some_and(|findings| !findings.is_empty())
        || detail
            .get("suppressed_findings")
            .and_then(|value| value.as_array())
            .is_some_and(|findings| !findings.is_empty())
        || detail
            .get("assertion_failure")
            .and_then(|value| value.as_bool())
            == Some(true);
    for key in ["diagnostics", "failure_diagnostics"] {
        if let Some(values) = detail.get(key).and_then(|value| value.as_array()) {
            for value in values {
                if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
                    if target_cause && diagnostic.kind == FailureKind::NonzeroExit {
                        continue;
                    }
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }
    }
    if let Some(value) = detail.get("diagnostic") {
        if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
            append_unique_diagnostic(&mut diagnostics, diagnostic);
        }
    }
    // ExecutionResult is deliberately kept as a nested legacy-compatible
    // object. Promote its typed diagnostics and authoritative termination to
    if let Some(execution) = detail.get("execution") {
        if let Some(values) = execution
            .get("diagnostics")
            .and_then(|value| value.as_array())
        {
            for value in values {
                if let Ok(diagnostic) = serde_json::from_value::<FailureDiagnostic>(value.clone()) {
                    let target_cause = detail
                        .get("findings")
                        .and_then(|value| value.as_array())
                        .is_some_and(|findings| !findings.is_empty())
                        || detail
                            .get("suppressed_findings")
                            .and_then(|value| value.as_array())
                            .is_some_and(|findings| !findings.is_empty())
                        || detail
                            .get("assertion_failure")
                            .and_then(|value| value.as_bool())
                            == Some(true);
                    if target_cause && diagnostic.kind == FailureKind::NonzeroExit {
                        continue;
                    }
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }
    }
    diagnostics
}
pub fn stage_diagnostics(stage: &VerificationStage) -> Vec<FailureDiagnostic> {
    diagnostics_from_stage_detail(stage.detail.as_ref())
}

fn annotate_stage_diagnostics(stages: &mut [VerificationStage], coverage_gate: CoverageGate) {
    for stage in stages {
        let mut diagnostics = diagnostics_from_stage_detail(stage.detail.as_ref());
        let should_infer = matches!(
            stage.status,
            StageStatus::Failed | StageStatus::Inconclusive
        ) || (stage.name == "lint"
            && stage
                .detail
                .as_ref()
                .and_then(|value| value.get("runner_failed"))
                .and_then(|value| value.as_bool())
                == Some(true));

        if stage.name == "execute" || stage.name == "test" {
            let execution = stage
                .detail
                .as_ref()
                .and_then(|value| value.get("execution"))
                .or(stage.detail.as_ref());
            if let Some(execution) = execution {
                if let Some(termination) = execution.get("termination").and_then(|value| {
                    serde_json::from_value::<ProcessTermination>(value.clone()).ok()
                }) {
                    let assertion_failure = stage.name == "test"
                        && (stage.message.as_deref().is_some_and(|message| {
                            message.contains("Assertion failed")
                                || message.contains("AssertionError")
                        }) || stage
                            .detail
                            .as_ref()
                            .and_then(|value| value.get("assertion_failure"))
                            .and_then(|value| value.as_bool())
                            == Some(true));
                    let has_target_finding = stage
                        .detail
                        .as_ref()
                        .and_then(|value| value.get("findings"))
                        .and_then(|value| value.as_array())
                        .is_some_and(|findings| !findings.is_empty())
                        || stage
                            .detail
                            .as_ref()
                            .and_then(|value| value.get("suppressed_findings"))
                            .and_then(|value| value.as_array())
                            .is_some_and(|findings| !findings.is_empty());
                    let module_load_blocked = stage
                        .detail
                        .as_ref()
                        .and_then(|value| value.get("module_load_blocked"))
                        .and_then(|value| value.as_bool())
                        == Some(true)
                        || execution
                            .get("stderr")
                            .and_then(|value| value.as_str())
                            .is_some_and(is_typescript_module_load_error);
                    let should_record_exit = !module_load_blocked
                        && !assertion_failure
                        && (termination.kind != ProcessTerminationKind::Exited
                            || (termination.exit_code != Some(0) && !has_target_finding));
                    if should_record_exit {
                        append_unique_diagnostic(
                            &mut diagnostics,
                            diagnostic_from_termination(
                                &termination,
                                stage.message.clone().unwrap_or_else(|| {
                                    format!("{} process did not exit successfully", stage.name)
                                }),
                            ),
                        );
                    }
                }
            }
        }

        if should_infer {
            if let Some(diagnostic) = diagnostic_from_stage(stage, coverage_gate) {
                // Preserve typed target/resource causes already emitted by the
                // harness, while still guaranteeing one cause for this stage.
                let same_kind = diagnostics
                    .iter()
                    .any(|existing| existing.kind == diagnostic.kind);
                if !same_kind {
                    append_unique_diagnostic(&mut diagnostics, diagnostic);
                }
            }
        }

        if !diagnostics.is_empty() {
            let detail = stage.detail.get_or_insert_with(|| serde_json::json!({}));
            if let Some(object) = detail.as_object_mut() {
                if object
                    .get("diagnostic")
                    .and_then(|value| {
                        serde_json::from_value::<FailureDiagnostic>(value.clone()).ok()
                    })
                    .is_none()
                {
                    let diagnostics_key = if stage.name == "lint" {
                        "failure_diagnostics"
                    } else {
                        "diagnostics"
                    };
                    object.insert(
                        diagnostics_key.into(),
                        serde_json::to_value(&diagnostics)
                            .unwrap_or_else(|_| serde_json::json!([])),
                    );
                }
            }
        }
    }
}

fn diagnostics_from_stages(stages: &[VerificationStage]) -> Vec<FailureDiagnostic> {
    let mut diagnostics = Vec::new();
    for stage in stages {
        for diagnostic in diagnostics_from_stage_detail(stage.detail.as_ref()) {
            append_unique_diagnostic(&mut diagnostics, diagnostic);
        }
    }
    diagnostics
}

fn build_report(mut stages: Vec<VerificationStage>, gate: CoverageGate) -> VerificationReport {
    // Normalize stage-local causes before computing the report-level verdict.
    // This keeps old stage JSON readable while ensuring every failed or
    // inconclusive stage has a typed, deduplicated provenance record.
    annotate_stage_diagnostics(&mut stages, gate);
    let diagnostics = diagnostics_from_stages(&stages);
    let mut summary = compute_report_summary(&stages);
    summary.diagnostics = DiagnosticsSummary::from_diagnostics(&diagnostics);
    let evidence = evidence_from_stages(&stages);
    let (verdict, strength) = final_verdict(&stages, &summary.coverage, gate, &evidence);
    VerificationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        stages,
        verdict,
        strength,
        summary,
        diagnostics_summary: (!diagnostics.is_empty())
            .then(|| DiagnosticsSummary::from_diagnostics(&diagnostics)),
        diagnostics,
        report_path: None,
    }
}

fn evidence_from_stages(stages: &[VerificationStage]) -> VerificationEvidence {
    let parsed = stages
        .iter()
        .any(|stage| stage.name == "parse" && stage.status != StageStatus::Failed);
    let static_checks_completed = parsed
        && stages
            .iter()
            .any(|stage| matches!(stage.name.as_str(), "lint" | "complexity"));
    let mut evidence = VerificationEvidence {
        parsed,
        static_checks_completed,
        ..Default::default()
    };
    for stage in stages {
        if stage.name == "execute" {
            if let Some(detail) = &stage.detail {
                evidence.valid_invocations += detail
                    .get("valid_invocations")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                evidence.evaluated_oracles += detail
                    .get("evaluated_oracles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
            }
        }
        if stage.name == "test" && stage.status == StageStatus::Passed {
            evidence.authoritative_test_completed = true;
        }
    }
    evidence
}

fn coverage_summary_from_stages(stages: &[VerificationStage]) -> CoverageSummary {
    let mut summary = CoverageSummary::default();
    for stage in stages.iter().filter(|stage| stage.name == "coverage") {
        let Some(functions) = stage
            .detail
            .as_ref()
            .and_then(|d| d.get("functions"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for function in functions {
            let required = function
                .get("required")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if required {
                summary.required += 1;
            }
            match function.get("status").and_then(|v| v.as_str()) {
                Some(
                    "checked_direct"
                    | "checked_via_factory"
                    | "checked_via_caller"
                    | "checked_via_authoritative_test",
                ) if required => summary.behaviorally_checked += 1,
                Some("reached_via_factory") if required => summary.reached_only += 1,
                Some("blocked_module_load") => summary.blocked += 1,
                Some(
                    "skipped_no_fuzzable_surface"
                    | "skipped_unsupported_type"
                    | "skipped_internal_helper"
                    | "skipped_method"
                    | "skipped_nested"
                    | "skipped_private_name"
                    | "skipped_diff_filtered",
                ) => summary.skipped += 1,
                _ => {}
            }
        }
        summary.no_inputs_reached += stage
            .detail
            .as_ref()
            .and_then(|d| d.get("no_inputs_reached"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
    }
    summary
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

fn minimal_plan_counts(detail: &serde_json::Value) -> serde_json::Value {
    let plan = detail
        .get("verification_plan")
        .unwrap_or(&serde_json::Value::Null);
    let parameter_domains = plan
        .get("parameter_domains")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let inputs = plan
        .get("inputs")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut source_kind_counts = serde_json::Map::new();
    for parameter in parameter_domains {
        for source in parameter
            .get("sources")
            .and_then(|value| value.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if let Some(kind) = source.get("kind").and_then(|value| value.as_str()) {
                let count = source_kind_counts
                    .get(kind)
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                source_kind_counts.insert(kind.to_string(), serde_json::Value::from(count + 1));
            }
        }
    }
    serde_json::json!({
        "domain_param_count": parameter_domains.len(),
        "closed_domain_param_count": parameter_domains.iter().filter(|parameter| parameter.get("closed").and_then(|value| value.as_bool()).unwrap_or(false)).count(),
        "valid_case_count": inputs.iter().filter(|input| input.get("classification").and_then(|value| value.as_str()) == Some("valid")).count(),
        "invalid_case_count": inputs.iter().filter(|input| input.get("classification").and_then(|value| value.as_str()) == Some("invalid")).count(),
        "source_kind_counts": source_kind_counts,
    })
}

fn minimal_stage_view(stage: &VerificationStage) -> serde_json::Value {
    let mut value = serde_json::json!({
        "name": stage.name,
        "status": stage.status,
        "duration_ms": stage.duration_ms,
    });
    if let Some(message) = &stage.message {
        value["message"] = serde_json::Value::String(message.clone());
    }
    if let Some(detail) = &stage.detail {
        let trimmed = match stage.name.as_str() {
            "complexity" => Some(serde_json::json!({
                "threshold": detail.get("threshold").cloned().unwrap_or(serde_json::Value::Null),
                "metric": detail.get("metric").cloned().unwrap_or(serde_json::Value::Null),
                "checked_functions": detail.get("checked_functions").cloned().unwrap_or(serde_json::Value::Null),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "violations": detail.get("violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "suppressed_violations": detail.get("suppressed_violations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_functions": detail.get("source_directive_functions").cloned().unwrap_or_else(|| serde_json::json!([])),
                "source_directive_suppression_count": detail.get("source_directive_suppression_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
            })),
            "coverage" => Some(serde_json::json!({
                "counts": detail.get("counts").cloned().unwrap_or(serde_json::json!({})),
                "diff_scoped": detail.get("diff_scoped").cloned().unwrap_or(serde_json::Value::Null),
                "seed_input_count": detail.get("seed_input_count").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seeded_functions": detail.get("seeded_functions").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "seed_sources": detail.get("seed_sources").cloned().unwrap_or_else(|| serde_json::json!([])),
                "inferred_context_properties": detail.get("inferred_context_properties").cloned().unwrap_or_else(|| serde_json::json!({})),
                "plan": minimal_plan_counts(detail),
            })),
            "execute" => Some(serde_json::json!({
                "runtime": detail.get("runtime").cloned().unwrap_or(serde_json::Value::Null),
                "valid_invocations": detail.get("valid_invocations").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "evaluated_oracles": detail.get("evaluated_oracles").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "no_inputs_reached": detail.get("no_inputs_reached").cloned().unwrap_or_else(|| serde_json::Value::from(0)),
                "findings": detail.get("findings").cloned().unwrap_or_else(|| serde_json::json!([])),
                "suppressed_findings": detail.get("suppressed_findings").cloned().unwrap_or_else(|| serde_json::json!([])),
                "findings_summary": detail.get("findings_summary").cloned().unwrap_or_else(|| serde_json::json!({})),
                "plan": minimal_plan_counts(detail),
            })),
            "lint" => Some(serde_json::json!({
                "diagnostics": detail.get("diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_diagnostics": detail.get("runner_diagnostics").cloned().unwrap_or_else(|| serde_json::json!([])),
                "runner_failed": detail.get("runner_failed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "unavailable": detail.get("unavailable").cloned().unwrap_or(serde_json::Value::Bool(false)),
            })),
            "portability" => Some(serde_json::json!({
                "reason": detail.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                "failing_imports": detail.get("failing_imports").cloned().unwrap_or_else(|| serde_json::json!([])),
                "fix_hint": detail.get("fix_hint").cloned().unwrap_or(serde_json::Value::Null),
                "suppressed": detail.get("suppressed").cloned().unwrap_or(serde_json::Value::Bool(false)),
                "repo_runtime": detail.get("repo_runtime").cloned().unwrap_or(serde_json::Value::Null),
                "node_result": serde_json::json!({
                    "stderr": detail
                        .get("node_result")
                        .and_then(|node| node.get("stderr"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }),
            })),
            _ => None,
        };
        if let Some(trimmed) = trimmed {
            value["detail"] = trimmed;
        }
    }
    value
}

pub fn report_json_value(
    report: &VerificationReport,
    report_level: ReportLevel,
) -> serde_json::Value {
    match report_level {
        ReportLevel::Minimal => serde_json::json!({
            "schema_version": report.schema_version,
            "verdict": report.verdict,
            "strength": report.strength,
            "summary": report.summary,
            "diagnostics": report.diagnostics,
            "diagnostics_summary": report.diagnostics_summary,
            "report_path": report.report_path,
            "stages": report
                .stages
                .iter()
                .map(minimal_stage_view)
                .collect::<Vec<_>>(),
        }),
        ReportLevel::Full => serde_json::to_value(report).unwrap_or_else(|_| serde_json::json!({})),
    }
}

fn clip_human(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(limit).collect();
    format!("{clipped}...")
}

fn human_number(detail: &serde_json::Value, key: &str) -> usize {
    detail
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize
}
fn stage_status_text(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Passed => "PASS",
        StageStatus::Failed => "FAIL",
        StageStatus::Inconclusive => "INCONCLUSIVE",
        StageStatus::Advisory => "ADVISORY",
        StageStatus::Skipped => "SKIPPED",
    }
}

pub fn report_human_summary(report: &VerificationReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Overall: {:?} ({:?})", report.verdict, report.strength);
    if let Some(path) = &report.report_path {
        let _ = writeln!(out, "Report Path: {path}");
    }

    let summary = &report.summary;
    let _ = writeln!(
        out,
        "Coverage: {} analyzed, {} fuzzed, {} skipped, {} module-load blocked",
        summary.functions_analyzed,
        summary.functions_fuzzed,
        summary.functions_skipped,
        summary.functions_blocked_module_load
    );
    let _ = writeln!(
        out,
        "Execute: {} findings ({} gating, {} advisory, {} suppressed)",
        summary.findings.total,
        summary.findings.gating,
        summary.findings.advisory,
        summary.findings.suppressed
    );
    let _ = writeln!(
        out,
        "Lint: {} issues, {} runner failures",
        summary.lint_issues, summary.lint_runner_failures
    );
    let _ = writeln!(
        out,
        "Complexity: {} violations, {} suppressed",
        summary.complexity_violations, summary.suppressed_complexity_violations
    );
    if !report.diagnostics.is_empty() {
        let _ = writeln!(out, "Diagnostics: {}", report.diagnostics.len());
        for diagnostic in &report.diagnostics {
            let _ = writeln!(
                out,
                "  {:?}/{:?} ({:?}, {:?}): {}",
                diagnostic.domain,
                diagnostic.kind,
                diagnostic.component,
                diagnostic.impact,
                clip_human(&diagnostic.message, 160)
            );
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "Stages:");
    for stage in &report.stages {
        let mut extra = String::new();
        if let Some(detail) = &stage.detail {
            match stage.name.as_str() {
                "execute" => {
                    let crash = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("crash"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let property = detail
                        .get("finding_counts")
                        .and_then(|counts| counts.get("property_violation"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let no_inputs = human_number(detail, "no_inputs_reached");
                    extra = format!("crash={crash}, property={property}, no_inputs={no_inputs}");
                }
                "coverage" => {
                    let counts = detail.get("counts").cloned().unwrap_or_default();
                    let checked = [
                        "checked_direct",
                        "checked_via_factory",
                        "checked_via_caller",
                        "checked_via_authoritative_test",
                    ]
                    .iter()
                    .map(|key| {
                        counts
                            .get(*key)
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                    })
                    .sum::<u64>();
                    let factory = counts
                        .get("checked_via_factory")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let reached = counts
                        .get("reached_via_factory")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let skipped = counts
                        .as_object()
                        .map(|obj| {
                            obj.iter()
                                .filter(|(key, _)| {
                                    !key.starts_with("checked_") && *key != "reached_via_factory"
                                })
                                .map(|(_, value)| value.as_u64().unwrap_or(0))
                                .sum()
                        })
                        .unwrap_or(0);
                    extra = format!("checked={checked}, factory={factory}, reached={reached}, skipped={skipped}");
                }
                "lint" => {
                    let issues = detail
                        .get("diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let runner_failures = detail
                        .get("runner_diagnostics")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let unavailable = detail
                        .get("unavailable")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    extra = format!(
                        "issues={issues}, runner_failures={runner_failures}, unavailable={unavailable}"
                    );
                }
                "complexity" => {
                    let violations = detail
                        .get("violations")
                        .and_then(|value| value.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    let threshold = human_number(detail, "threshold");
                    extra = format!("violations={violations}, threshold={threshold}");
                }
                _ => {}
            }
        }
        let _ = if extra.is_empty() {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms",
                stage.name,
                stage_status_text(stage.status),
                stage.duration_ms
            )
        } else {
            writeln!(
                out,
                "  {:<12} {:<4} {:>5} ms  {}",
                stage.name,
                stage_status_text(stage.status),
                stage.duration_ms,
                extra
            )
        };
        if let Some(message) = &stage.message {
            let _ = writeln!(out, "    {}", clip_human(message, 160));
        }
    }

    if let Some(complexity_stage) = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
    {
        if let Some(violations) = complexity_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("violations"))
            .and_then(|value| value.as_array())
        {
            if !violations.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Complexity Offenders:");
                for (idx, violation) in violations.iter().take(5).enumerate() {
                    let function = violation
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let line = violation
                        .get("line")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cyclomatic = violation
                        .get("complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let cognitive = violation
                        .get("cognitive_complexity")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let _ = writeln!(
                        out,
                        "  {}. {} (line {}) cyclomatic={} cognitive={}",
                        idx + 1,
                        function,
                        line,
                        cyclomatic,
                        cognitive
                    );
                }
            }
        }
    }

    if let Some(execute_stage) = report.stages.iter().find(|stage| stage.name == "execute") {
        if let Some(failures) = execute_stage
            .detail
            .as_ref()
            .and_then(|detail| detail.get("findings"))
            .and_then(|value| value.as_array())
        {
            if !failures.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "Top Execute Findings:");
                for (idx, failure) in failures.iter().take(5).enumerate() {
                    let function = failure
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<unknown>");
                    let severity = failure
                        .get("severity")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown");
                    let message = failure
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let _ = writeln!(
                        out,
                        "  {}. {} [{}] {}",
                        idx + 1,
                        function,
                        severity,
                        clip_human(message, 140)
                    );
                }
            }
        }
    }

    out.trim_end().to_string()
}

fn compute_report_summary(stages: &[VerificationStage]) -> ReportSummary {
    let mut summary = ReportSummary {
        functions_analyzed: 0,
        functions_fuzzed: 0,
        functions_skipped: 0,
        functions_blocked_module_load: 0,
        fuzz_pass: 0,
        fuzz_no_inputs_reached: 0,
        findings: FindingsSummary::default(),
        suppressed_complexity_violations: 0,
        suppressed_portability_warnings: 0,
        lint_issues: 0,
        lint_runner_failures: 0,
        complexity_violations: 0,
        coverage: CoverageSummary::default(),
        diagnostics: DiagnosticsSummary::default(),
    };
    summary.coverage = coverage_summary_from_stages(stages);
    for stage in stages {
        let Some(detail) = &stage.detail else {
            continue;
        };
        match stage.name.as_str() {
            "parse" => {
                summary.functions_analyzed = detail
                    .get("functions")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0)
            }
            "coverage" => {
                if let Some(funcs) = detail.get("functions").and_then(|v| v.as_array()) {
                    for func in funcs {
                        match func.get("status").and_then(|v| v.as_str()) {
                            Some(
                                "checked_direct"
                                | "checked_via_factory"
                                | "checked_via_caller"
                                | "checked_via_authoritative_test",
                            ) => summary.functions_fuzzed += 1,
                            Some("blocked_module_load") => {
                                summary.functions_blocked_module_load += 1
                            }
                            Some(_) => summary.functions_skipped += 1,
                            None => {}
                        }
                    }
                }
            }
            "execute" => {
                summary.findings = detail
                    .get("findings_summary")
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .unwrap_or_default();
                summary.fuzz_pass = detail
                    .get("valid_invocations")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                summary.fuzz_no_inputs_reached = detail
                    .get("no_inputs_reached")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
            }
            "lint" => {
                let runner_failed = detail
                    .get("runner_failed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                summary.lint_issues = if runner_failed {
                    0
                } else {
                    detail
                        .get("diagnostics")
                        .and_then(|v| v.as_array())
                        .map(|v| v.len())
                        .unwrap_or(0)
                };
                if runner_failed {
                    summary.lint_runner_failures += 1;
                }
            }
            "complexity" => {
                summary.complexity_violations = detail
                    .get("violations")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                summary.suppressed_complexity_violations = detail
                    .get("suppressed_violations")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
            }
            "portability"
                if detail
                    .get("suppressed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false) =>
            {
                summary.suppressed_portability_warnings += 1
            }
            _ => {}
        }
    }
    summary.coverage.no_inputs_reached = summary.fuzz_no_inputs_reached;
    summary
}

fn set_repro_commands(value: &mut serde_json::Value, report_path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            let finding_id = map
                .get("id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            if let (Some(finding_id), Some(repro)) = (
                finding_id,
                map.get_mut("repro").and_then(|value| value.as_object_mut()),
            ) {
                repro.insert(
                    "command".into(),
                    serde_json::Value::String(format!(
                        "court-jester replay --report {report_path} --finding {finding_id}"
                    )),
                );
            }
            for child in map.values_mut() {
                set_repro_commands(child, report_path);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                set_repro_commands(child, report_path);
            }
        }
        _ => {}
    }
}

fn write_report(
    output_dir: &str,
    report: &VerificationReport,
    source_file: Option<&str>,
    language: &Language,
    _report_level: ReportLevel,
) -> Option<String> {
    use chrono::Utc;

    let _ = std::fs::create_dir_all(output_dir);
    let total_duration = report
        .stages
        .iter()
        .map(|stage| stage.duration_ms)
        .sum::<u64>();

    let now = Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let file_timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();

    let persisted = PersistedReport {
        schema_version: REPORT_SCHEMA_VERSION,
        meta: ReportMeta {
            source_file: source_file.map(|s| s.to_string()),
            language: format!("{:?}", language).to_lowercase(),
            timestamp,
            duration_ms: total_duration,
        },
        stages: report.stages.clone(),
        verdict: report.verdict,
        strength: report.strength,
        summary: report.summary.clone(),
        diagnostics: report.diagnostics.clone(),
        diagnostics_summary: report.diagnostics_summary.clone(),
    };
    let basename = source_file
        .map(|s| {
            std::path::Path::new(s)
                .file_stem()
                .and_then(|os| os.to_str())
                .unwrap_or("inline")
                .to_string()
        })
        .unwrap_or_else(|| "inline".to_string());

    let filename = format!("{file_timestamp}-{basename}.json");
    let path = std::path::Path::new(output_dir).join(&filename);

    let mut json_value = serde_json::to_value(&persisted).ok()?;
    set_repro_commands(&mut json_value, path.to_string_lossy().as_ref());

    match serde_json::to_string_pretty(&json_value) {
        Ok(json) => {
            if std::fs::write(&path, &json).is_ok() {
                Some(path.to_string_lossy().to_string())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Parse schema-v3 findings emitted by the generated harness.
pub fn parse_findings(stdout: &str) -> Option<Vec<VerificationFinding>> {
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
pub fn repair_summary(report: &VerificationReport) -> RepairSummary {
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
                    && diagnostic.kind != FailureKind::ContractViolation
            });
            let infrastructure = non_target_blocker
                || findings.iter().any(|finding| {
                    !finding.suppressed && finding.severity == FindingSeverity::Infrastructure
                })
                || report.stages.iter().any(|stage| {
                    stage.status == StageStatus::Inconclusive
                        && matches!(stage.name.as_str(), "portability" | "execute")
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
        verdict: report.verdict,
        strength: report.strength,
        recommended_action,
        primary_finding,
        findings,
        coverage: report.summary.coverage.clone(),
        diagnostics: report.diagnostics.clone(),
        diagnostics_summary: report.diagnostics_summary.clone(),
    }
}

fn persisted_findings(report: &PersistedReport) -> Vec<VerificationFinding> {
    findings_from_stages(&report.stages)
}

pub fn load_persisted_report(path: &str) -> Result<PersistedReport, String> {
    let bytes = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read report '{path}': {error}"))?;
    let report: PersistedReport = serde_json::from_str(&bytes)
        .map_err(|error| format!("invalid persisted report: {error}"))?;
    if report.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported report schema {}; expected {}",
            report.schema_version, REPORT_SCHEMA_VERSION
        ));
    }
    Ok(report)
}
pub fn replay_launch_context(
    report_path: &str,
    finding_id: &str,
) -> Result<Option<ReproLaunchContext>, String> {
    let report = load_persisted_report(report_path)?;
    let mut matches = persisted_findings(&report)
        .into_iter()
        .filter(|finding| finding.id == finding_id);
    let finding = matches
        .next()
        .ok_or_else(|| format!("finding '{finding_id}' was not found in report"))?;
    if matches.next().is_some() {
        return Err(format!("finding id '{finding_id}' is duplicated"));
    }
    Ok(finding.launch_context)
}

fn replay_payload(stdout: &str) -> Result<serde_json::Value, String> {
    const MARKER: &str = "__COURT_JESTER_REPLAY_JSON__";
    if stdout.matches(MARKER).count() != 1 {
        return Err("replay sentinel must occur exactly once".into());
    }
    let after = stdout
        .split_once(MARKER)
        .map(|(_, value)| value.trim())
        .unwrap_or_default();
    let line = after.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return Err("replay sentinel has no JSON payload".into());
    }
    serde_json::from_str(line).map_err(|error| format!("invalid replay sentinel JSON: {error}"))
}

fn validate_differential_repro(
    differential: &DifferentialRepro,
    dependency_project_dir: Option<&str>,
) -> Result<(), String> {
    for source in differential
        .base_files
        .iter()
        .chain(differential.candidate_files.iter())
    {
        if stable_digest(&source.content) != source.sha256 {
            return Err(format!(
                "embedded source digest mismatch for {}",
                source.relative_path
            ));
        }
    }
    let tree_digest = |files: &[EmbeddedSource]| {
        if files.len() == 1 {
            return stable_digest(&files[0].content);
        }
        let mut entries = files
            .iter()
            .map(|source| format!("{}\n{}", source.relative_path, source.content))
            .collect::<Vec<_>>();
        entries.sort();
        stable_digest(&entries.join("\n"))
    };
    if tree_digest(&differential.base_files) != differential.base_tree_sha256 {
        return Err("embedded base tree digest mismatch".into());
    }
    if tree_digest(&differential.candidate_files) != differential.candidate_tree_sha256 {
        return Err("embedded candidate tree digest mismatch".into());
    }
    if !differential
        .dependency_contract
        .third_party_modules
        .is_empty()
        && dependency_project_dir.is_none()
    {
        return Err("replay requires --dependency-project-dir for third-party modules".into());
    }
    if let Some(root) = dependency_project_dir {
        for lockfile in &differential.dependency_contract.lockfiles {
            let path = Path::new(root).join(&lockfile.relative_path);
            let content = std::fs::read_to_string(&path)
                .map_err(|error| format!("dependency lockfile unavailable: {error}"))?;
            if stable_digest(&content) != lockfile.sha256 {
                return Err(format!(
                    "dependency lockfile digest mismatch for {}",
                    lockfile.relative_path
                ));
            }
        }
    }
    Ok(())
}
fn materialize_embedded_tree(
    files: &[EmbeddedSource],
    relative_entry: &str,
    label: &str,
) -> Result<(tempfile::TempDir, String, String), String> {
    let root = tempfile::tempdir()
        .map_err(|error| format!("failed to create {label} replay root: {error}"))?;
    let mut entry_content = None;
    let mut entry_path = None;
    for embedded in files {
        let relative = Path::new(&embedded.relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "invalid embedded source path '{}'",
                embedded.relative_path
            ));
        }
        let destination = root.path().join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to materialize {label} replay source: {error}"))?;
        }
        std::fs::write(&destination, &embedded.content)
            .map_err(|error| format!("failed to materialize {label} replay source: {error}"))?;
        if embedded.relative_path == relative_entry {
            entry_content = Some(embedded.content.clone());
            entry_path = Some(destination.to_string_lossy().to_string());
        }
    }
    match (entry_content, entry_path) {
        (Some(content), Some(path)) => Ok((root, content, path)),
        _ => Err(format!(
            "differential entry '{relative_entry}' is absent from embedded {label} sources"
        )),
    }
}

pub async fn replay_report(
    report_path: &str,
    finding_id: &str,
    dependency_project_dir: Option<&str>,
    runtime_profile: RuntimeProfile,
    python_docker_image: &str,
    typescript_docker_image: &str,
) -> Result<ReplayReport, String> {
    replay_report_with_options(
        report_path,
        finding_id,
        dependency_project_dir,
        runtime_profile,
        python_docker_image,
        typescript_docker_image,
        None,
        None,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn replay_report_with_options(
    report_path: &str,
    finding_id: &str,
    dependency_project_dir: Option<&str>,
    runtime_profile: RuntimeProfile,
    python_docker_image: &str,
    typescript_docker_image: &str,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
    network: Option<NetworkPolicy>,
    harness_args: Option<&[HarnessArg]>,
) -> Result<ReplayReport, String> {
    let report = load_persisted_report(report_path)?;
    let mut matches = persisted_findings(&report)
        .into_iter()
        .filter(|finding| finding.id == finding_id);
    let finding = matches
        .next()
        .ok_or_else(|| format!("finding '{finding_id}' was not found in report"))?;
    if matches.next().is_some() {
        return Err(format!("finding id '{finding_id}' is duplicated"));
    }
    let language = Language::parse(&report.meta.language).ok_or_else(|| {
        format!(
            "unsupported report language '{}'; expected python or typescript",
            report.meta.language
        )
    })?;

    let launch_context = finding.launch_context.as_ref();
    let docker_image = match language {
        Language::Python => python_docker_image,
        Language::TypeScript => typescript_docker_image,
    };
    let replay_timeout = timeout_seconds
        .or_else(|| launch_context.map(|context| context.limits.timeout_seconds))
        .unwrap_or(10.0);
    let replay_memory = memory_mb
        .or_else(|| launch_context.map(|context| context.limits.memory_mb))
        .unwrap_or(128);
    let replay_network = network
        .or_else(|| launch_context.map(|context| context.limits.network_policy))
        .unwrap_or(NetworkPolicy::Deny);
    let replay_harness_args = harness_args.unwrap_or_else(|| {
        launch_context
            .map(|context| context.harness_args.as_slice())
            .unwrap_or(&[])
    });
    if let Some(differential) = finding.repro.differential.as_ref() {
        if let Err(reason) = validate_differential_repro(differential, dependency_project_dir) {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(&reason),
            });
        }
        let (base_root, base_source, base_entry) = match materialize_embedded_tree(
            &differential.base_files,
            &differential.relative_entry,
            "base",
        ) {
            Ok(materialized) => materialized,
            Err(reason) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&reason),
                })
            }
        };
        let (candidate_root, candidate_source, candidate_entry) = match materialize_embedded_tree(
            &differential.candidate_files,
            &differential.relative_entry,
            "candidate",
        ) {
            Ok(materialized) => materialized,
            Err(reason) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&reason),
                })
            }
        };
        let Some(symbol) = finding.repro.function.as_deref() else {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result("differential repro has no function symbol"),
            });
        };
        let base_context = match crate::resolve_execution_context(ContextRequest {
            invocation_dir: base_root.path(),
            explicit_project_dir: Some(base_root.path()),
            target_file: Some(Path::new(&base_entry)),
            test_file: None,
            language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay base context unavailable: {error}"
                    )),
                })
            }
        };
        let candidate_context = match crate::resolve_execution_context(ContextRequest {
            invocation_dir: candidate_root.path(),
            explicit_project_dir: Some(candidate_root.path()),
            target_file: Some(Path::new(&candidate_entry)),
            test_file: None,
            language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay candidate context unavailable: {error}"
                    )),
                })
            }
        };
        let base_analysis =
            analyze::analyze_with_context(&base_source, &base_context.target_source);
        let candidate_analysis =
            analyze::analyze_with_context(&candidate_source, &candidate_context.target_source);
        let base_function = base_analysis
            .functions
            .iter()
            .find(|function| function.name == symbol);
        let candidate_function = candidate_analysis
            .functions
            .iter()
            .find(|function| function.name == symbol);
        let (Some(base_function), Some(candidate_function)) = (base_function, candidate_function)
        else {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay surface is absent from an embedded tree",
                ),
            });
        };
        if !compatible_surface(candidate_function, base_function) {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "embedded differential surface signatures are incompatible",
                ),
            });
        }
        let Some(differential_case) = differential_case_from_arguments(
            candidate_function,
            &finding.repro.arguments,
            &language,
        ) else {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay arguments do not match the stored surface bindings",
                ),
            });
        };
        let base_probe =
            differential_probe(&base_source, base_function, &differential_case, &language);
        let candidate_probe = differential_probe(
            &candidate_source,
            candidate_function,
            &differential_case,
            &language,
        );
        let base_options = SandboxOptions {
            timeout_seconds: replay_timeout,
            memory_mb: replay_memory,
            runtime_profile,
            network_policy: replay_network,
            harness_args: replay_harness_args,
            docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
            project_dir: base_root.path().to_str(),
            source_file: Some(&base_entry),
        };
        let candidate_options = SandboxOptions {
            timeout_seconds: replay_timeout,
            memory_mb: replay_memory,
            runtime_profile,
            network_policy: replay_network,
            harness_args: replay_harness_args,
            docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
            project_dir: candidate_root.path().to_str(),
            source_file: Some(&candidate_entry),
        };
        base_options.validate()?;
        candidate_options.validate()?;
        let base_execution = sandbox::execute(&base_probe, &language, base_options).await;
        let candidate_execution =
            sandbox::execute(&candidate_probe, &language, candidate_options).await;
        let base_snapshot = match differential_snapshot(&base_execution) {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay baseline snapshot unsupported: {reason}"
                    )),
                })
            }
        };
        let candidate_snapshot = match differential_snapshot(&candidate_execution) {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                return Ok(ReplayReport {
                    schema_version: REPORT_SCHEMA_VERSION,
                    finding_id: finding.id,
                    outcome: ReplayOutcome::Inconclusive,
                    execution: err_execution_result(&format!(
                        "differential replay candidate snapshot unsupported: {reason}"
                    )),
                })
            }
        };
        if differential_binding_failure(&base_snapshot, &language)
            || differential_binding_failure(&candidate_snapshot, &language)
            || (base_snapshot == candidate_snapshot && base_snapshot.exception_type.is_some())
        {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "differential replay case is an invalid generated invocation",
                ),
            });
        }
        let reproduced = base_snapshot != candidate_snapshot;
        let payload = serde_json::json!({
            "reproduced": reproduced,
            "severity": finding.repro.expectation.severity,
            "oracle_kind": finding.repro.expectation.oracle_kind,
            "category": finding.repro.expectation.category,
        });
        let execution = ExecutionResult {
            stdout: format!(
                "__COURT_JESTER_REPLAY_JSON__{}\n",
                serde_json::to_string(&payload).unwrap_or_default()
            ),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: base_execution
                .duration_ms
                .saturating_add(candidate_execution.duration_ms),
            timed_out: false,
            memory_error: false,
            termination: Some(ProcessTermination {
                kind: ProcessTerminationKind::Exited,
                exit_code: Some(0),
                signal: None,
                signal_name: None,
            }),
            diagnostics: vec![],
        };
        return Ok(ReplayReport {
            schema_version: REPORT_SCHEMA_VERSION,
            finding_id: finding.id,
            outcome: if reproduced {
                ReplayOutcome::Reproduced
            } else {
                ReplayOutcome::NotReproduced
            },
            execution,
        });
    }

    let mut source_file_owned = None;
    let mut source = String::new();
    if let Some(path) = report.meta.source_file.as_deref() {
        let source_path = if Path::new(path).is_file() {
            PathBuf::from(path)
        } else if let Some(root) = dependency_project_dir {
            Path::new(root).join(path)
        } else {
            return Ok(ReplayReport {
                schema_version: REPORT_SCHEMA_VERSION,
                finding_id: finding.id,
                outcome: ReplayOutcome::Inconclusive,
                execution: err_execution_result(
                    "relative replay source requires --dependency-project-dir",
                ),
            });
        };
        source = std::fs::read_to_string(&source_path)
            .map_err(|error| format!("source context unavailable for replay: {error}"))?;
        source_file_owned = Some(source_path.to_string_lossy().to_string());
    }
    let code = if source.is_empty() {
        finding.repro.snippet.clone()
    } else {
        format!("{source}\n{}", finding.repro.snippet)
    };
    let source_file = source_file_owned.as_deref();
    let project_dir_owned = dependency_project_dir.map(ToOwned::to_owned).or_else(|| {
        source_file.and_then(|path| {
            Path::new(path)
                .parent()
                .and_then(|parent| parent.to_str())
                .map(ToOwned::to_owned)
        })
    });
    let project_dir = project_dir_owned.as_deref();
    let options = SandboxOptions {
        timeout_seconds: replay_timeout,
        memory_mb: replay_memory,
        runtime_profile,
        network_policy: replay_network,
        harness_args: replay_harness_args,
        docker_image: (runtime_profile == RuntimeProfile::Isolated).then_some(docker_image),
        project_dir,
        source_file,
    };
    options.validate()?;
    let execution = sandbox::execute(&code, &language, options).await;
    let outcome = match replay_payload(&execution.stdout) {
        Ok(payload) => {
            let reproduced = payload
                .get("reproduced")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let expected = |value: serde_json::Value| value.as_str().map(ToOwned::to_owned);
            let matches_expectation = payload.get("severity").and_then(|value| value.as_str())
                == expected(
                    serde_json::to_value(finding.repro.expectation.severity).unwrap_or_default(),
                )
                .as_deref()
                && payload.get("oracle_kind").and_then(|value| value.as_str())
                    == expected(
                        serde_json::to_value(finding.repro.expectation.oracle_kind)
                            .unwrap_or_default(),
                    )
                    .as_deref()
                && payload.get("category").and_then(|value| value.as_str())
                    == expected(
                        serde_json::to_value(finding.repro.expectation.category)
                            .unwrap_or_default(),
                    )
                    .as_deref();
            if matches_expectation {
                if reproduced {
                    ReplayOutcome::Reproduced
                } else {
                    ReplayOutcome::NotReproduced
                }
            } else {
                ReplayOutcome::Inconclusive
            }
        }
        Err(_) => ReplayOutcome::Inconclusive,
    };
    Ok(ReplayReport {
        schema_version: REPORT_SCHEMA_VERSION,
        finding_id: finding.id,
        outcome,
        execution,
    })
}
