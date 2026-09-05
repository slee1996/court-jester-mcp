use std::collections::{BTreeMap, HashSet};

use crate::types::{FailureDiagnostic, FailureDomain, FailureKind, ProcessTerminationKind};

mod docker_lifecycle;
mod events;
pub use docker_lifecycle::wait_for_docker_cleanup;
use docker_lifecycle::{supervise_docker_lifecycle, DockerLifecycle};
mod process;
pub(crate) use process::run_command_with_limits;
use process::{launch_failure, run_launch_command, termination};

pub use events::{
    parse_harness_events, HarnessEventSummary, HarnessSurfaceEvidence,
    HARNESS_EVENT_MAX_LINE_BYTES, HARNESS_EVENT_MAX_RECORDS, HARNESS_EVENT_PROTOCOL_VERSION,
    HARNESS_EVENT_SENTINEL,
};

/// Describe the in-memory instrumentation overlay used for authoritative tests.
/// The original source is never rewritten; unsupported runner/import modes are
/// explicit and must be treated as inconclusive by verification.
pub fn build_instrumentation_overlay(
    language: &Language,
    runner: TestRunner,
    source_file: &str,
    surfaces: &[String],
) -> InstrumentationOverlay {
    let (mode, supported) = match language {
        Language::Python => (InstrumentationMode::PythonSitecustomize, true),
        Language::TypeScript => match runner {
            TestRunner::Bun => (InstrumentationMode::BunPreload, true),
            TestRunner::Node | TestRunner::Auto => (InstrumentationMode::NodeModuleRegister, true),
            TestRunner::RepoNative => (InstrumentationMode::Unsupported, false),
        },
    };
    InstrumentationOverlay {
        mode,
        source_file: source_file.to_string(),
        surfaces: surfaces.to_vec(),
        supported,
        reason: (!supported)
            .then(|| "repo-native runner does not expose a module transform hook".into()),
    }
}

use std::time::Instant;
use tokio::process::Command;

use crate::types::*;

/// Find a binary on the given PATH, returning its absolute path if found.
fn which_binary(path_env: &str, binary: &str) -> Option<String> {
    for dir in path_env.split(':') {
        let candidate = std::path::Path::new(dir).join(binary);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn vitest_package_entrypoint(
    package_dir: &std::path::Path,
    expected_entrypoint: Option<&std::path::Path>,
) -> Result<(std::path::PathBuf, bool), String> {
    let package_dir = std::fs::canonicalize(package_dir)
        .map_err(|error| format!("failed to resolve the Vitest package: {error}"))?;
    let manifest = std::fs::read_to_string(package_dir.join("package.json"))
        .map_err(|error| format!("failed to read the Vitest package manifest: {error}"))?;
    let package: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("failed to parse the Vitest package manifest: {error}"))?;
    if package.get("name").and_then(serde_json::Value::as_str) != Some("vitest") {
        return Err("Vitest package manifest has an unexpected package name".into());
    }
    let bin = package
        .get("bin")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("vitest").and_then(serde_json::Value::as_str))
        })
        .ok_or_else(|| {
            "Vitest package manifest does not declare its JavaScript entrypoint".to_string()
        })?;
    let entrypoint = std::fs::canonicalize(package_dir.join(bin))
        .map_err(|error| format!("failed to resolve the Vitest JavaScript entrypoint: {error}"))?;
    if !entrypoint.starts_with(&package_dir) {
        return Err("Vitest JavaScript entrypoint escapes its package directory".into());
    }
    if expected_entrypoint.is_some_and(|expected| entrypoint != expected) {
        return Err("Vitest launcher target does not match its package manifest entrypoint".into());
    }
    let legacy_threads = package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse::<u64>().ok())
        == Some(0);
    Ok((entrypoint, legacy_threads))
}

fn vitest_project_entrypoint(
    executable: &std::path::Path,
) -> Result<(std::path::PathBuf, bool), String> {
    let canonical_executable = std::fs::canonicalize(executable)
        .map_err(|error| format!("failed to resolve the Vitest executable: {error}"))?;
    let canonical_package = canonical_executable.ancestors().skip(1).find(|directory| {
        let parent = directory.parent();
        directory.file_name() != Some(std::ffi::OsStr::new(".bin"))
            && (parent.and_then(std::path::Path::file_name)
                == Some(std::ffi::OsStr::new("node_modules"))
                || (parent
                    .and_then(std::path::Path::file_name)
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.starts_with('@'))
                    && parent
                        .and_then(std::path::Path::parent)
                        .and_then(std::path::Path::file_name)
                        == Some(std::ffi::OsStr::new("node_modules"))))
    });
    if let Some(package_dir) = canonical_package {
        return vitest_package_entrypoint(package_dir, Some(&canonical_executable));
    }

    let metadata = std::fs::symlink_metadata(executable)
        .map_err(|error| format!("failed to inspect the Vitest executable: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("Vitest launcher symlink does not resolve into a Vitest package".into());
    }

    let bin_dir = executable
        .parent()
        .filter(|directory| directory.file_name() == Some(std::ffi::OsStr::new(".bin")))
        .ok_or_else(|| "Vitest executable is not inside node_modules/.bin".to_string())?;
    let node_modules = bin_dir
        .parent()
        .filter(|directory| directory.file_name() == Some(std::ffi::OsStr::new("node_modules")))
        .ok_or_else(|| "Vitest executable is not inside node_modules/.bin".to_string())?;
    vitest_package_entrypoint(&node_modules.join("vitest"), None)
}

fn is_valid_python_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn resolved_typescript_import(value: &str, source_dir: &std::path::Path) -> Option<String> {
    if !(value.starts_with("./") || value.starts_with("../"))
        || value.contains('?')
        || value.contains('#')
        || std::path::Path::new(value).extension().is_some()
    {
        return None;
    }
    let path = source_dir.join(value);
    for extension in ["ts", "tsx", "js", "jsx"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Some(format!("{value}.{extension}"));
        }
    }
    for extension in ["ts", "tsx", "js", "jsx"] {
        if path.join(format!("index.{extension}")).is_file() {
            return Some(format!("{value}/index.{extension}"));
        }
    }
    None
}

fn typescript_runtime_identifiers(code: &str) -> HashSet<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .expect("Failed to load TypeScript grammar");
    let Some(tree) = parser.parse(code, None) else {
        return HashSet::new();
    };
    let mut identifiers = HashSet::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "identifier"
                | "shorthand_property_identifier"
                | "shorthand_property_identifier_pattern"
        ) {
            let mut ancestor = node.parent();
            let mut imported = false;
            while let Some(parent) = ancestor {
                if parent.kind() == "import_statement" {
                    imported = true;
                    break;
                }
                ancestor = parent.parent();
            }
            if !imported {
                if let Ok(identifier) = node.utf8_text(code.as_bytes()) {
                    identifiers.insert(identifier.to_string());
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    identifiers
}

fn typescript_virtual_type_imports(code: &str) -> BTreeMap<String, Vec<String>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .expect("Failed to load TypeScript grammar");
    let Some(tree) = parser.parse(code, None) else {
        return BTreeMap::new();
    };
    let runtime_identifiers = typescript_runtime_identifiers(code);
    let mut imports = BTreeMap::<String, Vec<String>>::new();
    let mut cursor = tree.root_node().walk();
    for statement in tree.root_node().named_children(&mut cursor) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Some(source) = statement.child_by_field_name("source") else {
            continue;
        };
        let Ok(source_text) = source.utf8_text(code.as_bytes()) else {
            continue;
        };
        let Some(specifier) = parse_quoted_path(source_text) else {
            continue;
        };
        let Ok(statement_text) = statement.utf8_text(code.as_bytes()) else {
            continue;
        };
        if statement_text.trim_start().starts_with("import type ") {
            continue;
        }

        let mut specifier_nodes = Vec::new();
        let mut stack = vec![statement];
        while let Some(node) = stack.pop() {
            if node.kind() == "import_specifier" {
                specifier_nodes.push(node);
                continue;
            }
            let mut children = node.walk();
            stack.extend(node.named_children(&mut children));
        }
        if specifier_nodes.is_empty() {
            continue;
        }

        for import_specifier in specifier_nodes {
            let Ok(import_text) = import_specifier.utf8_text(code.as_bytes()) else {
                continue;
            };
            if import_text.trim_start().starts_with("type ") {
                continue;
            }
            let mut identifiers = Vec::new();
            let mut children = import_specifier.walk();
            for child in import_specifier.named_children(&mut children) {
                if matches!(child.kind(), "identifier" | "type_identifier") {
                    if let Ok(identifier) = child.utf8_text(code.as_bytes()) {
                        identifiers.push(identifier.to_string());
                    }
                }
            }
            let Some(source_name) = identifiers.first().cloned() else {
                continue;
            };
            let local_name = identifiers.last().unwrap_or(&source_name);
            if !runtime_identifiers.contains(local_name)
                && source_name.chars().enumerate().all(|(index, character)| {
                    character == '_'
                        || character == '$'
                        || (index == 0 && character.is_ascii_alphabetic())
                        || (index > 0 && character.is_ascii_alphanumeric())
                })
            {
                imports
                    .entry(specifier.clone())
                    .or_default()
                    .push(source_name);
            }
        }
    }
    imports.retain(|_, names| !names.is_empty());
    for names in imports.values_mut() {
        names.sort();
        names.dedup();
    }
    imports
}

fn isolated_virtual_type_import_candidates(
    context: &ExecutionContext,
) -> BTreeMap<String, Vec<String>> {
    if !matches!(
        context.target_source.mode,
        crate::types::SourceMode::TypeScript | crate::types::SourceMode::Tsx
    ) {
        return BTreeMap::new();
    }
    let Some(source_file) = context.target_source.source_file.as_deref() else {
        return BTreeMap::new();
    };
    let Ok(code) = std::fs::read_to_string(source_file) else {
        return BTreeMap::new();
    };
    typescript_virtual_type_imports(&code)
}

fn isolated_failure_source_file(
    context: &ExecutionContext,
    process: &ExecutionResult,
) -> Option<std::path::PathBuf> {
    let output = format!("{}\n{}", process.stderr, process.stdout);
    output.lines().find_map(|line| {
        ["/workspace/", "/court-jester/dependencies/"]
            .iter()
            .find_map(|prefix| {
                let start = line.find(prefix)? + prefix.len();
                let relative = line[start..].split(':').next()?;
                let relative = std::path::Path::new(relative);
                if relative.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir | std::path::Component::RootDir
                    )
                }) {
                    return None;
                }
                let source_file = context.workspace_root.join(relative);
                source_file.is_file().then_some(source_file)
            })
    })
}

fn isolated_failure_package_name(
    context: &ExecutionContext,
    process: &ExecutionResult,
) -> Option<String> {
    let source_file = isolated_failure_source_file(context, process)?;
    source_file
        .ancestors()
        .take_while(|directory| directory.starts_with(&context.workspace_root))
        .find_map(|directory| {
            let manifest = std::fs::read_to_string(directory.join("package.json")).ok()?;
            serde_json::from_str::<serde_json::Value>(&manifest)
                .ok()?
                .get("name")?
                .as_str()
                .map(str::to_owned)
        })
}

fn isolated_virtual_type_import_candidate_from_failure(
    context: &ExecutionContext,
    process: &ExecutionResult,
    existing: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> Option<(String, String, Vec<String>)> {
    let output = format!("{}\n{}", process.stderr, process.stdout);
    let source_file = isolated_failure_source_file(context, process)?;
    if !matches!(
        source_file
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("ts" | "tsx")
    ) {
        return None;
    }
    let relative = source_file
        .strip_prefix(&context.workspace_root)
        .ok()?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let code = std::fs::read_to_string(source_file).ok()?;
    let candidates = typescript_virtual_type_imports(&code);
    candidates.into_iter().find_map(|(specifier, names)| {
        let already_present = existing
            .get(&relative)
            .is_some_and(|imports| imports.contains_key(&specifier));
        (!already_present
            && (output.contains(&format!("'{specifier}'"))
                || output.contains(&format!("\"{specifier}\""))))
        .then(|| (relative.clone(), specifier, names))
    })
}
fn resolve_generated_typescript_relative_imports(
    code: &str,
    source_file: Option<&std::path::Path>,
) -> String {
    let Some(source_file) = source_file else {
        return code.to_string();
    };
    let Some(source_dir) = source_file.parent() else {
        return code.to_string();
    };
    let code = code.to_string();
    let mut rewritten = String::with_capacity(code.len());
    let mut cursor = 0;
    while cursor < code.len() {
        let Some(relative_quote) = code[cursor..]
            .find(['"', '\''])
            .map(|offset| cursor + offset)
        else {
            rewritten.push_str(&code[cursor..]);
            break;
        };
        rewritten.push_str(&code[cursor..relative_quote]);
        let quote = code.as_bytes()[relative_quote] as char;
        let Some(end_offset) = code[relative_quote + 1..].find(quote) else {
            rewritten.push_str(&code[relative_quote..]);
            break;
        };
        let end = relative_quote + 1 + end_offset;
        let value = &code[relative_quote + 1..end];
        let line_start = code[..relative_quote]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let line_prefix = code[line_start..relative_quote].trim_start();
        let import_context = line_prefix.contains(" from ")
            || line_prefix.starts_with("import ")
            || line_prefix.contains("import(")
            || line_prefix.contains("require(");
        if import_context {
            if let Some(resolved) = resolved_typescript_import(value, source_dir) {
                rewritten.push(quote);
                rewritten.push_str(&resolved);
                rewritten.push(quote);
                cursor = end + 1;
                continue;
            }
        }
        rewritten.push(quote);
        rewritten.push_str(value);
        rewritten.push(quote);
        cursor = end + 1;
    }
    rewritten
}

pub fn typescript_code_requires_bun_runtime(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("Bun.")
            || trimmed.contains("typeof Bun")
            || trimmed.contains("instanceof Bun")
            || trimmed.contains("from \"bun\"")
            || trimmed.contains("from 'bun'")
            || trimmed.contains("from \"bun:")
            || trimmed.contains("from 'bun:")
            || trimmed.contains("import \"bun\"")
            || trimmed.contains("import 'bun'")
            || trimmed.contains("require(\"bun\")")
            || trimmed.contains("require('bun')")
    })
}

fn dir_declares_bun_package_manager(dir: &std::path::Path) -> bool {
    let package_json = dir.join("package.json");
    std::fs::read_to_string(package_json)
        .map(|text| text.contains("\"packageManager\"") && text.contains("bun@"))
        .unwrap_or(false)
}

pub fn detect_repo_typescript_runner(
    project_dir: Option<&str>,
    source_file: Option<&str>,
) -> Option<String> {
    let mut starts = Vec::new();
    if let Some(dir) = project_dir {
        starts.push(std::path::PathBuf::from(dir));
    }
    if let Some(source_file) = source_file {
        if let Some(parent) = std::path::Path::new(source_file).parent() {
            starts.push(parent.to_path_buf());
        }
    }

    for start in starts {
        let mut dir = start.as_path();
        loop {
            if dir.join("bun.lock").exists()
                || dir.join("bun.lockb").exists()
                || dir_declares_bun_package_manager(dir)
            {
                return Some("bun".into());
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }
    }

    None
}

fn parse_quoted_path(input: &str) -> Option<String> {
    let quote = input.chars().find(|c| *c == '"' || *c == '\'')?;
    let start = input.find(quote)? + 1;
    let end = start + input[start..].find(quote)?;
    Some(input[start..end].to_string())
}

fn resolve_typescript_import_file(
    source_file: &str,
    import_path: &str,
) -> Option<std::path::PathBuf> {
    let source_dir = std::path::Path::new(source_file).parent()?;
    let base = source_dir.join(import_path);

    if base.is_file() {
        return Some(base);
    }
    for ext in [".ts", ".tsx", "/index.ts", "/index.tsx"] {
        let candidate = std::path::PathBuf::from(format!("{}{}", base.display(), ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
fn extract_typescript_named_relative_imports(code: &str) -> Vec<(String, Vec<String>)> {
    let mut imports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("import ") || trimmed.starts_with("import type ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };
        let names = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry = entry.strip_prefix("type ").unwrap_or(entry);
                let export_name = entry
                    .split_once(" as ")
                    .map(|(name, _)| name)
                    .unwrap_or(entry)
                    .trim();
                (!export_name.is_empty()).then(|| export_name.to_string())
            })
            .collect::<Vec<_>>();
        if !names.is_empty() {
            imports.push((path, names));
        }
    }

    imports
}

#[derive(Clone, Debug)]
struct RelativeReexportSpecifier {
    source_name: String,
    exported_name: String,
    type_only: bool,
}

fn extract_typescript_named_relative_reexports(
    code: &str,
) -> Vec<(String, Vec<RelativeReexportSpecifier>)> {
    let mut reexports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("export ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };

        let clause = clause.trim();
        let statement_type_only = clause.starts_with("type ");
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };

        let specifiers = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry_type_only = statement_type_only || entry.starts_with("type ");
                let entry = entry.strip_prefix("type ").unwrap_or(entry).trim();
                let (source_name, exported_name) = entry
                    .split_once(" as ")
                    .map(|(source_name, exported_name)| (source_name.trim(), exported_name.trim()))
                    .unwrap_or((entry, entry));
                if source_name.is_empty() || exported_name.is_empty() {
                    return None;
                }
                Some(RelativeReexportSpecifier {
                    source_name: source_name.to_string(),
                    exported_name: exported_name.to_string(),
                    type_only: entry_type_only,
                })
            })
            .collect::<Vec<_>>();
        if !specifiers.is_empty() {
            reexports.push((path, specifiers));
        }
    }

    reexports
}

fn source_defines_runtime_export(code: &str, name: &str) -> bool {
    [
        "export const ",
        "export let ",
        "export var ",
        "export function ",
        "export class ",
        "export enum ",
    ]
    .iter()
    .any(|prefix| {
        code.lines().any(|line| {
            let Some(remainder) = line.trim_start().strip_prefix(prefix) else {
                return false;
            };
            remainder.strip_prefix(name).is_some_and(|suffix| {
                suffix
                    .chars()
                    .next()
                    .is_none_or(|character| character != '_' && !character.is_ascii_alphanumeric())
            })
        })
    })
}

fn resolve_typescript_runtime_reexport(
    source_file: &std::path::Path,
    name: &str,
    visited: &mut HashSet<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let source_file = std::fs::canonicalize(source_file).ok()?;
    if !visited.insert(source_file.clone()) {
        return None;
    }
    let code = std::fs::read_to_string(&source_file).ok()?;
    if source_defines_runtime_export(&code, name) {
        return Some(source_file);
    }
    let source_text = source_file.to_str()?;

    let mut named_matches = Vec::new();
    for (import_path, specifiers) in extract_typescript_named_relative_reexports(&code) {
        for specifier in specifiers {
            if specifier.exported_name != name || specifier.type_only {
                continue;
            }
            let target = resolve_typescript_import_file(source_text, &import_path)?;
            let mut branch_visited = visited.clone();
            let resolved = resolve_typescript_runtime_reexport(
                &target,
                &specifier.source_name,
                &mut branch_visited,
            )?;
            if !named_matches.contains(&resolved) {
                named_matches.push(resolved);
            }
        }
    }
    if named_matches.len() == 1 {
        return named_matches.pop();
    }
    if !named_matches.is_empty() {
        return None;
    }

    let mut wildcard_matches = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim().trim_end_matches(';');
        let Some(from_clause) = trimmed.strip_prefix("export * from ") else {
            continue;
        };
        let Some(import_path) = parse_quoted_path(from_clause) else {
            continue;
        };
        let Some(target) = resolve_typescript_import_file(source_text, &import_path) else {
            continue;
        };
        let mut branch_visited = visited.clone();
        if let Some(resolved) =
            resolve_typescript_runtime_reexport(&target, name, &mut branch_visited)
        {
            if !wildcard_matches.contains(&resolved) {
                wildcard_matches.push(resolved);
            }
        }
    }
    (wildcard_matches.len() == 1).then(|| wildcard_matches.remove(0))
}

fn relative_typescript_specifier(
    source_dir: &std::path::Path,
    target: &std::path::Path,
) -> Option<String> {
    let source_dir = std::fs::canonicalize(source_dir).ok()?;
    let target = std::fs::canonicalize(target).ok()?;
    let source_components = source_dir.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let shared = source_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = std::path::PathBuf::new();
    for _ in shared..source_components.len() {
        relative.push("..");
    }
    for component in &target_components[shared..] {
        relative.push(component.as_os_str());
    }
    let relative = relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    Some(if relative.starts_with('.') {
        relative
    } else {
        format!("./{relative}")
    })
}

fn isolated_typescript_barrel_redirects(context: &ExecutionContext) -> BTreeMap<String, String> {
    let Some(source_file) = context.target_source.source_file.as_deref() else {
        return BTreeMap::new();
    };
    let Ok(code) = std::fs::read_to_string(source_file) else {
        return BTreeMap::new();
    };
    let Some(source_dir) = source_file.parent() else {
        return BTreeMap::new();
    };
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .is_err()
    {
        return BTreeMap::new();
    }
    let Some(tree) = parser.parse(&code, None) else {
        return BTreeMap::new();
    };
    let mut redirects = BTreeMap::new();
    let mut cursor = tree.root_node().walk();
    for statement in tree.root_node().named_children(&mut cursor) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Ok(statement_text) = statement.utf8_text(code.as_bytes()) else {
            continue;
        };
        let Some(import_body) = statement_text.trim().strip_prefix("import ") else {
            continue;
        };
        if import_body.starts_with("type ") {
            continue;
        }
        let Some((clause, _)) = import_body.split_once(" from ") else {
            continue;
        };
        let clause = clause.trim();
        if !clause.starts_with('{') || !clause.ends_with('}') {
            continue;
        }
        let Some(source) = statement.child_by_field_name("source") else {
            continue;
        };
        let Ok(source_text) = source.utf8_text(code.as_bytes()) else {
            continue;
        };
        let Some(specifier) = parse_quoted_path(source_text) else {
            continue;
        };
        if !(specifier.starts_with("./") || specifier.starts_with("../"))
            || code.match_indices(&specifier).count() != 1
        {
            continue;
        }
        let Some(barrel) =
            resolve_typescript_import_file(source_file.to_str().unwrap_or_default(), &specifier)
        else {
            continue;
        };
        let mut names = Vec::new();
        let mut stack = vec![statement];
        while let Some(node) = stack.pop() {
            if node.kind() == "import_specifier" {
                let Ok(import_text) = node.utf8_text(code.as_bytes()) else {
                    continue;
                };
                if import_text.trim_start().starts_with("type ") {
                    continue;
                }
                let mut identifiers = Vec::new();
                let mut children = node.walk();
                for child in node.named_children(&mut children) {
                    if matches!(child.kind(), "identifier" | "type_identifier") {
                        if let Ok(identifier) = child.utf8_text(code.as_bytes()) {
                            identifiers.push(identifier.to_string());
                        }
                    }
                }
                if let Some(source_name) = identifiers.first() {
                    names.push(source_name.clone());
                }
                continue;
            }
            let mut children = node.walk();
            stack.extend(node.named_children(&mut children));
        }
        if names.is_empty() {
            continue;
        }
        let leaves = names
            .iter()
            .filter_map(|name| {
                resolve_typescript_runtime_reexport(&barrel, name, &mut HashSet::new())
            })
            .collect::<Vec<_>>();
        if leaves.len() != names.len() || leaves.iter().any(|leaf| leaf != &leaves[0]) {
            continue;
        }
        let Ok(barrel) = std::fs::canonicalize(&barrel) else {
            continue;
        };
        if leaves[0] == barrel {
            continue;
        }
        if let Some(redirect) = relative_typescript_specifier(source_dir, &leaves[0]) {
            redirects.insert(specifier, redirect);
        }
    }
    redirects
}

fn source_key(source_file: &str) -> String {
    std::fs::canonicalize(source_file)
        .unwrap_or_else(|_| std::path::PathBuf::from(source_file))
        .to_string_lossy()
        .to_string()
}

fn target_exports_name_only_as_type(
    code: &str,
    source_file: &str,
    name: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let visit_key = format!("{}::{name}", source_key(source_file));
    if !visited.insert(visit_key) {
        return false;
    }

    let type_alias_prefix = format!("export type {name}");
    let interface_prefix = format!("export interface {name}");

    if code.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(&type_alias_prefix) || trimmed.starts_with(&interface_prefix)
    }) {
        return true;
    }

    extract_typescript_named_relative_reexports(code)
        .into_iter()
        .any(|(import_path, specifiers)| {
            specifiers.into_iter().any(|specifier| {
                if specifier.exported_name != name {
                    return false;
                }
                if specifier.type_only {
                    return true;
                }
                let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                    Some(path) => path,
                    None => return false,
                };
                let imported_code = match std::fs::read_to_string(&resolved) {
                    Ok(code) => code,
                    Err(_) => return false,
                };
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    &specifier.source_name,
                    visited,
                )
            })
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports_inner(
    code: &str,
    source_file: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let source_key = source_key(source_file);
    if !visited.insert(source_key) {
        return false;
    }

    extract_typescript_named_relative_imports(code)
        .into_iter()
        .any(|(import_path, names)| {
            let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                Some(path) => path,
                None => return false,
            };
            let imported_code = match std::fs::read_to_string(&resolved) {
                Ok(code) => code,
                Err(_) => return false,
            };
            let mut export_visited = std::collections::HashSet::new();
            names.iter().any(|name| {
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    name,
                    &mut export_visited,
                )
            }) || has_typescript_type_only_relative_imports_inner(
                &imported_code,
                resolved.to_str().unwrap_or_default(),
                visited,
            )
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports(code: &str, source_file: Option<&str>) -> bool {
    let source_file = match source_file {
        Some(source_file) => source_file,
        None => return false,
    };

    has_typescript_type_only_relative_imports_inner(
        code,
        source_file,
        &mut std::collections::HashSet::new(),
    )
}

/// Remove only imports proven to contain type-only exports. Node 24's native
/// type stripping handles syntax, but it must not evaluate a type-only binding
/// from a runtime module. Unknown bindings are left untouched.
fn strip_typescript_type_only_imports(code: &str, source_file: Option<&str>) -> String {
    let Some(source_file) = source_file else {
        return code.to_string();
    };
    let mut out = String::with_capacity(code.len());
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import type ") {
            continue;
        }
        if trimmed.starts_with("import {") && trimmed.contains("} from") {
            let Some((left, right)) = line.split_once("} from") else {
                out.push_str(line);
                out.push('\n');
                continue;
            };
            let names = left.split_once('{').map(|(_, names)| names).unwrap_or("");
            let import_path = parse_quoted_path(right).unwrap_or_default();
            if !(import_path.starts_with("./") || import_path.starts_with("../")) {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let resolved = resolve_typescript_import_file(source_file, &import_path);
            let kept = names
                .split(',')
                .filter_map(|entry| {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        return None;
                    }
                    let source_name = entry
                        .split_once(" as ")
                        .map(|(name, _)| name.trim())
                        .unwrap_or(entry);
                    let is_type = resolved
                        .as_ref()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .map(|text| {
                            let mut visited = std::collections::HashSet::new();
                            target_exports_name_only_as_type(
                                &text,
                                resolved.as_ref().and_then(|p| p.to_str()).unwrap_or(""),
                                source_name,
                                &mut visited,
                            )
                        })
                        .unwrap_or(false);
                    (!is_type).then(|| entry.to_string())
                })
                .collect::<Vec<_>>();
            if kept.is_empty() {
                continue;
            }
            let prefix = &line[..line.find('{').unwrap_or(0)];
            out.push_str(prefix);
            out.push_str("{ ");
            out.push_str(&kept.join(", "));
            out.push_str(" } from");
            out.push_str(right);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

fn source_matches_disk(code: &str, source_file: Option<&str>) -> Option<std::path::PathBuf> {
    let source_file = source_file?;
    let disk_code = std::fs::read_to_string(source_file).ok()?;
    if disk_code != code {
        return None;
    }
    Some(
        std::fs::canonicalize(source_file)
            .unwrap_or_else(|_| std::path::PathBuf::from(source_file)),
    )
}

fn materialization_entry_is_ignored(
    source_root: &std::path::Path,
    source: &std::path::Path,
    name: &std::ffi::OsStr,
) -> bool {
    if matches!(
        name.to_str(),
        Some(
            ".git"
                | ".hg"
                | ".svn"
                | "target"
                | "node_modules"
                | "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".venv"
                | "venv"
                | ".ruff_cache"
                | "dist"
                | "build"
        )
    ) {
        return true;
    }
    name == std::ffi::OsStr::new("results")
        && source.strip_prefix(source_root).ok() == Some(std::path::Path::new("bench"))
}

fn copy_materialization_tree(
    source: &std::path::Path,
    destination: &std::path::Path,
    trusted_source_root: Option<&std::path::Path>,
) -> std::io::Result<()> {
    let source_root = std::fs::canonicalize(source)?;
    let trusted_source_root = trusted_source_root.map(std::fs::canonicalize).transpose()?;
    let mut active_directories = HashSet::new();
    copy_materialization_tree_inner(
        source,
        destination,
        &source_root,
        trusted_source_root.as_deref(),
        &mut active_directories,
    )
}

fn materialization_path_is_allowed(
    path: &std::path::Path,
    source_root: &std::path::Path,
    trusted_source_root: Option<&std::path::Path>,
) -> bool {
    path.starts_with(source_root)
        || trusted_source_root.is_some_and(|trusted| path.starts_with(trusted))
}

fn copy_materialization_tree_inner(
    source: &std::path::Path,
    destination: &std::path::Path,
    source_root: &std::path::Path,
    trusted_source_root: Option<&std::path::Path>,
    active_directories: &mut HashSet<std::path::PathBuf>,
) -> std::io::Result<()> {
    let canonical_source = std::fs::canonicalize(source)?;
    if !materialization_path_is_allowed(&canonical_source, source_root, trusted_source_root)
        || !active_directories.insert(canonical_source.clone())
    {
        return Ok(());
    }
    let result = (|| {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            if materialization_entry_is_ignored(source_root, &canonical_source, &entry.file_name())
            {
                continue;
            }
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                let resolved = match std::fs::canonicalize(&source_path) {
                    Ok(resolved) => resolved,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                if !materialization_path_is_allowed(&resolved, source_root, trusted_source_root) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "materialization symlink escapes source root: {}",
                            source_path.display()
                        ),
                    ));
                }
                if resolved.is_dir() {
                    copy_materialization_tree_inner(
                        &resolved,
                        &destination_path,
                        source_root,
                        trusted_source_root,
                        active_directories,
                    )?;
                } else if resolved.is_file() {
                    std::fs::copy(resolved, destination_path)?;
                }
            } else if file_type.is_dir() {
                copy_materialization_tree_inner(
                    &source_path,
                    &destination_path,
                    source_root,
                    trusted_source_root,
                    active_directories,
                )?;
            } else if file_type.is_file() {
                std::fs::copy(&source_path, destination_path)?;
            }
        }
        Ok(())
    })();
    active_directories.remove(&canonical_source);
    result
}
pub fn runtime_tempdir(
    _runtime_profile: crate::types::RuntimeProfile,
) -> std::io::Result<tempfile::TempDir> {
    #[cfg(target_os = "macos")]
    if _runtime_profile == crate::types::RuntimeProfile::Isolated {
        if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
            let parent = std::path::PathBuf::from(home).join("Library/Caches/court-jester/runtime");
            if std::fs::create_dir_all(&parent).is_ok() {
                if let Ok(directory) = tempfile::Builder::new()
                    .prefix("court-jester-")
                    .tempdir_in(parent)
                {
                    return Ok(directory);
                }
            }
        }
    }
    tempfile::tempdir()
}

fn standalone_runtime_tempdir(
    runtime_profile: crate::types::RuntimeProfile,
) -> std::io::Result<tempfile::TempDir> {
    runtime_tempdir(runtime_profile)
}

fn docker_image_for_harness<'a>(
    configured_image: &'a str,
    runtime: &crate::types::HarnessRuntime,
) -> &'a str {
    if matches!(
        runtime,
        crate::types::HarnessRuntime::BunScript | crate::types::HarnessRuntime::BunTest
    ) && configured_image == crate::types::DEFAULT_TYPESCRIPT_DOCKER_IMAGE
    {
        crate::types::DEFAULT_BUN_DOCKER_IMAGE
    } else {
        configured_image
    }
}

fn docker_runtime_user(_has_node_dependency_bind: bool) -> String {
    #[cfg(target_os = "macos")]
    if _has_node_dependency_bind {
        // Docker Desktop can transiently expose read-only pnpm store files with
        // host-only modes. Root may read the bind but cannot write it.
        return "0:0".to_string();
    }
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    format!("{uid}:{gid}")
}

struct NodePackageResolver {
    _directory: tempfile::TempDir,
    loader: std::path::PathBuf,
}

fn create_node_package_resolver(
    runtime_profile: crate::types::RuntimeProfile,
) -> Result<NodePackageResolver, String> {
    create_node_package_resolver_with_virtual_type_imports(runtime_profile, None)
}

fn create_node_package_resolver_with_virtual_type_imports(
    runtime_profile: crate::types::RuntimeProfile,
    virtual_type_imports: Option<&str>,
) -> Result<NodePackageResolver, String> {
    let directory = runtime_tempdir(runtime_profile)
        .map_err(|error| format!("failed to create Node package resolver: {error}"))?;
    let loader = directory.path().join("package-resolver.mjs");
    let loader_source = r##"
import { readFileSync, realpathSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const mode = process.env.COURT_JESTER_NODE_RESOLUTION_MODE || "";
const overlayRoot = realPathOrSelf(process.env.COURT_JESTER_NODE_OVERLAY_ROOT || "");
const sourceRoot = realPathOrSelf(process.env.COURT_JESTER_NODE_SOURCE_ROOT || "");
const targetRoot = realPathOrSelf(process.env.COURT_JESTER_NODE_TARGET_ROOT || "");
const overlayTargetRoot = realPathOrSelf(
  process.env.COURT_JESTER_NODE_OVERLAY_TARGET_ROOT || "",
);
const generatedArtifact = realPathOrSelf(
  process.env.COURT_JESTER_NODE_GENERATED_ARTIFACT || "",
);
const loaderConfiguration = __COURT_JESTER_VIRTUAL_TYPE_IMPORTS__;
const virtualTypeImports = loaderConfiguration.removals || {};
const importRedirects = loaderConfiguration.redirects || {};
const targetSelfReferenceName = readSelfReferenceName(overlayTargetRoot);

function realPathOrSelf(value) {
  if (!value) return value;
  try {
    return realpathSync(value);
  } catch {
    return value;
  }
}

function readSelfReferenceName(root) {
  try {
    const manifest = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8"));
    return Object.hasOwn(manifest, "exports") ? manifest.name : undefined;
  } catch {
    return undefined;
  }
}

function isPackageSpecifier(specifier) {
  return !specifier.startsWith(".")
    && !specifier.startsWith("/")
    && !specifier.startsWith("#")
    && !/^[A-Za-z][A-Za-z0-9+.-]*:/.test(specifier);
}

function requestedPackageName(specifier) {
  const segments = specifier.split("/");
  return specifier.startsWith("@") ? segments.slice(0, 2).join("/") : segments[0];
}

function containsPath(root, candidate) {
  if (!root) return false;
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`)
    && relative !== ".."
    && !path.isAbsolute(relative));
}
const unsupportedAliasPrefix =
  "court-jester unsupported TypeScript path alias configuration:";
let cachedPathConfiguration;

function parseJsonConfig(configPath) {
  const source = readFileSync(configPath, "utf8");
  let withoutComments = "";
  let quote = "";
  let escaped = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (quote) {
      withoutComments += character;
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === quote) quote = "";
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      withoutComments += character;
      continue;
    }
    if (character === "/" && next === "/") {
      while (index < source.length && source[index] !== "\n") index += 1;
      withoutComments += "\n";
      continue;
    }
    if (character === "/" && next === "*") {
      index += 2;
      while (index < source.length && !(source[index] === "*" && source[index + 1] === "/")) {
        if (source[index] === "\n") withoutComments += "\n";
        index += 1;
      }
      index += 1;
      continue;
    }
    if (index !== 0 || character !== "\uFEFF") withoutComments += character;
  }

  let output = "";
  quote = "";
  escaped = false;
  for (let index = 0; index < withoutComments.length; index += 1) {
    const character = withoutComments[index];
    if (quote) {
      output += character;
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === quote) quote = "";
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      output += character;
      continue;
    }
    if (character === ",") {
      let next = index + 1;
      while (next < withoutComments.length && /\s/.test(withoutComments[next])) next += 1;
      if (withoutComments[next] === "}" || withoutComments[next] === "]") continue;
    }
    output += character;
  }
  return JSON.parse(output);
}

function unsupportedAlias(message) {
  throw new Error(`${unsupportedAliasPrefix} ${message}`);
}

function resolveConfigExtends(configPath, value) {
  if (typeof value !== "string" || !value.startsWith(".")) {
    unsupportedAlias(`only project-relative tsconfig extends are supported (${configPath})`);
  }
  let candidate = path.resolve(path.dirname(configPath), value);
  if (!path.extname(candidate)) candidate += ".json";
  const resolved = realPathOrSelf(candidate);
  if (!containsPath(overlayRoot, resolved)) {
    unsupportedAlias(`tsconfig extends escapes the project mirror (${value})`);
  }
  return resolved;
}

function readPathConfiguration(configPath, visited = new Set()) {
  const resolvedConfig = realPathOrSelf(configPath);
  if (!containsPath(overlayRoot, resolvedConfig)) {
    unsupportedAlias(`tsconfig escapes the project mirror (${configPath})`);
  }
  if (visited.has(resolvedConfig)) {
    unsupportedAlias(`cyclic tsconfig extends (${configPath})`);
  }
  visited.add(resolvedConfig);
  let config;
  try {
    config = parseJsonConfig(resolvedConfig);
  } catch (error) {
    unsupportedAlias(`cannot parse ${configPath}: ${error.message}`);
  }
  let aliases = new Map();
  let baseUrl;
  if (config.extends !== undefined) {
    const inherited = readPathConfiguration(
      resolveConfigExtends(resolvedConfig, config.extends),
      visited,
    );
    aliases = new Map(inherited.aliases);
    baseUrl = inherited.baseUrl;
  }
  const compilerOptions = config.compilerOptions || {};
  if (compilerOptions.paths !== undefined
      && (typeof compilerOptions.paths !== "object" || Array.isArray(compilerOptions.paths))) {
    unsupportedAlias(`compilerOptions.paths must be an object (${configPath})`);
  }
  if (compilerOptions.baseUrl !== undefined) {
    if (typeof compilerOptions.baseUrl !== "string") {
      unsupportedAlias(`compilerOptions.baseUrl must be a string (${configPath})`);
    }
    baseUrl = path.resolve(path.dirname(resolvedConfig), compilerOptions.baseUrl);
    if (!containsPath(overlayRoot, baseUrl)) {
      unsupportedAlias(`baseUrl escapes the project mirror (${configPath})`);
    }
  }
  const mappingBase = baseUrl || path.dirname(resolvedConfig);
  for (const [pattern, targets] of Object.entries(compilerOptions.paths || {})) {
    if ((pattern.match(/\*/g) || []).length > 1
        || !Array.isArray(targets)
        || targets.length === 0
        || targets.some((target) => typeof target !== "string"
          || (target.match(/\*/g) || []).length > 1)) {
      unsupportedAlias(`unsupported path mapping '${pattern}' (${configPath})`);
    }
    for (const target of targets) {
      const staticPrefix = path.resolve(mappingBase, target.split("*", 1)[0]);
      if (!containsPath(overlayRoot, staticPrefix)) {
        unsupportedAlias(`path mapping '${pattern}' escapes the project mirror`);
      }
    }
    aliases.set(pattern, { base: mappingBase, targets });
  }
  visited.delete(resolvedConfig);
  return { aliases, baseUrl };
}

function configuredPathResolution() {
  if (cachedPathConfiguration !== undefined) return cachedPathConfiguration;
  let directory = overlayTargetRoot;
  while (directory && containsPath(overlayRoot, directory)) {
    for (const name of ["tsconfig.json", "jsconfig.json"]) {
      const candidate = path.join(directory, name);
      let isFile = false;
      try {
        isFile = statSync(candidate).isFile();
      } catch {}
      if (isFile) {
        cachedPathConfiguration = readPathConfiguration(candidate);
        return cachedPathConfiguration;
      }
    }
    if (directory === overlayRoot) break;
    directory = path.dirname(directory);
  }
  cachedPathConfiguration = { aliases: new Map(), baseUrl: undefined };
  return cachedPathConfiguration;
}

function matchPathAlias(pattern, specifier) {
  const star = pattern.indexOf("*");
  if (star < 0) return pattern === specifier ? "" : undefined;
  const prefix = pattern.slice(0, star);
  const suffix = pattern.slice(star + 1);
  if (!specifier.startsWith(prefix) || !specifier.endsWith(suffix)) return undefined;
  return specifier.slice(prefix.length, specifier.length - suffix.length);
}

function resolveAliasFile(candidate, root = overlayRoot) {
  const candidates = [
    candidate,
    ...[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json"].map(
      (extension) => `${candidate}${extension}`,
    ),
    ...["index.ts", "index.tsx", "index.js", "index.jsx", "index.mjs", "index.cjs"].map(
      (name) => path.join(candidate, name),
    ),
  ];
  for (const possible of candidates) {
    if (!containsPath(root, possible)) {
      unsupportedAlias("resolved project path escapes the configured root");
    }
    try {
      const resolved = realpathSync(possible);
      if (!containsPath(root, resolved)) {
        unsupportedAlias("resolved project path escapes the configured root");
      }
      if (statSync(resolved).isFile()) return resolved;
    } catch {}
  }
  return undefined;
}

function resolvePathAlias(specifier) {
  const configuration = configuredPathResolution();
  let selected;
  for (const [pattern, entry] of configuration.aliases) {
    const wildcard = matchPathAlias(pattern, specifier);
    if (wildcard === undefined) continue;
    const star = pattern.indexOf("*");
    if (star < 0) {
      selected = { entry, wildcard };
      break;
    }
    if (!selected || star > selected.prefixLength) {
      selected = { entry, wildcard, prefixLength: star };
    }
  }
  if (selected) {
    for (const target of selected.entry.targets) {
      const relative = target.includes("*")
        ? target.replace("*", selected.wildcard)
        : target;
      const resolved = resolveAliasFile(path.resolve(selected.entry.base, relative));
      if (resolved) return resolved;
    }
  }
  return configuration.baseUrl
    ? resolveAliasFile(path.resolve(configuration.baseUrl, specifier))
    : undefined;
}
function isExplicitPathAliasSpecifier(specifier) {
  return specifier.startsWith("~/")
    || specifier.startsWith("~~/")
    || specifier.startsWith("@/")
    || specifier.startsWith("@@/")
    || specifier.startsWith("#");
}

function fileResolution(file) {
  const resolution = { url: pathToFileURL(file).href, shortCircuit: true };
  if (path.extname(file) === ".json") {
    resolution.format = "json";
    resolution.importAttributes = { type: "json" };
  }
  return resolution;
}
function aliasResolution(specifier) {
  const alias = resolvePathAlias(specifier);
  return alias ? fileResolution(alias) : undefined;
}

function relativeResolution(specifier, parentPath) {
  const root = containsPath(overlayRoot, parentPath)
    ? overlayRoot
    : containsPath(sourceRoot, parentPath)
      ? sourceRoot
      : undefined;
  if (!root) return undefined;
  let relative = resolveAliasFile(
    path.resolve(path.dirname(parentPath), specifier),
    root,
  );
  if (!relative && root === overlayRoot && sourceRoot) {
    const sourceParent = path.join(sourceRoot, path.relative(overlayRoot, parentPath));
    relative = resolveAliasFile(
      path.resolve(path.dirname(sourceParent), specifier),
      sourceRoot,
    );
  }
  return relative ? fileResolution(relative) : undefined;
}

function packageSubpathResolution(specifier, parentPath) {
  const packageName = requestedPackageName(specifier);
  if (specifier === packageName) return undefined;
  const root = containsPath(sourceRoot, parentPath)
    ? sourceRoot
    : containsPath(overlayRoot, parentPath)
      ? overlayRoot
      : undefined;
  if (!root) return undefined;
  let directory = path.dirname(parentPath);
  while (containsPath(root, directory)) {
    const resolved = resolveAliasFile(
      path.join(directory, "node_modules", specifier),
      root,
    );
    if (resolved) return fileResolution(resolved);
    if (directory === root) break;
    directory = path.dirname(directory);
  }
  return undefined;
}

function isInsideNodeModules(candidate) {
  return candidate.split(path.sep).includes("node_modules");
}

function overlayParentPath(context) {
  if (!overlayRoot || !context.parentURL?.startsWith("file:")) return undefined;
  try {
    const parentPath = realPathOrSelf(fileURLToPath(context.parentURL));
    return containsPath(overlayRoot, parentPath) ? parentPath : undefined;
  } catch {
    return undefined;
  }
}

function sourceProjectParentPath(context) {
  if (!sourceRoot || !context.parentURL?.startsWith("file:")) return undefined;
  try {
    const parentPath = realPathOrSelf(fileURLToPath(context.parentURL));
    return containsPath(sourceRoot, parentPath) ? parentPath : undefined;
  } catch {
    return undefined;
  }
}

function sourceParentURL(parentPath) {
  const relative = path.relative(overlayRoot, parentPath);
  return pathToFileURL(path.join(sourceRoot, relative)).href;
}

function projectRelativePath(parentPath) {
  if (!parentPath) return undefined;
  const root = containsPath(overlayRoot, parentPath)
    ? overlayRoot
    : containsPath(sourceRoot, parentPath)
      ? sourceRoot
      : undefined;
  return root ? path.relative(root, parentPath).split(path.sep).join("/") : undefined;
}


function packageWasNotFound(error, packageName) {
  if (error?.code !== "ERR_MODULE_NOT_FOUND") return false;
  return error.message?.includes(`Cannot find package '${packageName}'`) === true
    || error.message?.includes(`Cannot find package "${packageName}"`) === true;
}

export async function resolve(specifier, context, nextResolve) {
  const parentPath = overlayParentPath(context);
  const projectParentPath = parentPath || sourceProjectParentPath(context);
  const importer = projectRelativePath(projectParentPath);
  const redirect = importer && importRedirects[importer]?.[specifier];
  if (redirect) {
    const redirected = relativeResolution(redirect, projectParentPath);
    if (redirected) return redirected;
  }
  if (projectParentPath && specifier.startsWith(".")) {
    const relative = relativeResolution(specifier, projectParentPath);
    if (relative) return relative;
  }
  if (projectParentPath && isExplicitPathAliasSpecifier(specifier)) {
    const alias = aliasResolution(specifier);
    if (alias) return alias;
  }
  if (!isPackageSpecifier(specifier)) {
    return nextResolve(specifier, context);
  }
  if (!parentPath) {
    try {
      return await nextResolve(specifier, context);
    } catch (error) {
      const packageSubpath = projectParentPath
        && error?.code === "ERR_MODULE_NOT_FOUND"
        ? packageSubpathResolution(specifier, projectParentPath)
        : undefined;
      if (packageSubpath) return packageSubpath;
      throw error;
    }
  }

  const packageName = requestedPackageName(specifier);
  if (mode === "existing") {
    try {
      return await nextResolve(specifier, context);
    } catch (error) {
      if (isInsideNodeModules(parentPath) || !packageWasNotFound(error, packageName)) {
        throw error;
      }
      const alias = aliasResolution(specifier);
      if (alias) return alias;
      if (parentPath === generatedArtifact) {
        const mappedParentURL = sourceParentURL(parentPath);
        if (mappedParentURL !== context.parentURL) {
          try {
            return await nextResolve(specifier, {
              ...context,
              parentURL: mappedParentURL,
            });
          } catch (mappedError) {
            if (!packageWasNotFound(mappedError, packageName)) throw mappedError;
          }
        }
        return nextResolve(specifier, {
          ...context,
          parentURL: pathToFileURL(path.join(targetRoot, "package.json")).href,
        });
      }
      return nextResolve(specifier, {
        ...context,
        parentURL: sourceParentURL(parentPath),
      });
    }
  }

  const originatesFromTarget = parentPath === generatedArtifact
    || containsPath(overlayTargetRoot, parentPath);
  if (!originatesFromTarget) {
    return nextResolve(specifier, {
      ...context,
      parentURL: sourceParentURL(parentPath),
    });
  }
  const resolutionRoot = packageName === targetSelfReferenceName
    ? overlayTargetRoot
    : targetRoot;
  try {
    return await nextResolve(specifier, {
      ...context,
      parentURL: pathToFileURL(path.join(resolutionRoot, "package.json")).href,
    });
  } catch (error) {
    if (!packageWasNotFound(error, packageName)) throw error;
    const alias = aliasResolution(specifier);
    if (alias) return alias;
    throw error;
  }
}

function projectJsonModule(url, context) {
  if (!url.startsWith("file:")
      || context.importAttributes?.type === "json"
      || (!overlayRoot && !sourceRoot)) {
    return undefined;
  }
  let filename;
  try {
    filename = realPathOrSelf(fileURLToPath(url));
  } catch {
    return undefined;
  }
  if (path.extname(filename) !== ".json"
      || !(containsPath(overlayRoot, filename) || containsPath(sourceRoot, filename))) {
    return undefined;
  }
  const value = JSON.parse(readFileSync(filename, "utf8"));
  return {
    format: "module",
    source: `export default ${JSON.stringify(value)};\n`,
    shortCircuit: true,
  };
}

const dependencyReadRetryDelays = [10, 25, 50, 100];

function isProjectDependencyURL(url) {
  if (!url.startsWith("file:") || !sourceRoot) return false;
  try {
    const filename = realPathOrSelf(fileURLToPath(url));
    return containsPath(sourceRoot, filename)
      && filename.split(path.sep).includes("node_modules");
  } catch {
    return false;
  }
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function rewriteTypeOnlyImports(url, loaded) {
  if (!url.startsWith("file:") || loaded?.source == null) return loaded;
  let importer;
  try {
    importer = projectRelativePath(realPathOrSelf(fileURLToPath(url)));
  } catch {
    return loaded;
  }
  const removals = importer && virtualTypeImports[importer];
  if (!removals) return loaded;
  let source = typeof loaded.source === "string"
    ? loaded.source
    : Buffer.from(loaded.source).toString("utf8");
  for (const [specifier, names] of Object.entries(removals)) {
    const pattern = new RegExp(
      `import\\s*\\{([^}]*)\\}\\s*from\\s*(["'])${escapeRegExp(specifier)}\\2\\s*;?`,
      "g",
    );
    source = source.replace(pattern, (statement, bindings) => {
      const removed = new Set(names);
      const kept = bindings.split(",").filter((binding) => {
        const sourceName = binding
          .trim()
          .replace(/^type\\s+/, "")
          .split(/\\s+as\\s+/)[0];
        return sourceName && !removed.has(sourceName);
      });
      if (kept.length === 0) {
        return statement.replace(/[^\n]/g, " ");
      }
      return statement.replace(bindings, ` ${kept.map((entry) => entry.trim()).join(", ")} `);
    });
  }
  return { ...loaded, source };
}

export async function load(url, context, nextLoad) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      const jsonModule = projectJsonModule(url, context);
      const loaded = jsonModule || await nextLoad(url, context);
      return rewriteTypeOnlyImports(url, loaded);
    } catch (error) {
      if (error?.code !== "EACCES"
          || !isProjectDependencyURL(url)
          || attempt === dependencyReadRetryDelays.length) {
        throw error;
      }
      await new Promise((resolveRetry) => {
        setTimeout(resolveRetry, dependencyReadRetryDelays[attempt]);
      });
    }
  }
}
"##
    .replace(
        "__COURT_JESTER_VIRTUAL_TYPE_IMPORTS__",
        virtual_type_imports.unwrap_or(r#"{"removals":{},"redirects":{}}"#),
    );
    std::fs::write(&loader, loader_source)
        .map_err(|error| format!("failed to write Node package resolver: {error}"))?;
    Ok(NodePackageResolver {
        _directory: directory,
        loader,
    })
}

fn node_dependency_paths(dependency_roots: &[std::path::PathBuf]) -> Vec<String> {
    dependency_roots
        .iter()
        .map(|root| root.join("node_modules"))
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn node_package_resolver_roots(
    target_package_root: &std::path::Path,
    dependency_roots: &[std::path::PathBuf],
) -> Vec<String> {
    let mut roots = Vec::with_capacity(dependency_roots.len() + 1);
    for root in std::iter::once(target_package_root)
        .chain(dependency_roots.iter().map(std::path::PathBuf::as_path))
    {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
        .into_iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect()
}

struct NetworkGuard {
    _directory: tempfile::TempDir,
    python_sitecustomize: std::path::PathBuf,
    node_preload: std::path::PathBuf,
    instrumentation_preload: std::path::PathBuf,
    instrumented_source: Option<std::path::PathBuf>,
    vitest_coordinator: std::path::PathBuf,
    portable_vitest_coordinator: std::path::PathBuf,
    typescript_loader: std::path::PathBuf,
}

fn create_network_guard(
    runtime_profile: crate::types::RuntimeProfile,
    instrumented_source: Option<&str>,
) -> Result<NetworkGuard, String> {
    let directory = runtime_tempdir(runtime_profile)
        .map_err(|error| format!("failed to create network guard: {error}"))?;
    let python_sitecustomize = directory.path().join("sitecustomize.py");
    let node_preload = directory.path().join("network-guard.cjs");
    let instrumentation_preload = directory.path().join("instrumentation-preload.cjs");
    let instrumented_source_path = instrumented_source
        .map(|source| {
            let path = directory.path().join("instrumented-target.ts");
            std::fs::write(&path, source)
                .map(|_| path)
                .map_err(|error| format!("failed to write instrumentation source: {error}"))
        })
        .transpose()?;
    let vitest_coordinator = directory.path().join("vitest-coordinator.mjs");
    let portable_vitest_coordinator = directory.path().join("portable-vitest-coordinator.mjs");
    let typescript_loader = directory.path().join("typescript-loader.mjs");
    std::fs::write(
        &python_sitecustomize,
        r#"
import asyncio
import os
import socket
import subprocess

_NETWORK_MESSAGE = "court-jester network access denied"
_PROCESS_MESSAGE = "court-jester process spawn denied"

def _deny_network(*_args, **_kwargs):
    raise PermissionError(_NETWORK_MESSAGE)

def _deny_process(*_args, **_kwargs):
    raise PermissionError(_PROCESS_MESSAGE)

for _name in (
    "connect", "connect_ex", "send", "sendall", "sendto", "sendmsg",
    "setpeername", "getpeername",
):
    if hasattr(socket.socket, _name):
        setattr(socket.socket, _name, _deny_network)
for _name in (
    "create_connection", "create_server", "getaddrinfo", "gethostbyname",
    "gethostbyname_ex", "gethostbyaddr", "getnameinfo",
):
    if hasattr(socket, _name):
        setattr(socket, _name, _deny_network)
for _name in ("create_connection", "create_datagram_endpoint", "create_server"):
    if hasattr(asyncio.BaseEventLoop, _name):
        setattr(asyncio.BaseEventLoop, _name, _deny_network)
for _name in ("sock_connect", "sock_sendall", "sock_sendto", "sock_recv", "sock_recv_into"):
    if hasattr(asyncio.BaseEventLoop, _name):
        setattr(asyncio.BaseEventLoop, _name, _deny_network)
for _name in (
    "open_connection", "start_server", "create_connection",
    "create_datagram_endpoint", "getaddrinfo",
):
    if hasattr(asyncio, _name):
        setattr(asyncio, _name, _deny_network)
for _name in (
    "Popen", "run", "call", "check_call", "check_output", "getoutput",
    "getstatusoutput",
):
    if hasattr(subprocess, _name):
        setattr(subprocess, _name, _deny_process)
for _name in dir(os):
    if _name == "system" or _name == "popen" or _name.startswith("spawn") or _name.startswith("exec"):
        try:
            setattr(os, _name, _deny_process)
        except (AttributeError, TypeError):
            pass
"#,
    )
    .map_err(|error| format!("failed to write Python network guard: {error}"))?;
    std::fs::write(
        &node_preload,
        r#"
"use strict";
const networkMessage = "court-jester network access denied";
const processMessage = "court-jester process spawn denied";
const denyNetwork = () => { throw new Error(networkMessage); };
const denyProcess = () => { throw new Error(processMessage); };
function patch(object, names, replacement) {
  if (!object) return;
  for (const name of names) {
    if (typeof object[name] === "function") object[name] = replacement;
  }
}
for (const moduleName of ["net", "tls", "dgram", "dns", "http", "https"]) {
  try {
    const module = require(moduleName);
    patch(module, [
      "connect", "createConnection", "createServer", "createSocket",
      "lookup", "lookupService", "resolve", "resolve4", "resolve6",
      "resolveAny", "resolveCaa", "resolveCname", "resolveMx",
      "resolveNaptr", "resolveNs", "resolvePtr", "resolveSoa",
      "resolveSrv", "resolveTxt", "reverse", "request", "get",
    ], denyNetwork);
    if (module.promises) {
      patch(module.promises, [
        "lookup", "lookupService", "resolve", "resolve4", "resolve6",
        "resolveAny", "resolveCaa", "resolveCname", "resolveMx",
        "resolveNaptr", "resolveNs", "resolvePtr", "resolveSoa",
        "resolveSrv", "resolveTxt", "reverse",
      ], denyNetwork);
    }
  } catch (_) {}
}
try {
  const childProcess = require("child_process");
  patch(childProcess, [
    "spawn", "spawnSync", "exec", "execSync", "execFile", "execFileSync",
    "fork",
  ], denyProcess);
} catch (_) {}
try {
  const builtinModule = require("module");
  if (typeof builtinModule.syncBuiltinESMExports === "function") {
    builtinModule.syncBuiltinESMExports();
  }
} catch (_) {}
try {
  if (globalThis.Bun) {
    for (const name of ["spawn", "spawnSync", "connect", "listen", "serve"]) {
      if (typeof globalThis.Bun[name] === "function") {
        globalThis.Bun[name] = name === "spawn" || name === "spawnSync" ? denyProcess : denyNetwork;
      }
    }
    if (typeof globalThis.Bun.fetch === "function") globalThis.Bun.fetch = denyNetwork;
  }
} catch (_) {}
try {
  const workerThreads = require("worker_threads");
  if (typeof workerThreads.Worker === "function") {
    workerThreads.Worker = function CourtJesterBlockedWorker() {
      return denyProcess();
    };
    const builtinModule = require("module");
    if (typeof builtinModule.syncBuiltinESMExports === "function") {
      builtinModule.syncBuiltinESMExports();
    }
  }
} catch (_) {}
globalThis.fetch = denyNetwork;
globalThis.WebSocket = function CourtJesterBlockedWebSocket() { denyNetwork(); };
globalThis.__COURT_JESTER_NETWORK_GUARD__ = true;
"#,
    )
    .map_err(|error| format!("failed to write Node network guard: {error}"))?;
    std::fs::write(
        &instrumentation_preload,
        r#"
"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { fileURLToPath, pathToFileURL } = require("node:url");
const target = process.env.COURT_JESTER_INSTRUMENT_TARGET;
const payload = process.env.COURT_JESTER_INSTRUMENT_PAYLOAD;
if (target && payload) {
  const targetPath = path.resolve(target);
  const originalReadFileSync = fs.readFileSync.bind(fs);
  const originalReadFile = fs.readFile.bind(fs);
  const source = originalReadFileSync(payload);
  if (globalThis.Bun && typeof globalThis.Bun.plugin === "function") {
    const escapedTargetPath = targetPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const loader = targetPath.endsWith(".tsx")
      ? "tsx"
      : targetPath.endsWith(".jsx")
        ? "jsx"
        : targetPath.endsWith(".js") || targetPath.endsWith(".mjs") || targetPath.endsWith(".cjs")
          ? "js"
          : "ts";
    globalThis.Bun.plugin({
      name: "court-jester-instrumentation",
      setup(build) {
        build.onLoad({ filter: new RegExp(`^${escapedTargetPath}$`) }, () => ({
          contents: source.toString("utf8"),
          loader,
        }));
      },
    });
  }
  function normalized(value) {
    try {
      if (value instanceof URL) value = fileURLToPath(value);
      if (Buffer.isBuffer(value)) value = value.toString();
      return typeof value === "string" ? path.resolve(value) : "";
    } catch {
      return "";
    }
  }
  function rendered(options) {
    const encoding = typeof options === "string" ? options : options?.encoding;
    return encoding ? source.toString(encoding) : Buffer.from(source);
  }
  fs.readFileSync = function courtJesterReadFileSync(filename, options) {
    return normalized(filename) === targetPath
      ? rendered(options)
      : originalReadFileSync(filename, options);
  };
  fs.readFile = function courtJesterReadFile(filename, options, callback) {
    if (typeof options === "function") {
      callback = options;
      options = undefined;
    }
    if (normalized(filename) !== targetPath) {
      return originalReadFile(filename, options, callback);
    }
    queueMicrotask(() => callback(null, rendered(options)));
  };
  if (fs.promises?.readFile) {
    const originalPromiseReadFile = fs.promises.readFile.bind(fs.promises);
    fs.promises.readFile = async function courtJesterPromiseReadFile(filename, options) {
      return normalized(filename) === targetPath
        ? rendered(options)
        : originalPromiseReadFile(filename, options);
    };
  }
  try {
    const builtinModule = require("node:module");
    builtinModule.syncBuiltinESMExports();
    if (
      typeof builtinModule.register === "function" &&
      !process.env.COURT_JESTER_TYPESCRIPT_MODULE &&
      process.env.COURT_JESTER_INSTRUMENT_LOADER_PID !== String(process.pid)
    ) {
      process.env.COURT_JESTER_INSTRUMENT_LOADER_PID = String(process.pid);
      const hookSource = `
        import fs from "node:fs";
        import path from "node:path";
        import { fileURLToPath } from "node:url";
        const targetPath = path.resolve(process.env.COURT_JESTER_INSTRUMENT_TARGET);
        const source = fs.readFileSync(process.env.COURT_JESTER_INSTRUMENT_PAYLOAD);
        export async function load(url, context, nextLoad) {
          const loaded = await nextLoad(url, context);
          if (!url.startsWith("file:") || path.resolve(fileURLToPath(url)) !== targetPath) {
            return loaded;
          }
          return { ...loaded, source };
        }
      `;
      builtinModule.register(
        `data:text/javascript,${encodeURIComponent(hookSource)}`,
        pathToFileURL(__filename).href,
      );
    }
  } catch {}
}
"#,
    )
    .map_err(|error| format!("failed to write instrumentation preload: {error}"))?;
    std::fs::write(
        &vitest_coordinator,
        r#"
import { pathToFileURL } from "node:url";

const [entrypoint, guard, instrumentation, ...args] = process.argv.slice(2);
if (!entrypoint || !guard || !instrumentation) {
  throw new Error("court-jester Vitest coordinator requires an entrypoint, worker guard, and instrumentation preload");
}
await import(pathToFileURL(instrumentation).href);
const preload = `--require=${guard} --require=${instrumentation}`;
process.env.NODE_OPTIONS = process.env.NODE_OPTIONS
  ? `${preload} ${process.env.NODE_OPTIONS}`
  : preload;
process.argv = [process.execPath, entrypoint, ...args];
await import(pathToFileURL(entrypoint).href);
"#,
    )
    .map_err(|error| format!("failed to write Vitest coordinator: {error}"))?;
    std::fs::write(
        &portable_vitest_coordinator,
        r#"
import { fileURLToPath, pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const [vitestModule, typescriptModule, testFile, guard, instrumentation, ...extraFilters] = process.argv.slice(2);
const workspaceRoot = process.env.COURT_JESTER_WORKSPACE_ROOT || "/workspace";
if (!vitestModule || !typescriptModule || !testFile || !guard || !instrumentation) {
  throw new Error("court-jester portable Vitest coordinator requires runner, compiler, test, guard, and instrumentation paths");
}
await import(pathToFileURL(instrumentation).href);
const typescriptNamespace = await import(pathToFileURL(typescriptModule).href);
let vitestNamespace = {};
let vitestImportError;
try {
  vitestNamespace = await import(pathToFileURL(vitestModule).href);
} catch (error) {
  vitestImportError = error;
}
let declaredVitestVersion = "0";
try {
  const vitestRoot = path.dirname(path.dirname(fs.realpathSync(vitestModule)));
  declaredVitestVersion = JSON.parse(
    fs.readFileSync(path.join(vitestRoot, "package.json"), "utf8"),
  ).version || "0";
} catch {}
const startVitest = vitestNamespace.startVitest;
const vitestMajor = Number.parseInt(
  String(vitestNamespace.version || declaredVitestVersion).split(".", 1)[0],
  10,
);
const ts = typescriptNamespace.default || typescriptNamespace;
let compilerOptions = {};
const configPath = ts.findConfigFile(process.cwd(), ts.sys.fileExists, "tsconfig.json");
if (configPath) {
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
  if (!loaded.error) {
    compilerOptions = ts.parseJsonConfigFileContent(
      loaded.config,
      ts.sys,
      path.dirname(configPath),
    ).options;
  }
}
Object.assign(compilerOptions, {
  module: ts.ModuleKind.ESNext,
  target: ts.ScriptTarget.ES2022,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  experimentalDecorators: true,
  emitDecoratorMetadata: false,
  useDefineForClassFields: false,
  sourceMap: false,
  inlineSourceMap: true,
  inlineSources: true,
  verbatimModuleSyntax: false,
  importsNotUsedAsValues: ts.ImportsNotUsedAsValues.Remove,
  preserveValueImports: false,
});
function resolveWorkspaceFile(base) {
  for (const candidate of [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    `${base}.js`,
    path.join(base, "index.ts"),
    path.join(base, "index.tsx"),
    path.join(base, "index.js"),
  ]) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate;
  }
  return null;
}
function workspacePackageMap(workspace) {
  const packages = new Map();
  for (const parent of ["packages", "apps"]) {
    const directory = path.join(workspace, parent);
    if (!fs.existsSync(directory)) continue;
    for (const name of fs.readdirSync(directory)) {
      const root = path.join(directory, name);
      const manifestPath = path.join(root, "package.json");
      if (!fs.existsSync(manifestPath)) continue;
      try {
        const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
        if (typeof manifest.name === "string") packages.set(manifest.name, root);
      } catch {}
    }
  }
  return packages;
}
const workspacePackages = workspacePackageMap(workspaceRoot);
function mockedModulesInTest(filename) {
  let source = "";
  try {
    source = fs.readFileSync(filename, "utf8");
  } catch {
    return new Set();
  }
  const modules = new Set();
  const mockCall = /\b(?:vi\s*\.\s*)?mock\s*\(\s*(['"])([^'"]+)\1/g;
  for (const match of source.matchAll(mockCall)) modules.add(match[2]);
  return modules;
}
const mockedModules = mockedModulesInTest(testFile);
function exportedDeclarationName(statement, exportedName) {
  if (ts.isFunctionDeclaration(statement) || ts.isClassDeclaration(statement)
      || ts.isEnumDeclaration(statement)) {
    const exported = statement.modifiers?.some(
      (modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword,
    );
    return exported && statement.name?.text === exportedName
      ? exportedName
      : null;
  }
  if (ts.isVariableStatement(statement)
      && statement.modifiers?.some(
        (modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword,
      )) {
    return statement.declarationList.declarations.some(
      (declaration) => ts.isIdentifier(declaration.name)
        && declaration.name.text === exportedName,
    ) ? exportedName : null;
  }
  return null;
}
function relativeExportFile(importer, specifier) {
  if (!specifier.startsWith(".")) return null;
  return resolveWorkspaceFile(path.resolve(path.dirname(importer), specifier));
}
function findExportedSymbol(filename, exportedName, visited = new Set()) {
  const key = `${filename}\0${exportedName}`;
  if (visited.has(key)) return null;
  visited.add(key);
  let source;
  try {
    source = fs.readFileSync(filename, "utf8");
  } catch {
    return null;
  }
  const parsed = ts.createSourceFile(
    filename,
    source,
    ts.ScriptTarget.Latest,
    true,
    filename.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  for (const statement of parsed.statements) {
    if (exportedDeclarationName(statement, exportedName)) {
      return { filename, importedName: exportedName };
    }
    if (!ts.isExportDeclaration(statement)) continue;
    const moduleName = statement.moduleSpecifier
      && ts.isStringLiteral(statement.moduleSpecifier)
      ? statement.moduleSpecifier.text
      : null;
    const target = moduleName ? relativeExportFile(filename, moduleName) : null;
    if (statement.exportClause && ts.isNamedExports(statement.exportClause)) {
      for (const element of statement.exportClause.elements) {
        if (element.name.text !== exportedName) continue;
        const importedName = element.propertyName?.text || element.name.text;
        return target
          ? { filename: target, importedName }
          : { filename, importedName };
      }
    } else if (!statement.exportClause && target) {
      const resolved = findExportedSymbol(target, exportedName, visited);
      if (resolved) return resolved;
    }
  }
  return null;
}
function narrowWorkspaceImports(source, filename) {
  const parsed = ts.createSourceFile(
    filename,
    source,
    ts.ScriptTarget.Latest,
    true,
    filename.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const replacements = [];
  for (const statement of parsed.statements) {
    if (!ts.isImportDeclaration(statement)
        || !ts.isStringLiteral(statement.moduleSpecifier)
        || !statement.importClause
        || statement.importClause.isTypeOnly
        || !statement.importClause.namedBindings
        || !ts.isNamedImports(statement.importClause.namedBindings)) {
      continue;
    }
    const packageRoot = workspacePackages.get(statement.moduleSpecifier.text);
    if (mockedModules.has(statement.moduleSpecifier.text)) continue;
    if (!packageRoot) continue;
    const manifest = JSON.parse(
      fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
    );
    const sourceEntry = typeof manifest.source === "string"
      ? manifest.source
      : "src/index.ts";
    const entrypoint = resolveWorkspaceFile(path.resolve(packageRoot, sourceEntry));
    if (!entrypoint) continue;
    const runtimeImports = statement.importClause.namedBindings.elements.filter(
      (element) => !element.isTypeOnly,
    );
    const resolved = runtimeImports.map((element) => ({
      element,
      target: findExportedSymbol(
        entrypoint,
        element.propertyName?.text || element.name.text,
      ),
    }));
    if (resolved.some((entry) => !entry.target)) continue;
    const byTarget = new Map();
    for (const { element, target } of resolved) {
      const imports = byTarget.get(target.filename) || [];
      const localName = element.name.text;
      imports.push(target.importedName === localName
        ? target.importedName
        : `${target.importedName} as ${localName}`);
      byTarget.set(target.filename, imports);
    }
    const replacement = [...byTarget].map(([target, imports]) =>
      `import { ${imports.join(", ")} } from ${JSON.stringify(target)};`
    ).join("\n");
    replacements.push({
      start: statement.getStart(parsed),
      end: statement.end,
      replacement,
    });
  }
  let narrowed = source;
  for (const replacement of replacements.reverse()) {
    narrowed = narrowed.slice(0, replacement.start)
      + replacement.replacement
      + narrowed.slice(replacement.end);
  }
  return narrowed;
}

const workspaceResolver = {
  name: "court-jester-workspace-resolver",
  enforce: "pre",
  resolveId(specifier) {
    for (const [packageName, packageRoot] of workspacePackages) {
      if (specifier === packageName) {
        const manifest = JSON.parse(
          fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
        );
        const sourceEntry = typeof manifest.source === "string" ? manifest.source : "src/index.ts";
        return resolveWorkspaceFile(path.resolve(packageRoot, sourceEntry));
      }
      if (specifier.startsWith(`${packageName}/`)) {
        return resolveWorkspaceFile(
          path.join(packageRoot, specifier.slice(packageName.length + 1)),
        );
      }
    }
    return null;
  },
};
const transform = {
  name: "court-jester-typescript-transform",
  enforce: "pre",
  transform(source, id) {
    const withoutQuery = id.split("?", 1)[0];
    const filename = withoutQuery.startsWith("file:")
      ? fileURLToPath(withoutQuery)
      : withoutQuery;
    if (!(filename === workspaceRoot || filename.startsWith(`${workspaceRoot}${path.sep}`))
        || filename.endsWith(".d.ts")
        || !/\.[cm]?[jt]sx?$/.test(filename)) {
      return null;
    }
    if (!/\.[cm]?tsx?$/.test(filename)) {
      return { code: source, map: null };
    }
    const output = ts.transpileModule(narrowWorkspaceImports(source, filename), {
      compilerOptions,
      fileName: filename,
      reportDiagnostics: false,
    });
    return { code: output.outputText, map: null };
  },
};
const preload = `--require=${guard}`;
process.env.NODE_OPTIONS = process.env.NODE_OPTIONS
  ? `${preload} ${process.env.NODE_OPTIONS}`
  : preload;
const normalizedTestFile = path.isAbsolute(testFile)
  ? path.relative(workspaceRoot, testFile)
  : testFile;
const filters = [normalizedTestFile, ...extraFilters];
const options = {
  run: true,
  reporters: ["json"],
  cache: false,
  threads: false,
  pool: "forks",
  maxWorkers: 1,
  minWorkers: 1,
};
const viteOverrides = {
  root: process.cwd(),
  esbuild: false,
  cacheDir: "/tmp/court-jester-vite",
  plugins: [workspaceResolver, transform],
};
async function runVitest(overrides) {
  if (vitestImportError) throw vitestImportError;
  if (typeof startVitest !== "function") {
    throw new Error("the selected Vitest package does not export startVitest");
  }
  return vitestMajor >= 1
    ? startVitest("test", filters, options, overrides)
    : startVitest(filters, options, overrides);
}
function capturedWrite(chunks, chunk, encoding, callback) {
  chunks.push(Buffer.isBuffer(chunk) ? chunk.toString() : String(chunk));
  const completed = typeof encoding === "function" ? encoding : callback;
  if (typeof completed === "function") completed();
  return true;
}
async function captureVitest(overrides) {
  const previousExitCode = process.exitCode;
  const stdoutWrite = process.stdout.write;
  const stderrWrite = process.stderr.write;
  const stdout = [];
  const stderr = [];
  process.exitCode = 0;
  process.stdout.write = (chunk, encoding, callback) =>
    capturedWrite(stdout, chunk, encoding, callback);
  process.stderr.write = (chunk, encoding, callback) =>
    capturedWrite(stderr, chunk, encoding, callback);
  let result;
  let error;
  try {
    result = await runVitest(overrides);
  } catch (caught) {
    error = caught;
  } finally {
    process.stdout.write = stdoutWrite;
    process.stderr.write = stderrWrite;
  }
  const exitCode = Number(process.exitCode || 0);
  process.exitCode = previousExitCode;
  return {
    result,
    error,
    exitCode,
    stdout: stdout.join(""),
    stderr: stderr.join(""),
  };
}
function collectedTestCount(output) {
  let count = 0;
  for (const match of output.matchAll(/"numTotalTests"\s*:\s*(\d+)/g)) {
    count = Math.max(count, Number(match[1]));
  }
  return count;
}
function nativeToolchainFailure(error) {
  const message = String(error?.stack || error || "");
  return [
    "another platform",
    "@rollup/rollup-",
    "@esbuild/",
    "invalid ELF header",
    "Exec format Error",
    "Exec format error",
    "not a valid Win32 application",
    "wrong architecture",
  ].some((fragment) => message.includes(fragment));
}
function matchingRunnerFailure(error) {
  const message = String(error?.stack || error || "");
  return nativeToolchainFailure(error) || [
    "Vitest failed to access its internal state",
    "Vitest was initialized with native Node instead of Vite Node",
    "customEqualityTesters",
    "workerState.config",
  ].some((fragment) => message.includes(fragment));
}
function completedVitestAttempt(attempt) {
  return !attempt.error
    && attempt.result !== false
    && (attempt.exitCode === 0 || collectedTestCount(attempt.stdout) > 0);
}
function publishVitestAttempt(attempt) {
  if (attempt.stdout) process.stdout.write(attempt.stdout);
  if (attempt.stderr) process.stderr.write(attempt.stderr);
  process.exitCode = attempt.result === false ? 1 : attempt.exitCode;
}
function matchingRunnerModule() {
  const vitestRoot = path.dirname(path.dirname(fs.realpathSync(vitestModule)));
  const runnerRoots = [
    path.join(vitestRoot, "node_modules", "@vitest", "runner"),
    path.join(path.dirname(vitestRoot), "@vitest", "runner"),
  ];
  for (const root of runnerRoots) {
    for (const entry of ["dist/index.js", "dist/index.mjs"]) {
      const candidate = path.join(root, entry);
      if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate;
    }
  }
  throw new Error(
    `matching @vitest/runner is unavailable beside ${fs.realpathSync(vitestModule)}`,
  );
}
function errorText(error) {
  return String(error?.stack || error?.message || error || "unknown test runner error");
}
function assertionStatus(task) {
  if (task.mode === "skip" || task.mode === "todo") return "pending";
  if (task.result?.state === "pass") return "passed";
  if (task.result?.state === "skip" || task.result?.state === "todo") return "pending";
  return "failed";
}
function collectAssertions(task, ancestors, assertions) {
  if (task.type === "test") {
    const status = assertionStatus(task);
    assertions.push({
      ancestorTitles: ancestors,
      title: task.name,
      fullName: [...ancestors, task.name].join(" "),
      status,
      duration: task.result?.duration,
      failureMessages: (task.result?.errors || []).map(errorText),
    });
    return;
  }
  const nextAncestors = task.type === "suite" && ancestors.length > 0
    ? [...ancestors, task.name]
    : task.type === "suite" && task.tasks
      ? [task.name]
      : ancestors;
  for (const child of task.tasks || []) {
    collectAssertions(child, nextAncestors, assertions);
  }
}
function collectSuiteErrors(task, errors) {
  if (task.type !== "test" && task.result?.state === "fail") {
    const taskErrors = (task.result?.errors || []).map(errorText);
    if (taskErrors.length > 0) {
      errors.push(...taskErrors);
    } else {
      errors.push(`suite ${task.name} failed`);
    }
  }
  for (const child of task.tasks || []) collectSuiteErrors(child, errors);
}
function directVitestSummary(files) {
  const testResults = files.map((file) => {
    const assertionResults = [];
    const suiteErrors = [];
    for (const task of file.tasks || []) {
      collectAssertions(task, [], assertionResults);
      collectSuiteErrors(task, suiteErrors);
    }
    const fileErrors = [...(file.result?.errors || []).map(errorText), ...suiteErrors];
    const failed = file.result?.state === "fail"
      || fileErrors.length > 0
      || assertionResults.some((assertion) => assertion.status === "failed");
    return {
      assertionResults,
      startTime: file.result?.startTime,
      endTime: file.result?.startTime && file.result?.duration
        ? file.result.startTime + file.result.duration
        : undefined,
      status: failed ? "failed" : "passed",
      message: fileErrors.join("\n"),
      name: file.filepath || file.name,
    };
  });
  const assertions = testResults.flatMap((result) => result.assertionResults);
  const failedTests = assertions.filter((assertion) => assertion.status === "failed").length;
  const passedTests = assertions.filter((assertion) => assertion.status === "passed").length;
  const pendingTests = assertions.length - failedTests - passedTests;
  const failedSuites = testResults.filter((result) => result.status === "failed").length;
  const passedSuites = testResults.length - failedSuites;
  return {
    numTotalTestSuites: testResults.length,
    numPassedTestSuites: passedSuites,
    numFailedTestSuites: failedSuites,
    numPendingTestSuites: 0,
    numTotalTests: assertions.length,
    numPassedTests: passedTests,
    numFailedTests: failedTests,
    numPendingTests: pendingTests,
    success: assertions.length > 0 && failedTests === 0 && failedSuites === 0,
    testResults,
  };
}
function initializationSummary(error) {
  return {
    numTotalTestSuites: 1,
    numPassedTestSuites: 0,
    numFailedTestSuites: 1,
    numPendingTestSuites: 0,
    numTotalTests: 0,
    numPassedTests: 0,
    numFailedTests: 0,
    numPendingTests: 0,
    success: false,
    testResults: [{
      assertionResults: [],
      status: "failed",
      message: errorText(error),
      name: testFile,
    }],
  };
}
async function runDirectVitest() {
  try {
    if (extraFilters.length > 0) {
      throw new Error(
        `matching package runner cannot preserve Vitest filters: ${extraFilters.join(" ")}`,
      );
    }
    const runnerNamespace = await import(pathToFileURL(matchingRunnerModule()).href);
    if (typeof runnerNamespace.startTests !== "function") {
      throw new Error("matching @vitest/runner does not export startTests");
    }
    const config = {
      root: process.cwd(),
      setupFiles: [],
      passWithNoTests: false,
      allowOnly: true,
      sequence: { seed: 1, hooks: "list", setupFiles: "list" },
      maxConcurrency: 1,
      testTimeout: 5000,
      hookTimeout: 10000,
      retry: 0,
      clearMocks: false,
      mockReset: false,
      restoreMocks: false,
      unstubGlobals: false,
      unstubEnvs: false,
      fakeTimers: {},
      expect: {},
    };
    globalThis.__vitest_worker__ = {
      config,
      environment: { name: "node", options: null },
      filepath: testFile,
      moduleCache: new Map(),
      providedContext: {},
      durations: { environment: 0, prepare: 0 },
      onCancel: new Promise(() => {}),
      rpc: {},
    };
    let importSequence = 0;
    const runner = {
      config,
      importFile(filepath) {
        const absolute = path.isAbsolute(filepath) ? filepath : path.resolve(filepath);
        importSequence += 1;
        return import(`${pathToFileURL(absolute).href}?court_jester=${importSequence}`);
      },
    };
    const files = await runnerNamespace.startTests([testFile], runner);
    const summary = directVitestSummary(files);
    process.stdout.write(`${JSON.stringify(summary)}\n`);
    process.exitCode = summary.success ? 0 : 1;
  } catch (error) {
    process.stdout.write(`${JSON.stringify(initializationSummary(error))}\n`);
    process.exitCode = 1;
  }
}

let attempt = await captureVitest(viteOverrides);
if (completedVitestAttempt(attempt)) {
  publishVitestAttempt(attempt);
} else {
  const firstFailure = attempt.error
    || `${attempt.stderr}\n${attempt.stdout}`;
  if (!nativeToolchainFailure(firstFailure)) {
    publishVitestAttempt(attempt);
    if (attempt.error) throw attempt.error;
  } else {
    process.stderr.write(
      "court-jester project config requires a host-incompatible native dependency; retrying without project config\n",
    );
    attempt = await captureVitest({ ...viteOverrides, configFile: false });
    if (completedVitestAttempt(attempt)) {
      publishVitestAttempt(attempt);
    } else {
      const secondFailure = attempt.error
        || `${attempt.stderr}\n${attempt.stdout}`;
      if (vitestMajor >= 1 && matchingRunnerFailure(secondFailure)) {
        process.stderr.write(
          "court-jester native Vitest dependencies are incompatible with the isolated runtime; using the matching package runner\n",
        );
        process.exitCode = 0;
        await runDirectVitest();
      } else {
        publishVitestAttempt(attempt);
        if (attempt.error) throw attempt.error;
        process.exitCode = 1;
      }
    }
  }
}
"#,
    )
    .map_err(|error| format!("failed to write portable Vitest coordinator: {error}"))?;
    std::fs::write(
        &typescript_loader,
        r#"
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const typescriptModule = process.env.COURT_JESTER_TYPESCRIPT_MODULE;
if (!typescriptModule) {
  throw new Error("court-jester TypeScript loader requires COURT_JESTER_TYPESCRIPT_MODULE");
}
const typescriptNamespace = await import(pathToFileURL(typescriptModule).href);
const ts = typescriptNamespace.default || typescriptNamespace;
const compilerOptionsByDirectory = new Map();
function canonicalPath(filename) {
  try {
    return fs.realpathSync(filename);
  } catch {
    return path.resolve(filename);
  }
}
const instrumentTarget = process.env.COURT_JESTER_INSTRUMENT_TARGET
  ? canonicalPath(process.env.COURT_JESTER_INSTRUMENT_TARGET)
  : null;
const instrumentPayload = process.env.COURT_JESTER_INSTRUMENT_PAYLOAD || null;

function compilerOptionsFor(filename) {
  const directory = path.dirname(filename);
  if (compilerOptionsByDirectory.has(directory)) {
    return compilerOptionsByDirectory.get(directory);
  }
  let compilerOptions = {};
  const configPath = ts.findConfigFile(directory, ts.sys.fileExists, "tsconfig.json");
  if (configPath) {
    const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
    if (!loaded.error) {
      compilerOptions = ts.parseJsonConfigFileContent(
        loaded.config,
        ts.sys,
        path.dirname(configPath),
      ).options;
    }
  }
  Object.assign(compilerOptions, {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2022,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    experimentalDecorators: true,
    emitDecoratorMetadata: false,
    useDefineForClassFields: false,
    sourceMap: false,
    inlineSourceMap: true,
    inlineSources: true,
    verbatimModuleSyntax: true,
    importsNotUsedAsValues: ts.ImportsNotUsedAsValues.Preserve,
    preserveValueImports: true,
  });
  compilerOptionsByDirectory.set(directory, compilerOptions);
  return compilerOptions;
}

export async function load(url, context, nextLoad) {
  if (!url.startsWith("file:")) return nextLoad(url, context);
  const filename = fileURLToPath(url);
  const isInstrumentTarget = instrumentTarget === canonicalPath(filename) && instrumentPayload;
  if (isInstrumentTarget && /\.[cm]?js$/.test(filename)) {
    const loaded = await nextLoad(url, context);
    return {
      ...loaded,
      source: fs.readFileSync(instrumentPayload, "utf8"),
    };
  }
  if (filename.endsWith(".json")) {
    const value = JSON.parse(fs.readFileSync(filename, "utf8"));
    return {
      format: "module",
      source: `export default ${JSON.stringify(value)};\n`,
      shortCircuit: true,
    };
  }
  if (filename.endsWith(".d.ts") || !/\.(?:[cm]?tsx?|[cm]?jsx)$/.test(filename)) {
    return nextLoad(url, context);
  }
  const source = isInstrumentTarget
    ? fs.readFileSync(instrumentPayload, "utf8")
    : fs.readFileSync(filename, "utf8");
  const output = ts.transpileModule(source, {
    compilerOptions: compilerOptionsFor(filename),
    fileName: filename,
    reportDiagnostics: false,
  });
  return {
    format: "module",
    source: output.outputText,
    shortCircuit: true,
  };
}
"#,
    )
    .map_err(|error| format!("failed to write TypeScript runtime loader: {error}"))?;
    Ok(NetworkGuard {
        _directory: directory,
        python_sitecustomize,
        node_preload,
        instrumentation_preload,
        instrumented_source: instrumented_source_path,
        vitest_coordinator,
        portable_vitest_coordinator,
        typescript_loader,
    })
}

fn prepend_env_value(env: &mut Vec<(String, String)>, key: &str, prefix: &str) {
    if let Some((_, value)) = env.iter_mut().find(|(name, _)| name == key) {
        if value.is_empty() {
            *value = prefix.to_string();
        } else {
            *value = format!("{prefix}:{}", value);
        }
    } else {
        env.push((key.to_string(), prefix.to_string()));
    }
}

fn apply_network_guard(env: &mut Vec<(String, String)>, language: &Language, guard: &NetworkGuard) {
    match language {
        Language::Python => {
            prepend_env_value(
                env,
                "PYTHONPATH",
                guard
                    .python_sitecustomize
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_string_lossy()
                    .as_ref(),
            );
        }
        Language::TypeScript => {
            let preload = format!("--require={}", guard.node_preload.display());
            if let Some((_, value)) = env.iter_mut().find(|(name, _)| name == "NODE_OPTIONS") {
                *value = format!("{preload} {value}");
            } else {
                env.push(("NODE_OPTIONS".into(), preload));
            }
        }
    }
}

/// Walk up from a directory to find the Python package root.
/// Returns the deepest ancestor that has __init__.py in it,
/// plus the path from there to the starting dir.
fn find_python_package_root(start_dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    // Walk up while __init__.py exists
    let mut dir = start_dir.to_path_buf();
    let mut parts: Vec<String> = vec![];

    loop {
        if !dir.join("__init__.py").exists() {
            break;
        }
        parts.push(dir.file_name()?.to_str()?.to_string());
        dir = dir.parent()?.to_path_buf();
    }

    if parts.is_empty() {
        return None;
    }

    parts.reverse();
    let module_prefix = parts.join(".");
    // dir is now the parent of the package root
    Some((dir, module_prefix))
}

fn module_run_for_python_source(
    source_path: &std::path::Path,
) -> Option<(std::path::PathBuf, String)> {
    let parent = source_path.parent()?;
    let stem = source_path.file_stem()?.to_str()?;
    if !is_valid_python_module_name(stem) {
        return None;
    }

    let (pkg_root_parent, module_prefix) = find_python_package_root(parent)?;
    Some((pkg_root_parent, format!("{module_prefix}.{stem}")))
}

pub async fn docker_daemon_ready() -> Result<(), String> {
    docker_daemon_ready_with_limits(10.0, 128).await
}

pub async fn docker_daemon_ready_with_limits(timeout: f64, memory_mb: u64) -> Result<(), String> {
    docker_readiness_output(&["info"], timeout, memory_mb)
        .await
        .map(|_| ())
}

pub async fn docker_image_id(image: &str) -> Result<String, String> {
    docker_image_id_with_limits(image, 10.0, 128).await
}

pub async fn docker_image_id_with_limits(
    image: &str,
    timeout: f64,
    memory_mb: u64,
) -> Result<String, String> {
    if image.trim().is_empty() || image.starts_with('-') {
        return Err("docker image must be non-empty and must not begin with '-'".into());
    }
    let output = docker_readiness_output(&["image", "inspect", image], timeout, memory_mb).await?;
    serde_json::from_str::<serde_json::Value>(&output)
        .ok()
        .and_then(|v| v.get(0)?.get("Id")?.as_str().map(str::to_owned))
        .ok_or_else(|| "docker image inspect returned no image id".into())
}

async fn docker_readiness_output(
    args: &[&str],
    timeout: f64,
    memory_mb: u64,
) -> Result<String, String> {
    if !timeout.is_finite() || timeout <= 0.0 || memory_mb == 0 {
        return Err("Docker readiness requires finite positive timeout and memory limits".into());
    }
    let result = docker_output(args, timeout, memory_mb).await;
    if !docker_command_succeeded(&result) {
        return Err(format!(
            "Docker {} readiness probe failed (exit {:?}): {}",
            args.join(" "),
            result.exit_code,
            result.stderr.trim()
        ));
    }
    Ok(result.stdout)
}

fn docker_command_succeeded(result: &ExecutionResult) -> bool {
    result.exit_code == Some(0) && !result.timed_out && !result.memory_error
}

fn docker_lifecycle_failure(
    operation: &str,
    result: ExecutionResult,
    cleanup: Option<&ExecutionResult>,
) -> ExecutionResult {
    let mut failure = launch_failure(format!(
        "Docker {operation} failed (client exit {:?}): {}",
        result.exit_code,
        result.stderr.trim()
    ));
    failure.timed_out = result.timed_out;
    failure.memory_error = result.memory_error;
    failure.duration_ms = result.duration_ms;
    if result.timed_out || result.memory_error {
        failure.termination = result.termination;
        for diagnostic in &mut failure.diagnostics {
            diagnostic.kind = if result.memory_error {
                FailureKind::MemoryLimit
            } else {
                FailureKind::Timeout
            };
            diagnostic.process = failure.termination.clone();
        }
    }
    if let Some(cleanup) = cleanup.filter(|output| !docker_command_succeeded(output)) {
        let message = format!(
            "Container cleanup could not be confirmed: {}",
            cleanup.stderr.trim()
        );
        failure.stderr.push_str(&format!("\n{message}"));
        for diagnostic in &mut failure.diagnostics {
            diagnostic.message.push_str(&format!("; {message}"));
        }
    }
    failure
}

async fn docker_output(args: &[&str], timeout: f64, memory_mb: u64) -> ExecutionResult {
    docker_program_output(std::path::Path::new("docker"), args, timeout, memory_mb).await
}

async fn docker_program_output(
    program: &std::path::Path,
    args: &[&str],
    timeout: f64,
    memory_mb: u64,
) -> ExecutionResult {
    if timeout <= 0.0 {
        let mut result = launch_failure("Docker lifecycle deadline exhausted");
        result.timed_out = true;
        result.termination = Some(termination(ProcessTerminationKind::TimedOut, None, None));
        return result;
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    run_command_with_limits(
        command,
        timeout,
        memory_mb,
        RuntimeProfile::LocalTrusted,
        NetworkPolicy::Deny,
        true,
        "docker unavailable",
    )
    .await
}

/// Execute code using the shared context, materialization, and harness launcher.
pub async fn execute(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_standalone(code, language, options, None).await
}

async fn execute_standalone(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
    runtime_override: Option<HarnessRuntime>,
) -> ExecutionResult {
    if let Err(message) = options.validate() {
        return launch_failure(message);
    }
    let invocation_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            return launch_failure(format!("cannot resolve invocation directory: {error}"))
        }
    };
    let standalone_root = if options.project_dir.is_none() && options.source_file.is_none() {
        Some(match standalone_runtime_tempdir(options.runtime_profile) {
            Ok(root) => root,
            Err(error) => {
                return launch_failure(format!("failed to create standalone workspace: {error}"));
            }
        })
    } else {
        None
    };
    let context = if let Some(root) = standalone_root.as_ref() {
        let root = root.path().to_path_buf();
        ExecutionContext {
            invocation_dir: root.clone(),
            workspace_root: root.clone(),
            target_package_root: root,
            materialization_source_root: None,
            test_package_root: None,
            dependency_roots: Vec::new(),
            target_source: SourceContext {
                language: *language,
                mode: if *language == Language::Python {
                    SourceMode::Python
                } else {
                    SourceMode::TypeScript
                },
                source_file: None,
                virtual_file_path: None,
            },
            test_source: None,
        }
    } else {
        match crate::resolve_execution_context(crate::types::ContextRequest {
            invocation_dir: &invocation_dir,
            explicit_project_dir: options.project_dir.map(std::path::Path::new),
            target_file: options.source_file.map(std::path::Path::new),
            test_file: None,
            language: *language,
            virtual_file_path: None,
        }) {
            Ok(context) => context,
            Err(error) => {
                return launch_failure(format!("cannot resolve execution context: {error}"));
            }
        }
    };
    let target_path = context.target_source.source_file.as_deref();
    let target_path_string = target_path.and_then(|path| path.to_str());
    let mode = context.target_source.mode;
    let extension = match mode {
        SourceMode::Python => "py",
        SourceMode::TypeScript => "ts",
        SourceMode::Tsx => "tsx",
    };
    let relative_entry = target_path
        .and_then(|path| path.strip_prefix(&context.workspace_root).ok())
        .filter(|path| !path.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from(format!(".court-jester-execute.{extension}")));
    let executable_code = if *language == Language::TypeScript {
        strip_typescript_type_only_imports(code, target_path_string)
    } else {
        code.to_string()
    };
    let existing = target_path_string
        .filter(|path| source_matches_disk(&executable_code, Some(path)).is_some())
        .is_some();
    let artifact = if existing {
        HarnessArtifact::Existing {
            relative_path: relative_entry,
        }
    } else {
        HarnessArtifact::Generated {
            code: executable_code,
            relative_path: relative_entry,
        }
    };
    let runtime = runtime_override.unwrap_or_else(|| match language {
        Language::Python => HarnessRuntime::Python,
        Language::TypeScript => {
            let source = target_path.and_then(|path| path.to_str());
            let repo_runner =
                detect_repo_typescript_runner(context.workspace_root.to_str(), source);
            let path = std::env::var("PATH").unwrap_or_default();
            if repo_runner.as_deref() == Some("bun") {
                HarnessRuntime::BunScript
            } else if which_binary(&path, "node").is_some() {
                HarnessRuntime::NodeScript
            } else {
                HarnessRuntime::BunScript
            }
        }
    });
    let harness = HarnessSpec {
        kind: HarnessKind::Standalone,
        runtime,
        test_adapter: None,
        source_mode: mode,
        artifact,
        args: Vec::new(),
        network: options.network_policy,
    };
    let project_dir_owned = context.workspace_root.to_string_lossy().into_owned();
    let source_file_owned = target_path.map(|path| path.to_string_lossy().into_owned());
    let limits = SandboxOptions {
        timeout_seconds: options.timeout_seconds,
        memory_mb: options.memory_mb,
        runtime_profile: options.runtime_profile,
        network_policy: options.network_policy,
        harness_args: options.harness_args,
        docker_image: options.docker_image,
        project_dir: Some(project_dir_owned.as_str()),
        source_file: source_file_owned.as_deref(),
        instrumentation_target: None,
        instrumented_source: None,
    };
    execute_harness(&context, harness, limits).await.process
}

pub async fn execute_typescript_node(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::NodeScript),
    )
    .await
}

pub async fn execute_typescript_bun(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::BunScript),
    )
    .await
}

pub async fn execute_typescript_bun_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_standalone(
        code,
        &Language::TypeScript,
        options,
        Some(HarnessRuntime::BunTest),
    )
    .await
}

pub async fn execute_typescript_repo_native(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_standalone(
            code,
            &Language::TypeScript,
            options,
            Some(HarnessRuntime::BunScript),
        )
        .await,
    )
}

pub async fn execute_typescript_repo_native_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_standalone(
            code,
            &Language::TypeScript,
            options,
            Some(HarnessRuntime::BunTest),
        )
        .await,
    )
}

fn normalize_harness_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    if path.is_absolute() {
        return Err("harness artifact paths must be relative".into());
    }
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("harness artifact path escapes its overlay".into())
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("harness artifact path must not be empty".into());
    }
    if normalized.to_string_lossy().contains('\0') {
        return Err("harness artifact path contains NUL".into());
    }
    Ok(normalized)
}

fn harness_extension_compatible(path: &std::path::Path, mode: SourceMode) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match mode {
        SourceMode::Python => extension.as_deref() == Some("py"),
        SourceMode::TypeScript => matches!(
            extension.as_deref(),
            Some("ts") | Some("mts") | Some("cts") | Some("js") | Some("mjs") | Some("cjs")
        ),
        SourceMode::Tsx => matches!(extension.as_deref(), Some("tsx") | Some("jsx")),
    }
}

fn resolve_harness_arg(
    argument: &HarnessArg,
    workspace_root: &std::path::Path,
) -> Result<std::ffi::OsString, String> {
    match argument {
        HarnessArg::Literal { literal } => {
            if literal.contains('\0') {
                Err("harness literal argument contains NUL".into())
            } else {
                Ok(literal.clone().into())
            }
        }
        HarnessArg::ProjectPath { project_path } => {
            let path = std::path::Path::new(project_path);
            if path.is_absolute() {
                return Err("project_path arguments must be relative".into());
            }
            let normalized = normalize_harness_path(path)?;
            let joined = workspace_root.join(normalized);
            let canonical = std::fs::canonicalize(&joined).map_err(|error| {
                format!("project_path '{}' is unavailable: {error}", project_path)
            })?;
            if !canonical.starts_with(workspace_root) {
                return Err(format!(
                    "project_path '{}' escapes workspace root",
                    project_path
                ));
            }
            Ok(canonical.into_os_string())
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum StructuredTestFailure {
    Assertion,
    Initialization(String),
    ProcessSpawnDenied,
}

fn structured_test_failure(
    output: &str,
    _recognize_missing_vitest_globals: bool,
) -> Option<StructuredTestFailure> {
    const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
    const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
    const MAX_JSON_DEPTH: usize = 256;
    const MAX_REPORTER_CANDIDATES: usize = 16;

    fn contains_failure(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Array(values) => values.iter().any(contains_failure),
            serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
                matches!(
                    (key.as_str(), value),
                    ("status", serde_json::Value::String(status)) if status == "failed"
                ) || matches!(
                    (key.as_str(), value),
                    ("pass", serde_json::Value::Bool(false))
                ) || contains_failure(value)
            }),
            _ => false,
        }
    }

    fn contains_process_spawn_denial(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::String(message) => {
                message.contains("court-jester process spawn denied")
            }
            serde_json::Value::Array(values) => values.iter().any(contains_process_spawn_denial),
            serde_json::Value::Object(fields) => fields.values().any(contains_process_spawn_denial),
            _ => false,
        }
    }

    fn reporter_failure(value: &serde_json::Value) -> Option<StructuredTestFailure> {
        let fields = value.as_object()?;
        let has_test_results = fields
            .get("testResults")
            .is_some_and(serde_json::Value::is_array);
        let has_summary = fields
            .get("success")
            .is_some_and(serde_json::Value::is_boolean)
            && [
                "numTotalTests",
                "numPassedTests",
                "numFailedTests",
                "numTotalTestSuites",
                "numPassedTestSuites",
                "numFailedTestSuites",
            ]
            .iter()
            .any(|key| fields.get(*key).is_some_and(serde_json::Value::is_number));
        if !has_test_results && !has_summary {
            return None;
        }
        let reported_failure = fields.get("success").and_then(serde_json::Value::as_bool)
            == Some(false)
            || ["numFailedTests", "numFailedTestSuites"].iter().any(|key| {
                fields
                    .get(*key)
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|count| count > 0)
            })
            || contains_failure(value);
        if reported_failure && contains_process_spawn_denial(value) {
            return Some(StructuredTestFailure::ProcessSpawnDenied);
        }

        if reported_failure
            && fields
                .get("numTotalTests")
                .and_then(serde_json::Value::as_u64)
                == Some(0)
        {
            let initialization_message = fields
                .get("testResults")
                .and_then(serde_json::Value::as_array)
                .and_then(|results| {
                    results.iter().find_map(|result| {
                        result
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|message| !message.is_empty())
                    })
                })
                .or_else(|| {
                    fields
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                })
                .unwrap_or("test runner failed before collecting any tests");
            return Some(StructuredTestFailure::Initialization(
                initialization_message.to_string(),
            ));
        }

        reported_failure.then_some(StructuredTestFailure::Assertion)
    }

    let mut scan_start = output.len().saturating_sub(MAX_SCAN_BYTES);
    while !output.is_char_boundary(scan_start) {
        scan_start += 1;
    }
    let output = &output[scan_start..];
    let bytes = output.as_bytes();
    let mut stack: Vec<(u8, usize, bool)> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut string_start = 0;
    let mut reporter_candidates = 0;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                let token = &bytes[string_start..index];
                if matches!(
                    token,
                    b"testResults" | b"numFailedTests" | b"numFailedTestSuites"
                ) {
                    if let Some(frame) = stack.last_mut() {
                        frame.2 = true;
                    }
                }
            }
            continue;
        }

        match byte {
            b'"' if !stack.is_empty() => {
                in_string = true;
                string_start = index + 1;
            }
            b'{' | b'[' => {
                if stack.len() == MAX_JSON_DEPTH {
                    stack.clear();
                } else {
                    stack.push((byte, index, false));
                }
            }
            b'}' | b']' => {
                let expected = if byte == b'}' { b'{' } else { b'[' };
                let Some(&(opening, start, has_reporter_marker)) = stack.last() else {
                    continue;
                };
                if opening != expected {
                    stack.clear();
                    continue;
                }
                stack.pop();

                let document_len = index + 1 - start;
                if opening != b'{'
                    || !has_reporter_marker
                    || document_len > MAX_DOCUMENT_BYTES
                    || reporter_candidates == MAX_REPORTER_CANDIDATES
                {
                    continue;
                }
                let candidate = &output[start..=index];
                reporter_candidates += 1;
                if let Ok(value) = serde_json::from_str(candidate) {
                    if let Some(failure) = reporter_failure(&value) {
                        return Some(failure);
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Decode a reporter's line envelope without interpreting its payload.
pub fn test_output_line(line: &str, adapter: Option<TestAdapter>) -> &str {
    let line = line.trim();
    if adapter == Some(TestAdapter::NodeTap) {
        line.strip_prefix("# ").unwrap_or(line)
    } else {
        line
    }
}

fn harness_diagnostics(
    adapter: Option<TestAdapter>,
    process: &ExecutionResult,
    limits: &ExecutionLimits,
) -> Vec<FailureDiagnostic> {
    // Node TAP moves child stdout/stderr into comment records. Preserve those
    // child diagnostics for the same classification rules used on raw stderr.
    let mut transported;
    let process = if adapter == Some(TestAdapter::NodeTap) {
        transported = process.clone();
        for line in process
            .stdout
            .lines()
            .filter(|line| line.trim().starts_with("# "))
        {
            transported.stderr.push('\n');
            transported.stderr.push_str(test_output_line(line, adapter));
        }
        &transported
    } else {
        process
    };
    let mut diagnostics = Vec::new();
    if let Some(termination) = process.termination.as_ref() {
        let (domain, kind, message) = match termination.kind {
            ProcessTerminationKind::TimedOut => (
                FailureDomain::Resource,
                FailureKind::Timeout,
                "harness timed out".to_string(),
            ),
            ProcessTerminationKind::MemoryLimit => (
                FailureDomain::Resource,
                FailureKind::MemoryLimit,
                "harness exceeded the memory limit".to_string(),
            ),
            ProcessTerminationKind::Signaled => (
                FailureDomain::Environment,
                FailureKind::Signal,
                termination
                    .signal_name
                    .clone()
                    .unwrap_or_else(|| "harness terminated by signal".into()),
            ),
            ProcessTerminationKind::LaunchFailed => (
                FailureDomain::Environment,
                FailureKind::LauncherFailure,
                process.stderr.clone(),
            ),
            ProcessTerminationKind::WaitFailed => (
                FailureDomain::Environment,
                FailureKind::ToolFailure,
                process.stderr.clone(),
            ),
            ProcessTerminationKind::Exited => {
                if process.exit_code == Some(0) {
                    (
                        FailureDomain::Environment,
                        FailureKind::ToolFailure,
                        String::new(),
                    )
                } else {
                    (
                        FailureDomain::Environment,
                        FailureKind::NonzeroExit,
                        process.stderr.clone(),
                    )
                }
            }
        };
        if !message.is_empty() {
            diagnostics.push(FailureDiagnostic {
                domain,
                kind,
                component: DiagnosticComponent::Sandbox,
                impact: DiagnosticImpact::Blocking,
                message,
                process: Some(termination.clone()),
                limits: Some(limits.clone()),
            });
        }
    }
    const UNSUPPORTED_ALIAS_PREFIX: &str =
        "court-jester unsupported TypeScript path alias configuration:";
    if let Some(message) = process
        .stderr
        .lines()
        .find(|line| line.contains(UNSUPPORTED_ALIAS_PREFIX))
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ModuleLoad,
            component: DiagnosticComponent::ModuleLoader,
            impact: DiagnosticImpact::Blocking,
            message: message.trim_start_matches("Error: ").trim().to_string(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    let incompatible_native_dependency = if process
        .stderr
        .contains("Prisma Client could not locate the Query Engine for runtime")
        && process.stderr.contains("the actual deployment required")
    {
        Some(
            "project Prisma Client was generated for a platform incompatible with the selected runtime"
                .to_string(),
        )
    } else if process.stderr.contains("esbuild") && process.stderr.contains("another platform") {
        Some(
            "project esbuild binary is incompatible with the selected runtime platform".to_string(),
        )
    } else {
        None
    };
    if let Some(message) = incompatible_native_dependency {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ModuleLoad,
            component: DiagnosticComponent::ModuleLoader,
            impact: DiagnosticImpact::Blocking,
            message,
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    if let Some(message) = process
        .stderr
        .lines()
        .chain(process.stdout.lines())
        .find(|line| {
            line.contains("EACCES")
                && line.contains("permission denied")
                && line.contains(DOCKER_DEPENDENCY_WORKSPACE)
        })
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ModuleLoad,
            component: DiagnosticComponent::ModuleLoader,
            impact: DiagnosticImpact::Blocking,
            message: message.trim_start_matches("Error: ").trim().to_string(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }

    if limits.network_policy == NetworkPolicy::Deny
        && process
            .stderr
            .contains("court-jester network access denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network access was denied by the harness sandbox".into(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    if limits.network_policy == NetworkPolicy::Deny
        && process.stderr.contains("court-jester process spawn denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ProcessSpawnDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "process spawning was denied by the harness sandbox".into(),
            process: process.termination.clone(),
            limits: Some(limits.clone()),
        });
    }
    if process.exit_code == Some(0) {
        return diagnostics;
    }
    let has_non_target_blocker = diagnostics.iter().any(|diagnostic| {
        diagnostic.impact == DiagnosticImpact::Blocking
            && diagnostic.domain != FailureDomain::TargetCode
            && diagnostic.kind != FailureKind::NonzeroExit
    });
    match adapter.unwrap_or(TestAdapter::Opaque) {
        TestAdapter::Opaque => {}
        TestAdapter::NodeTap => {
            if has_non_target_blocker {
                return diagnostics;
            }
            if process.stdout.contains("not ok") || process.stderr.contains("not ok") {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::TargetCode,
                    kind: FailureKind::AssertionFailure,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Gating,
                    message: "authoritative TAP test failed".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            } else if !process.stdout.contains("TAP version") {
                diagnostics.push(FailureDiagnostic {
                    domain: FailureDomain::VerifierHarness,
                    kind: FailureKind::HarnessProtocol,
                    component: DiagnosticComponent::AuthoritativeTestRunner,
                    impact: DiagnosticImpact::Blocking,
                    message: "Node test runner did not emit valid TAP output".into(),
                    process: process.termination.clone(),
                    limits: Some(limits.clone()),
                });
            }
        }
        TestAdapter::BunJunit => {
            if has_non_target_blocker {
                return diagnostics;
            }
            let reported_failure =
                process
                    .stdout
                    .lines()
                    .chain(process.stderr.lines())
                    .any(|line| {
                        let line = line.trim();
                        line.starts_with("(fail)")
                            || line.starts_with("(error)")
                            || [" fail", " error"].iter().any(|suffix| {
                                line.strip_suffix(suffix)
                                    .and_then(|count| count.trim().parse::<usize>().ok())
                                    .is_some_and(|count| count > 0)
                            })
                    });
            let (domain, kind, impact, message) = if reported_failure {
                (
                    FailureDomain::TargetCode,
                    FailureKind::AssertionFailure,
                    DiagnosticImpact::Gating,
                    "authoritative Bun test failed",
                )
            } else {
                (
                    FailureDomain::VerifierHarness,
                    FailureKind::HarnessProtocol,
                    DiagnosticImpact::Blocking,
                    "Bun test runner did not emit a recognized failure result",
                )
            };
            diagnostics.push(FailureDiagnostic {
                domain,
                kind,
                component: DiagnosticComponent::AuthoritativeTestRunner,
                impact,
                message: message.into(),
                process: process.termination.clone(),
                limits: Some(limits.clone()),
            });
        }
        adapter @ (TestAdapter::VitestJson | TestAdapter::JestJson) => {
            if has_non_target_blocker {
                return diagnostics;
            }
            match structured_test_failure(&process.stdout, adapter == TestAdapter::VitestJson) {
                Some(StructuredTestFailure::Initialization(message)) => {
                    diagnostics.push(FailureDiagnostic {
                        domain: FailureDomain::Environment,
                        kind: FailureKind::ModuleLoad,
                        component: DiagnosticComponent::ModuleLoader,
                        impact: DiagnosticImpact::Blocking,
                        message,
                        process: process.termination.clone(),
                        limits: Some(limits.clone()),
                    });
                }
                failure @ (Some(StructuredTestFailure::Assertion)
                | Some(StructuredTestFailure::ProcessSpawnDenied)) => {
                    let spawn_denied = failure == Some(StructuredTestFailure::ProcessSpawnDenied)
                        && limits.network_policy == NetworkPolicy::Deny;
                    diagnostics.push(FailureDiagnostic {
                        domain: if spawn_denied {
                            FailureDomain::Environment
                        } else {
                            FailureDomain::TargetCode
                        },
                        kind: if spawn_denied {
                            FailureKind::ProcessSpawnDenied
                        } else {
                            FailureKind::AssertionFailure
                        },
                        component: if spawn_denied {
                            DiagnosticComponent::Sandbox
                        } else {
                            DiagnosticComponent::AuthoritativeTestRunner
                        },
                        impact: if spawn_denied {
                            DiagnosticImpact::Blocking
                        } else {
                            DiagnosticImpact::Gating
                        },
                        message: if spawn_denied {
                            "process spawning was denied by the harness sandbox".into()
                        } else {
                            "authoritative test assertion failed".into()
                        },
                        process: process.termination.clone(),
                        limits: Some(limits.clone()),
                    });
                }
                None if has_non_target_blocker => return diagnostics,
                None => {
                    diagnostics.push(FailureDiagnostic {
                        domain: FailureDomain::VerifierHarness,
                        kind: FailureKind::HarnessProtocol,
                        component: DiagnosticComponent::AuthoritativeTestRunner,
                        impact: DiagnosticImpact::Blocking,
                        message: "test runner did not emit a recognized structured result".into(),
                        process: process.termination.clone(),
                        limits: Some(limits.clone()),
                    });
                }
            }
        }
    }
    diagnostics
}

#[derive(Debug)]
struct DockerPathMapping {
    project_root: std::path::PathBuf,
    container_artifact: std::path::PathBuf,
    container_cwd: std::path::PathBuf,
}

fn docker_path_mapping(
    source_root: &std::path::Path,
    project_root: &std::path::Path,
    host_artifact: &std::path::Path,
    launch_cwd: &std::path::Path,
) -> Result<DockerPathMapping, String> {
    let source_root = std::fs::canonicalize(source_root)
        .map_err(|error| format!("docker source root is unavailable: {error}"))?;
    std::fs::canonicalize(project_root)
        .map_err(|error| format!("docker project mirror is unavailable: {error}"))?;
    let host_artifact = std::fs::canonicalize(host_artifact)
        .map_err(|error| format!("docker harness artifact is unavailable: {error}"))?;
    let artifact_relative = host_artifact
        .strip_prefix(&source_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "docker harness artifact is outside the project mirror".to_string())?;
    let artifact_relative = normalize_harness_path(artifact_relative)?;
    let launch_cwd = std::fs::canonicalize(launch_cwd)
        .map_err(|error| format!("docker harness working directory is unavailable: {error}"))?;
    let cwd_relative = launch_cwd
        .strip_prefix(&source_root)
        .map_err(|_| "docker harness working directory is outside the project mirror")?;

    Ok(DockerPathMapping {
        project_root: project_root.to_path_buf(),
        container_artifact: std::path::Path::new("/workspace").join(artifact_relative),
        container_cwd: std::path::Path::new("/workspace").join(cwd_relative),
    })
}

const DOCKER_DEPENDENCY_WORKSPACE: &str = "/court-jester/dependencies";

#[derive(Debug, PartialEq, Eq)]
struct DockerDependencyMapping {
    workspace_root: std::path::PathBuf,
    container_roots: Vec<String>,
    node_paths: Vec<String>,
    node_bin_paths: Vec<String>,
}

fn docker_dependency_mapping(
    workspace_root: &std::path::Path,
    dependency_roots: &[std::path::PathBuf],
) -> Result<DockerDependencyMapping, String> {
    let workspace_root = std::fs::canonicalize(workspace_root)
        .map_err(|error| format!("docker dependency workspace is unavailable: {error}"))?;
    let container_workspace = std::path::Path::new(DOCKER_DEPENDENCY_WORKSPACE);
    let mut container_roots = Vec::new();
    let mut node_paths = Vec::new();
    let mut node_bin_paths = Vec::new();

    for dependency in dependency_roots {
        let canonical = match std::fs::canonicalize(dependency) {
            Ok(path) if path.is_dir() => path,
            _ => continue,
        };
        let Ok(relative) = canonical.strip_prefix(&workspace_root) else {
            continue;
        };
        let container_root = if relative.as_os_str().is_empty() {
            container_workspace.to_path_buf()
        } else {
            container_workspace.join(relative)
        };
        let container_root = container_root.to_string_lossy().into_owned();
        if container_roots
            .iter()
            .any(|existing| existing == &container_root)
        {
            continue;
        }
        if canonical.join("node_modules").is_dir() {
            node_paths.push(
                std::path::Path::new(&container_root)
                    .join("node_modules")
                    .to_string_lossy()
                    .into_owned(),
            );
            node_bin_paths.push(
                std::path::Path::new(&container_root)
                    .join("node_modules/.bin")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        container_roots.push(container_root);
    }

    Ok(DockerDependencyMapping {
        workspace_root,
        container_roots,
        node_paths,
        node_bin_paths,
    })
}

fn docker_project_module_path(
    workspace_root: &std::path::Path,
    target_package_root: &std::path::Path,
    dependency_roots: &[std::path::PathBuf],
    package_relative_paths: &[&std::path::Path],
) -> Option<String> {
    let workspace_root = std::fs::canonicalize(workspace_root).ok()?;
    for root in std::iter::once(target_package_root)
        .chain(dependency_roots.iter().map(std::path::PathBuf::as_path))
    {
        let Ok(canonical_root) = std::fs::canonicalize(root) else {
            continue;
        };
        if canonical_root.strip_prefix(&workspace_root).is_err() {
            continue;
        }
        for package_relative_path in package_relative_paths {
            let logical_path = canonical_root
                .join("node_modules")
                .join(package_relative_path);
            let Ok(resolved_path) = std::fs::canonicalize(&logical_path) else {
                continue;
            };
            if !resolved_path.is_file() {
                continue;
            }
            let Ok(resolved_relative) = resolved_path.strip_prefix(&workspace_root) else {
                continue;
            };
            return Some(
                std::path::Path::new(DOCKER_DEPENDENCY_WORKSPACE)
                    .join(resolved_relative)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    None
}

fn insert_docker_environment(create: &mut Vec<String>, value: String) {
    let image_index = create.len() - 1;
    create.splice(image_index..image_index, ["-e".to_string(), value]);
}

fn append_docker_node_preload(create: &mut Vec<String>, preload: &str) {
    let option = format!("--require={preload}");
    for index in 1..create.len() {
        if create[index - 1] == "-e" && create[index].starts_with("NODE_OPTIONS=") {
            create[index].push(' ');
            create[index].push_str(&option);
            return;
        }
    }
    insert_docker_environment(create, format!("NODE_OPTIONS={option}"));
}

fn configure_docker_node_loader(
    runtime: crate::types::HarnessRuntime,
    loader: &str,
    command: &mut Vec<String>,
) -> Option<String> {
    match runtime {
        crate::types::HarnessRuntime::NodeScript | crate::types::HarnessRuntime::NodeTest => {
            command.splice(
                1..1,
                ["--experimental-loader".to_string(), loader.to_string()],
            );
            None
        }
        crate::types::HarnessRuntime::Vitest => {
            command.splice(
                1..1,
                ["--experimental-loader".to_string(), loader.to_string()],
            );
            Some(format!("NODE_OPTIONS=--experimental-loader={loader}"))
        }
        crate::types::HarnessRuntime::TsxScript => {
            Some(format!("NODE_OPTIONS=--experimental-loader={loader}"))
        }
        _ => None,
    }
}

fn configure_docker_typescript_loader(loader: &str, command: &mut Vec<String>) {
    command.splice(
        1..1,
        ["--experimental-loader".to_string(), loader.to_string()],
    );
}

fn native_dependency_load_failed(process: &ExecutionResult) -> bool {
    let output = format!("{}\n{}", process.stderr, process.stdout).to_ascii_lowercase();
    [
        "invalid elf header",
        "exec format error",
        "another platform",
        "could not locate the query engine for runtime",
        "does not provide an export named",
    ]
    .iter()
    .any(|fragment| output.contains(fragment))
}

#[allow(clippy::too_many_arguments)]
async fn run_harness_in_docker(
    temporary_lease: Option<std::sync::Arc<tempfile::TempDir>>,
    host_artifact: &std::path::Path,
    launch_cwd: &std::path::Path,
    context: &crate::types::ExecutionContext,
    harness: &crate::types::HarnessSpec,
    network_guard_lease: Option<std::sync::Arc<NetworkGuard>>,
    limits: crate::types::SandboxOptions<'_>,
    isolated_type_import: Option<&str>,
) -> ExecutionResult {
    let root = temporary_lease.as_ref().map(|directory| directory.path());
    let network_guard = network_guard_lease.as_deref();
    let image = docker_image_for_harness(limits.docker_image.unwrap_or_default(), &harness.runtime);
    let started = Instant::now();
    let remaining = || (limits.timeout_seconds - started.elapsed().as_secs_f64()).max(0.0);
    let image_check =
        docker_output(&["image", "inspect", image], remaining(), limits.memory_mb).await;
    if !docker_command_succeeded(&image_check) {
        return docker_lifecycle_failure("image inspect", image_check, None);
    }

    let mirror = if root.is_none() {
        match runtime_tempdir(crate::types::RuntimeProfile::Isolated) {
            Ok(directory) => Some(directory),
            Err(error) => {
                return launch_failure(format!(
                    "docker setup failed creating project mirror: {error}"
                ));
            }
        }
    } else {
        None
    };
    if let Some(mirror) = mirror.as_ref() {
        if let Err(error) = copy_materialization_tree(
            &context.workspace_root,
            mirror.path(),
            context.materialization_source_root.as_deref(),
        ) {
            return launch_failure(format!(
                "docker setup failed materializing project mirror: {error}"
            ));
        }
    }
    let source_root = root.unwrap_or(&context.workspace_root);
    let project_root = root
        .or_else(|| mirror.as_ref().map(|directory| directory.path()))
        .unwrap_or(source_root);
    let mapping = match docker_path_mapping(source_root, project_root, host_artifact, launch_cwd) {
        Ok(mapping) => mapping,
        Err(error) => return launch_failure(error),
    };
    let project_root = mapping.project_root;
    let container_artifact = mapping.container_artifact;
    let container_cwd = mapping.container_cwd;

    let container = format!(
        "court-jester-harness-{}-{}",
        std::process::id(),
        started.elapsed().as_nanos()
    );
    let runtime_user = docker_runtime_user(
        context
            .dependency_roots
            .iter()
            .any(|root| root.join("node_modules").is_dir()),
    );
    let mut create = vec![
        "create".to_string(),
        "--name".to_string(),
        container.clone(),
        "--pull=never".to_string(),
        "--network".to_string(),
        "none".to_string(),
        "--cpus".to_string(),
        "1".to_string(),
        "--memory".to_string(),
        format!("{}m", limits.memory_mb),
        "--memory-swap".to_string(),
        format!("{}m", limits.memory_mb),
        "--pids-limit".to_string(),
        "128".to_string(),
        "--read-only".to_string(),
        "--tmpfs".to_string(),
        "/tmp:rw,nosuid,nodev,noexec,size=64m".to_string(),
        "--ulimit".to_string(),
        "fsize=10485760:10485760".to_string(),
        "--user".to_string(),
        runtime_user,
        "-e".to_string(),
        "HOME=/tmp".to_string(),
        "-e".to_string(),
        "PYTHONDONTWRITEBYTECODE=1".to_string(),
        "--mount".to_string(),
        format!(
            "type=bind,src={},dst=/workspace,readonly",
            project_root.display()
        ),
        "--workdir".to_string(),
        container_cwd.to_string_lossy().into_owned(),
        image.to_string(),
    ];

    let dependency_workspace = context
        .materialization_source_root
        .as_deref()
        .unwrap_or(&context.workspace_root);
    let dependency_mapping =
        match docker_dependency_mapping(dependency_workspace, &context.dependency_roots) {
            Ok(mapping) => mapping,
            Err(error) => return launch_failure(error),
        };
    let resolver_roots =
        node_package_resolver_roots(&context.target_package_root, &context.dependency_roots)
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
    let resolver_mapping = match docker_dependency_mapping(dependency_workspace, &resolver_roots) {
        Ok(mapping) => mapping,
        Err(error) => return launch_failure(error),
    };
    #[cfg(unix)]
    {
        let mut dependency_links = vec![(
            dependency_workspace.join("node_modules"),
            project_root.join("node_modules"),
            std::path::Path::new(DOCKER_DEPENDENCY_WORKSPACE).join("node_modules"),
        )];
        if let Some(target_relative) = context
            .target_package_root
            .strip_prefix(dependency_workspace)
            .ok()
            .or_else(|| {
                context
                    .target_package_root
                    .strip_prefix(&context.workspace_root)
                    .ok()
            })
            .filter(|relative| !relative.as_os_str().is_empty())
        {
            dependency_links.push((
                dependency_workspace
                    .join(target_relative)
                    .join("node_modules"),
                project_root.join(target_relative).join("node_modules"),
                std::path::Path::new(DOCKER_DEPENDENCY_WORKSPACE)
                    .join(target_relative)
                    .join("node_modules"),
            ));
        }
        for (host_source, host_link, container_target) in dependency_links {
            if !host_source.is_dir() || std::fs::symlink_metadata(&host_link).is_ok() {
                continue;
            }
            if let Some(parent) = host_link.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    return launch_failure(format!(
                        "docker setup failed preparing project dependency link: {error}"
                    ));
                }
            }
            if let Err(error) = std::os::unix::fs::symlink(container_target, &host_link) {
                return launch_failure(format!(
                    "docker setup failed linking project dependencies: {error}"
                ));
            }
        }
    }
    let portable_typescript = docker_project_module_path(
        dependency_workspace,
        &context.target_package_root,
        &context.dependency_roots,
        &[std::path::Path::new("typescript/lib/typescript.js")],
    );
    let portable_vitest = if harness.runtime == crate::types::HarnessRuntime::Vitest {
        docker_project_module_path(
            dependency_workspace,
            &context.target_package_root,
            &context.dependency_roots,
            &[
                std::path::Path::new("vitest/dist/node.mjs"),
                std::path::Path::new("vitest/dist/node.js"),
            ],
        )
        .zip(portable_typescript.clone())
    } else {
        None
    };
    let use_portable_typescript = portable_typescript.is_some()
        && (matches!(
            harness.runtime,
            crate::types::HarnessRuntime::NodeScript | crate::types::HarnessRuntime::NodeTest
        ) || portable_vitest.is_some());
    let python_paths = dependency_mapping.container_roots;
    let node_paths = dependency_mapping.node_paths;
    let node_bin_paths = dependency_mapping.node_bin_paths;
    let container_node_resolver_roots = resolver_mapping.container_roots;
    if !python_paths.is_empty() || !container_node_resolver_roots.is_empty() {
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst={},readonly",
                dependency_mapping.workspace_root.display(),
                DOCKER_DEPENDENCY_WORKSPACE
            ),
        );
    }
    if !python_paths.is_empty() {
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!("PYTHONPATH={}", python_paths.join(":")),
        );
    }
    if !node_paths.is_empty() {
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!("NODE_PATH={}", node_paths.join(":")),
        );
    }
    if !node_bin_paths.is_empty() {
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "PATH={}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                node_bin_paths.join(":")
            ),
        );
    }
    if portable_vitest.is_some() {
        let Some(guard) = network_guard else {
            return launch_failure("portable Vitest execution requires the runtime guard");
        };
        for (source, destination) in [
            (
                &guard.portable_vitest_coordinator,
                "/court-jester/portable-vitest-coordinator.mjs",
            ),
            (&guard.node_preload, "/court-jester/network-guard.cjs"),
            (
                &guard.instrumentation_preload,
                "/court-jester/instrumentation-preload.cjs",
            ),
        ] {
            create.insert(create.len() - 1, "--mount".to_string());
            create.insert(
                create.len() - 1,
                format!(
                    "type=bind,src={},dst={destination},readonly",
                    source.display()
                ),
            );
        }
    }
    if limits.instrumented_source.is_some() && portable_vitest.is_none() {
        let Some(guard) = network_guard else {
            return launch_failure("instrumented execution requires the runtime guard");
        };
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst=/court-jester/instrumentation-preload.cjs,readonly",
                guard.instrumentation_preload.display()
            ),
        );
    }
    if let (Some(target), Some(payload)) = (
        limits.instrumentation_target,
        network_guard.and_then(|guard| guard.instrumented_source.as_ref()),
    ) {
        let target = match std::fs::canonicalize(target) {
            Ok(target) => target,
            Err(error) => {
                return launch_failure(format!("instrumentation target is unavailable: {error}"));
            }
        };
        let workspace = std::fs::canonicalize(&context.workspace_root)
            .unwrap_or_else(|_| context.workspace_root.clone());
        let Some(relative) = target
            .strip_prefix(workspace)
            .ok()
            .filter(|relative| !relative.as_os_str().is_empty())
        else {
            return launch_failure("instrumentation target is outside the workspace");
        };
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst=/court-jester/instrumented-target.ts,readonly",
                payload.display()
            ),
        );
        for value in [
            format!(
                "COURT_JESTER_INSTRUMENT_TARGET={}",
                std::path::Path::new("/workspace").join(relative).display()
            ),
            "COURT_JESTER_INSTRUMENT_PAYLOAD=/court-jester/instrumented-target.ts".to_string(),
        ] {
            create.insert(create.len() - 1, "-e".to_string());
            create.insert(create.len() - 1, value);
        }
    }
    if use_portable_typescript {
        let Some(guard) = network_guard else {
            return launch_failure("portable TypeScript execution requires the runtime guard");
        };
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst=/court-jester/typescript-loader.mjs,readonly",
                guard.typescript_loader.display()
            ),
        );
        create.insert(create.len() - 1, "-e".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "COURT_JESTER_TYPESCRIPT_MODULE={}",
                portable_typescript.as_deref().unwrap_or_default()
            ),
        );
    }

    let virtual_type_imports = isolated_type_import.map(str::to_owned);
    let node_package_resolver = if (matches!(
        harness.runtime,
        crate::types::HarnessRuntime::NodeScript
            | crate::types::HarnessRuntime::NodeTest
            | crate::types::HarnessRuntime::TsxScript
    ) || portable_vitest.is_some())
        && harness.kind != crate::types::HarnessKind::PortabilityProbe
        && !container_node_resolver_roots.is_empty()
    {
        match create_node_package_resolver_with_virtual_type_imports(
            crate::types::RuntimeProfile::Isolated,
            virtual_type_imports.as_deref(),
        ) {
            Ok(resolver) => Some(resolver),
            Err(error) => return launch_failure(error),
        }
    } else {
        None
    };
    let container_package_resolver = "/court-jester/package-resolver.mjs";
    if let Some(resolver) = node_package_resolver.as_ref() {
        create.insert(create.len() - 1, "--mount".to_string());
        create.insert(
            create.len() - 1,
            format!(
                "type=bind,src={},dst={},readonly",
                resolver.loader.display(),
                container_package_resolver
            ),
        );
        let target_relative = context
            .target_package_root
            .strip_prefix(&context.workspace_root)
            .unwrap_or_else(|_| std::path::Path::new(""));
        let container_source_root = std::path::Path::new(DOCKER_DEPENDENCY_WORKSPACE);
        let resolver_environment = [
            format!(
                "COURT_JESTER_NODE_RESOLUTION_MODE={}",
                if root.is_some() {
                    "generated"
                } else {
                    "existing"
                }
            ),
            "COURT_JESTER_NODE_OVERLAY_ROOT=/workspace".to_string(),
            format!(
                "COURT_JESTER_NODE_SOURCE_ROOT={}",
                container_source_root.display()
            ),
            format!(
                "COURT_JESTER_NODE_TARGET_ROOT={}",
                container_source_root.join(target_relative).display()
            ),
            format!(
                "COURT_JESTER_NODE_OVERLAY_TARGET_ROOT={}",
                std::path::Path::new("/workspace")
                    .join(target_relative)
                    .display()
            ),
            format!(
                "COURT_JESTER_NODE_GENERATED_ARTIFACT={}",
                container_artifact.display()
            ),
        ];
        for value in resolver_environment {
            insert_docker_environment(&mut create, value);
        }
        if virtual_type_imports.is_some()
            && root.is_some()
            && harness.kind == crate::types::HarnessKind::GeneratedVerifier
        {
            let Some(source_file) = context.target_source.source_file.as_deref() else {
                return launch_failure("generated target has no source path");
            };
            if source_file.strip_prefix(&context.workspace_root).is_err() {
                return launch_failure(
                    "target source is outside the workspace dependency boundary",
                );
            }
        }
    }

    let artifact = container_artifact.to_string_lossy().into_owned();
    let mut command = match harness.runtime {
        crate::types::HarnessRuntime::Python => vec!["python3".to_string(), artifact],
        crate::types::HarnessRuntime::NodeScript => vec![
            "node".to_string(),
            "--no-warnings".to_string(),
            "--experimental-transform-types".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::TsxScript => vec!["tsx".to_string(), artifact],
        crate::types::HarnessRuntime::BunScript => {
            vec!["bun".to_string(), "run".to_string(), artifact]
        }
        crate::types::HarnessRuntime::NodeTest => vec![
            "node".to_string(),
            "--no-warnings".to_string(),
            "--experimental-transform-types".to_string(),
            "--test-reporter=tap".to_string(),
            "--test".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::BunTest => {
            vec!["bun".to_string(), "test".to_string(), artifact]
        }
        crate::types::HarnessRuntime::Vitest => {
            if let Some((vitest_module, typescript_module)) = portable_vitest.as_ref() {
                vec![
                    "node".to_string(),
                    "/court-jester/portable-vitest-coordinator.mjs".to_string(),
                    vitest_module.clone(),
                    typescript_module.clone(),
                    artifact,
                    "/court-jester/network-guard.cjs".to_string(),
                    "/court-jester/instrumentation-preload.cjs".to_string(),
                ]
            } else {
                vec![
                    "vitest".to_string(),
                    "run".to_string(),
                    "--reporter=json".to_string(),
                    artifact,
                ]
            }
        }
        crate::types::HarnessRuntime::Jest => {
            vec!["jest".to_string(), "--json".to_string(), artifact]
        }
        crate::types::HarnessRuntime::RepoTest => vec![
            "npm".to_string(),
            "test".to_string(),
            "--".to_string(),
            artifact,
        ],
        crate::types::HarnessRuntime::Jazzer => vec!["jazzer".to_string(), artifact],
    };
    if limits.instrumented_source.is_some() {
        let preload_index = match harness.runtime {
            crate::types::HarnessRuntime::BunScript | crate::types::HarnessRuntime::BunTest => {
                Some(2)
            }
            _ => None,
        };
        if let Some(index) = preload_index {
            command.splice(
                index..index,
                [
                    "--preload".to_string(),
                    "/court-jester/instrumentation-preload.cjs".to_string(),
                ],
            );
        }
    }
    if node_package_resolver.is_some() {
        if let Some(node_options) =
            configure_docker_node_loader(harness.runtime, container_package_resolver, &mut command)
        {
            insert_docker_environment(&mut create, node_options);
        }
    }
    if use_portable_typescript {
        configure_docker_typescript_loader("/court-jester/typescript-loader.mjs", &mut command);
    }
    if limits.instrumented_source.is_some()
        && portable_vitest.is_none()
        && !matches!(
            harness.runtime,
            crate::types::HarnessRuntime::Python
                | crate::types::HarnessRuntime::BunScript
                | crate::types::HarnessRuntime::BunTest
        )
    {
        append_docker_node_preload(&mut create, "/court-jester/instrumentation-preload.cjs");
    }
    for argument in harness.args.iter().chain(limits.harness_args.iter()) {
        match argument {
            crate::types::HarnessArg::Literal { literal } => {
                if literal.contains('\0') {
                    return launch_failure("harness literal argument contains NUL");
                }
                command.push(literal.clone());
            }
            crate::types::HarnessArg::ProjectPath { project_path } => {
                let relative = match normalize_harness_path(std::path::Path::new(project_path)) {
                    Ok(path) => path,
                    Err(error) => return launch_failure(error),
                };
                command.push(
                    std::path::Path::new("/workspace")
                        .join(relative)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    create.extend(command);
    let lifecycle = DockerLifecycle {
        program: which_binary(&std::env::var("PATH").unwrap_or_default(), "docker")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "docker".into()),
        create,
        container,
        started,
        limits: ExecutionLimits {
            timeout_seconds: limits.timeout_seconds,
            memory_mb: limits.memory_mb,
            runtime_profile: limits.runtime_profile,
            network_policy: NetworkPolicy::Deny,
        },
        adapter: harness.test_adapter,
        _workspace: mirror,
        _generated_workspace: temporary_lease,
        _runtime_guard: network_guard_lease,
        _resolver: node_package_resolver,
    };
    supervise_docker_lifecycle(lifecycle).await
}

fn virtual_env_bin(virtual_env: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    virtual_env.map(|root| {
        std::path::PathBuf::from(root).join(if cfg!(windows) { "Scripts" } else { "bin" })
    })
}

fn project_python_available(context: &ExecutionContext) -> bool {
    context.dependency_roots.iter().any(|root| {
        [".venv", "venv"].iter().any(|directory| {
            let bin = root
                .join(directory)
                .join(if cfg!(windows) { "Scripts" } else { "bin" });
            let executable = if cfg!(windows) {
                bin.join("python.exe")
            } else {
                bin.join("python3")
            };
            executable.is_file()
        })
    })
}

fn uv_python_runtime(
    path_env: &str,
    context: &ExecutionContext,
    active_virtual_env: bool,
) -> Option<(std::path::PathBuf, Vec<std::ffi::OsString>)> {
    if active_virtual_env || project_python_available(context) {
        return None;
    }
    let project_root = std::iter::once(&context.target_package_root)
        .chain(context.dependency_roots.iter())
        .find(|root| root.join("pyproject.toml").is_file() && root.join("uv.lock").is_file())?;
    let uv = which_binary(path_env, "uv")?;
    Some((
        uv.into(),
        vec![
            "run".into(),
            "--isolated".into(),
            "--frozen".into(),
            "--offline".into(),
            "--project".into(),
            project_root.as_os_str().to_owned(),
            "python3".into(),
        ],
    ))
}

pub async fn execute_harness(
    context: &ExecutionContext,
    harness: HarnessSpec,
    limits: SandboxOptions<'_>,
) -> HarnessExecution {
    if let Err(error) = limits.validate() {
        let process = launch_failure(error);
        return HarnessExecution {
            diagnostics: process.diagnostics.clone(),
            process,
        };
    }
    let effective_network =
        if limits.network_policy == NetworkPolicy::Deny || harness.network == NetworkPolicy::Deny {
            NetworkPolicy::Deny
        } else {
            NetworkPolicy::Allow
        };
    if limits.runtime_profile == RuntimeProfile::Isolated
        && effective_network == NetworkPolicy::Allow
    {
        let process = launch_failure("isolated harnesses cannot enable network access");
        let diagnostic = FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network allow is incompatible with isolated execution".into(),
            process: None,
            limits: Some(ExecutionLimits {
                timeout_seconds: limits.timeout_seconds,
                memory_mb: limits.memory_mb,
                runtime_profile: limits.runtime_profile,
                network_policy: effective_network,
            }),
        };
        return HarnessExecution {
            process,
            diagnostics: vec![diagnostic],
        };
    }

    let (temporary, host_artifact, mut launch_cwd) = match harness.artifact.clone() {
        HarnessArtifact::Generated {
            code,
            relative_path,
        } => {
            let relative_path = match normalize_harness_path(&relative_path) {
                Ok(path) => path,
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            if !harness_extension_compatible(&relative_path, harness.source_mode) {
                let process =
                    launch_failure("harness artifact extension is incompatible with source mode");
                return HarnessExecution {
                    diagnostics: vec![FailureDiagnostic {
                        domain: FailureDomain::VerifierHarness,
                        kind: FailureKind::UnsupportedSourceMode,
                        component: DiagnosticComponent::FuzzHarness,
                        impact: DiagnosticImpact::Blocking,
                        message: "harness artifact extension is incompatible with source mode"
                            .into(),
                        process: None,
                        limits: None,
                    }],
                    process,
                };
            }
            let temporary = match runtime_tempdir(limits.runtime_profile) {
                Ok(directory) => directory,
                Err(error) => {
                    let process =
                        launch_failure(format!("failed to create harness overlay: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            let package_overlay = if harness.source_mode == SourceMode::Python
                && context.workspace_root.join("__init__.py").is_file()
            {
                context
                    .workspace_root
                    .file_name()
                    .map(|name| name.to_os_string())
                    .filter(|name| is_valid_python_module_name(&name.to_string_lossy()))
            } else {
                None
            };
            let overlay_root = package_overlay
                .as_ref()
                .map(|name| temporary.path().join(name))
                .unwrap_or_else(|| temporary.path().to_path_buf());
            let should_materialize = limits
                .source_file
                .is_some_and(|path| std::path::Path::new(path).is_file())
                || (harness.kind == HarnessKind::Standalone && limits.project_dir.is_some());
            if should_materialize {
                if let Err(error) = copy_materialization_tree(
                    &context.workspace_root,
                    &overlay_root,
                    context.materialization_source_root.as_deref(),
                ) {
                    let process =
                        launch_failure(format!("failed to materialize project overlay: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            }
            let host_artifact = overlay_root.join(&relative_path);
            if let Some(parent) = host_artifact.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    let process =
                        launch_failure(format!("failed to create harness directory: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            }
            let code = match harness.source_mode {
                crate::types::SourceMode::TypeScript | crate::types::SourceMode::Tsx => {
                    resolve_generated_typescript_relative_imports(
                        &code,
                        context.target_source.source_file.as_deref(),
                    )
                }
                crate::types::SourceMode::Python => code,
            };
            if let Err(error) = std::fs::write(&host_artifact, code) {
                let process = launch_failure(format!("failed to materialize harness: {error}"));
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            let package_relative = context
                .target_package_root
                .strip_prefix(&context.workspace_root)
                .unwrap_or_else(|_| std::path::Path::new(""));
            let launch_cwd = if package_overlay.is_some() {
                temporary.path().to_path_buf()
            } else {
                temporary.path().join(package_relative)
            };
            if let Err(error) = std::fs::create_dir_all(&launch_cwd) {
                let process = launch_failure(format!(
                    "failed to create harness working directory: {error}"
                ));
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            (
                Some(std::sync::Arc::new(temporary)),
                host_artifact,
                launch_cwd,
            )
        }
        HarnessArtifact::Existing { relative_path } => {
            let relative_path = match normalize_harness_path(&relative_path) {
                Ok(path) => path,
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            if !harness_extension_compatible(&relative_path, harness.source_mode) {
                let process =
                    launch_failure("existing artifact extension is incompatible with source mode");
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
            let host_artifact = context.workspace_root.join(&relative_path);
            let workspace_root = std::fs::canonicalize(&context.workspace_root)
                .unwrap_or_else(|_| context.workspace_root.clone());
            let canonical = match std::fs::canonicalize(&host_artifact) {
                Ok(path) if path.starts_with(&workspace_root) => path,
                Ok(_) => {
                    let process = launch_failure("existing harness escapes workspace root");
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
                Err(error) => {
                    let process =
                        launch_failure(format!("existing harness is unavailable: {error}"));
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            (None, canonical, context.target_package_root.clone())
        }
    };

    let path_env = {
        let base = std::env::var("PATH").unwrap_or_default();
        let mut prefixes = Vec::new();
        if let Some(bin) = virtual_env_bin(std::env::var_os("VIRTUAL_ENV").as_deref()) {
            prefixes.push(bin.to_string_lossy().into_owned());
        }
        for root in &context.dependency_roots {
            prefixes.push(
                root.join("node_modules/.bin")
                    .to_string_lossy()
                    .into_owned(),
            );
            prefixes.push(root.join(".venv/bin").to_string_lossy().into_owned());
        }
        if prefixes.is_empty() {
            base
        } else if base.is_empty() {
            prefixes.join(":")
        } else {
            format!("{}:{}", prefixes.join(":"), base)
        }
    };
    let executable_name = match harness.runtime {
        HarnessRuntime::Python => "python3",
        HarnessRuntime::NodeScript | HarnessRuntime::NodeTest | HarnessRuntime::TsxScript => {
            if harness.runtime == HarnessRuntime::TsxScript {
                "tsx"
            } else {
                "node"
            }
        }
        HarnessRuntime::BunScript | HarnessRuntime::BunTest => "bun",
        HarnessRuntime::Vitest => "vitest",
        HarnessRuntime::Jest => "jest",
        HarnessRuntime::RepoTest => "npm",
        HarnessRuntime::Jazzer => "jazzer",
    };
    let mut executable = if limits.runtime_profile == RuntimeProfile::Isolated
        && harness.runtime != HarnessRuntime::Vitest
    {
        std::path::PathBuf::from(executable_name)
    } else {
        match which_binary(&path_env, executable_name) {
            Some(path) => std::path::PathBuf::from(path),
            None => {
                let process = launch_failure(format!(
                    "required runtime '{executable_name}' is unavailable"
                ));
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
        }
    };
    let mut runtime_prefix_args = Vec::new();
    if limits.runtime_profile == RuntimeProfile::LocalTrusted
        && harness.runtime == HarnessRuntime::Python
    {
        if let Some((uv, prefix_args)) = uv_python_runtime(
            &path_env,
            context,
            std::env::var_os("VIRTUAL_ENV").is_some(),
        ) {
            executable = uv;
            runtime_prefix_args = prefix_args;
        }
    }
    let mut args = runtime_prefix_args;
    match harness.runtime {
        HarnessRuntime::Python => {}
        HarnessRuntime::NodeScript | HarnessRuntime::TsxScript => {
            if harness.runtime == HarnessRuntime::NodeScript {
                args.extend(["--no-warnings", "--experimental-transform-types"].map(Into::into));
            }
        }
        HarnessRuntime::BunScript => {}
        HarnessRuntime::NodeTest => {
            args.extend(
                [
                    "--no-warnings",
                    "--experimental-transform-types",
                    "--test-reporter=tap",
                    "--test",
                ]
                .map(Into::into),
            );
        }
        HarnessRuntime::BunTest => args.push("test".into()),
        HarnessRuntime::Vitest => args.extend(["run", "--reporter=json"].map(Into::into)),
        HarnessRuntime::Jest => args.extend(["--json"].map(Into::into)),
        HarnessRuntime::RepoTest => args.extend(["test", "--"].map(Into::into)),
        HarnessRuntime::Jazzer => {}
    }
    let python_module = matches!(harness.runtime, HarnessRuntime::Python)
        .then(|| module_run_for_python_source(&host_artifact))
        .flatten();
    if let Some((module_root, module_name)) = python_module {
        launch_cwd = module_root;
        args.extend(["-m".into(), module_name.into()]);
    } else {
        args.push(host_artifact.clone().into_os_string());
    }
    for argument in harness.args.iter().chain(limits.harness_args.iter()) {
        match resolve_harness_arg(argument, &context.workspace_root) {
            Ok(value) => args.push(value),
            Err(error) => {
                let process = launch_failure(error);
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
        }
    }
    let mut env = Vec::new();
    env.push(("PATH".into(), path_env.clone()));
    let dependency_roots = context
        .dependency_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let node_paths = node_dependency_paths(&context.dependency_roots);
    if !node_paths.is_empty() {
        env.push((
            "NODE_PATH".into(),
            node_paths.join(if cfg!(windows) { ";" } else { ":" }),
        ));
    }
    if !dependency_roots.is_empty() {
        env.push((
            "PYTHONPATH".into(),
            dependency_roots.join(if cfg!(windows) { ";" } else { ":" }),
        ));
    }
    let node_package_resolver = if matches!(
        harness.runtime,
        HarnessRuntime::NodeScript | HarnessRuntime::NodeTest | HarnessRuntime::TsxScript
    ) && harness.kind != HarnessKind::PortabilityProbe
    {
        match create_node_package_resolver(limits.runtime_profile) {
            Ok(resolver) => {
                let package_relative = context
                    .target_package_root
                    .strip_prefix(&context.workspace_root)
                    .unwrap_or_else(|_| std::path::Path::new(""));
                let overlay_root = temporary
                    .as_ref()
                    .map(|directory| directory.path())
                    .unwrap_or(context.workspace_root.as_path());
                let overlay_target_root = if temporary.is_some() {
                    overlay_root.join(package_relative)
                } else {
                    context.target_package_root.clone()
                };
                for (name, value) in [
                    (
                        "COURT_JESTER_NODE_RESOLUTION_MODE",
                        if temporary.is_some() {
                            "generated"
                        } else {
                            "existing"
                        }
                        .to_string(),
                    ),
                    (
                        "COURT_JESTER_NODE_OVERLAY_ROOT",
                        overlay_root.to_string_lossy().into_owned(),
                    ),
                    (
                        "COURT_JESTER_NODE_SOURCE_ROOT",
                        context.workspace_root.to_string_lossy().into_owned(),
                    ),
                    (
                        "COURT_JESTER_NODE_TARGET_ROOT",
                        context.target_package_root.to_string_lossy().into_owned(),
                    ),
                    (
                        "COURT_JESTER_NODE_OVERLAY_TARGET_ROOT",
                        overlay_target_root.to_string_lossy().into_owned(),
                    ),
                    (
                        "COURT_JESTER_NODE_GENERATED_ARTIFACT",
                        host_artifact.to_string_lossy().into_owned(),
                    ),
                ] {
                    env.push((name.into(), value));
                }
                Some(resolver)
            }
            Err(error) => {
                let process = launch_failure(error);
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            }
        }
    } else {
        None
    };
    if let Some(resolver) = node_package_resolver.as_ref() {
        args.splice(
            0..0,
            [
                std::ffi::OsString::from("--experimental-loader"),
                resolver.loader.clone().into_os_string(),
            ],
        );
    }
    let runtime_guard =
        if effective_network == NetworkPolicy::Deny || limits.instrumented_source.is_some() {
            match create_network_guard(limits.runtime_profile, limits.instrumented_source) {
                Ok(guard) => Some(std::sync::Arc::new(guard)),
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            }
        } else {
            None
        };
    if let Some(guard) = runtime_guard.as_ref() {
        if effective_network == NetworkPolicy::Deny {
            match harness.source_mode {
                SourceMode::Python => apply_network_guard(&mut env, &Language::Python, guard),
                SourceMode::TypeScript | SourceMode::Tsx
                    if harness.runtime != HarnessRuntime::Vitest =>
                {
                    apply_network_guard(&mut env, &Language::TypeScript, guard)
                }
                SourceMode::TypeScript | SourceMode::Tsx => {}
            }
        }
        if let (Some(target), Some(payload)) = (
            limits.instrumentation_target,
            guard.instrumented_source.as_ref(),
        ) {
            env.push(("COURT_JESTER_INSTRUMENT_TARGET".into(), target.into()));
            env.push((
                "COURT_JESTER_INSTRUMENT_PAYLOAD".into(),
                payload.to_string_lossy().into_owned(),
            ));
            if !(matches!(
                harness.runtime,
                HarnessRuntime::BunScript | HarnessRuntime::BunTest
            ) || harness.runtime == HarnessRuntime::Vitest
                && effective_network == NetworkPolicy::Deny)
            {
                let preload = format!("--require={}", guard.instrumentation_preload.display());
                if let Some((_, value)) = env.iter_mut().find(|(name, _)| name == "NODE_OPTIONS") {
                    *value = format!("{preload} {value}");
                } else {
                    env.push(("NODE_OPTIONS".into(), preload));
                }
            }
        }
        let bun_preload_index = match harness.runtime {
            HarnessRuntime::BunScript => Some(0),
            HarnessRuntime::BunTest => Some(1),
            _ => None,
        };
        if let Some(index) = bun_preload_index {
            let mut preloads = Vec::new();
            if effective_network == NetworkPolicy::Deny {
                preloads.extend([
                    std::ffi::OsString::from("--preload"),
                    guard.node_preload.clone().into_os_string(),
                ]);
            }
            if limits.instrumented_source.is_some() {
                preloads.extend([
                    std::ffi::OsString::from("--preload"),
                    guard.instrumentation_preload.clone().into_os_string(),
                ]);
            }
            args.splice(index..index, preloads);
            env.retain(|(name, _)| name != "NODE_OPTIONS");
        }
        if harness.runtime == HarnessRuntime::Vitest && effective_network == NetworkPolicy::Allow {
            if let Ok((_, legacy_threads)) = vitest_project_entrypoint(&executable) {
                if legacy_threads {
                    args.splice(
                        2..2,
                        [
                            std::ffi::OsString::from("--threads"),
                            std::ffi::OsString::from("false"),
                        ],
                    );
                } else {
                    args.splice(
                        2..2,
                        [
                            std::ffi::OsString::from("--pool=forks"),
                            std::ffi::OsString::from("--maxWorkers=1"),
                            std::ffi::OsString::from("--minWorkers=1"),
                        ],
                    );
                }
            }
        }
        if harness.runtime == HarnessRuntime::Vitest && effective_network == NetworkPolicy::Deny {
            let (vitest_entrypoint, legacy_threads) = match vitest_project_entrypoint(&executable) {
                Ok(package) => package,
                Err(error) => {
                    let process = launch_failure(error);
                    return HarnessExecution {
                        diagnostics: process.diagnostics.clone(),
                        process,
                    };
                }
            };
            let Some(node) = which_binary(&path_env, "node") else {
                let process = launch_failure(
                    "required runtime 'node' is unavailable for the Vitest coordinator",
                );
                return HarnessExecution {
                    diagnostics: process.diagnostics.clone(),
                    process,
                };
            };
            executable = node.into();
            if legacy_threads {
                args.splice(
                    2..2,
                    [
                        std::ffi::OsString::from("--threads"),
                        std::ffi::OsString::from("false"),
                    ],
                );
            } else {
                args.splice(
                    2..2,
                    [
                        std::ffi::OsString::from("--pool=forks"),
                        std::ffi::OsString::from("--maxWorkers=1"),
                        std::ffi::OsString::from("--minWorkers=1"),
                    ],
                );
            }
            args.splice(
                0..0,
                [
                    guard.vitest_coordinator.clone().into_os_string(),
                    vitest_entrypoint.into_os_string(),
                    guard.node_preload.clone().into_os_string(),
                    guard.instrumentation_preload.clone().into_os_string(),
                ],
            );
        }
    }
    let plan = LaunchPlan {
        executable,
        args,
        cwd: launch_cwd,
        env: env
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
        host_artifact,
        container_artifact: None,
    };
    let execution_limits = ExecutionLimits {
        timeout_seconds: limits.timeout_seconds,
        memory_mb: limits.memory_mb,
        runtime_profile: limits.runtime_profile,
        network_policy: effective_network,
    };
    let is_typescript = !matches!(harness.source_mode, SourceMode::Python);
    let mut process = if limits.runtime_profile == RuntimeProfile::Isolated {
        let mut isolated = run_harness_in_docker(
            temporary.clone(),
            &plan.host_artifact,
            &plan.cwd,
            context,
            &harness,
            runtime_guard.clone(),
            limits,
            None,
        )
        .await;
        if harness.kind == HarnessKind::GeneratedVerifier
            && is_typescript
            && native_dependency_load_failed(&isolated)
        {
            let Some(target_file) = context.target_source.source_file.as_deref() else {
                return HarnessExecution {
                    process: isolated,
                    diagnostics: Vec::new(),
                };
            };
            let Some(target_relative) =
                target_file
                    .strip_prefix(&context.workspace_root)
                    .ok()
                    .map(|path| {
                        path.to_string_lossy()
                            .replace(std::path::MAIN_SEPARATOR, "/")
                    })
            else {
                return HarnessExecution {
                    process: isolated,
                    diagnostics: Vec::new(),
                };
            };
            let mut candidates = isolated_virtual_type_import_candidates(context);
            if let Some(package_name) = isolated_failure_package_name(context, &isolated) {
                if candidates.contains_key(&package_name) {
                    candidates.retain(|specifier, _| specifier == &package_name);
                }
            }
            let target_redirects = isolated_typescript_barrel_redirects(context);
            let has_redirects = !target_redirects.is_empty();
            let import_redirects = BTreeMap::from([(target_relative.clone(), target_redirects)]);
            let mut retry_candidates = candidates.into_iter().map(Some).collect::<Vec<_>>();
            if has_redirects {
                retry_candidates.insert(0, None);
            }
            'candidate: for candidate in retry_candidates {
                let mut virtual_imports = BTreeMap::new();
                if let Some((specifier, names)) = candidate {
                    virtual_imports.insert(
                        target_relative.clone(),
                        BTreeMap::from([(specifier, names)]),
                    );
                }
                for _ in 0..32 {
                    let configuration = serde_json::json!({
                        "removals": &virtual_imports,
                        "redirects": &import_redirects,
                    });
                    let Some(serialized) = serde_json::to_string(&configuration).ok() else {
                        break;
                    };
                    let retry = run_harness_in_docker(
                        temporary.clone(),
                        &plan.host_artifact,
                        &plan.cwd,
                        context,
                        &harness,
                        runtime_guard.clone(),
                        limits,
                        Some(&serialized),
                    )
                    .await;
                    if !native_dependency_load_failed(&retry) {
                        isolated = retry;
                        break 'candidate;
                    }
                    let Some((importer, specifier, names)) =
                        isolated_virtual_type_import_candidate_from_failure(
                            context,
                            &retry,
                            &virtual_imports,
                        )
                    else {
                        break;
                    };
                    virtual_imports
                        .entry(importer)
                        .or_default()
                        .insert(specifier, names);
                }
            }
        }
        isolated
    } else {
        run_launch_command(
            &plan,
            limits.timeout_seconds,
            limits.memory_mb,
            limits.runtime_profile,
            effective_network,
            is_typescript,
        )
        .await
    };
    let mut diagnostics = harness_diagnostics(harness.test_adapter, &process, &execution_limits);
    if matches!(harness.kind, HarnessKind::GeneratedVerifier)
        && process
            .termination
            .as_ref()
            .is_some_and(|termination| matches!(termination.kind, ProcessTerminationKind::Exited))
        && !diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == FailureDomain::Environment
                && diagnostic.kind == FailureKind::ModuleLoad
        })
    {
        let combined = format!("{}\n{}", process.stdout, process.stderr);
        match parse_harness_events(&combined) {
            Ok(summary) if !summary.harness_completed => diagnostics.push(FailureDiagnostic {
                domain: FailureDomain::VerifierHarness,
                kind: FailureKind::HarnessProtocol,
                component: DiagnosticComponent::FuzzHarness,
                impact: DiagnosticImpact::Blocking,
                message: "generated verifier did not emit harness_completed".into(),
                process: process.termination.clone(),
                limits: Some(execution_limits.clone()),
            }),
            Err(error) => diagnostics.push(FailureDiagnostic {
                domain: FailureDomain::VerifierHarness,
                kind: FailureKind::HarnessProtocol,
                component: DiagnosticComponent::FuzzHarness,
                impact: DiagnosticImpact::Blocking,
                message: error,
                process: process.termination.clone(),
                limits: Some(execution_limits.clone()),
            }),
            _ => {}
        }
    }
    process.diagnostics = diagnostics.clone();
    drop(temporary);
    HarnessExecution {
        process,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn docker_instrumentation_preload_preserves_loader_options_and_image_position() {
        for existing in [
            None,
            Some("--experimental-loader=/court-jester/resolver.mjs"),
        ] {
            let mut create = vec![
                "create".into(),
                "-e".into(),
                "HOME=/tmp".into(),
                "image".into(),
            ];
            if let Some(options) = existing {
                super::insert_docker_environment(&mut create, format!("NODE_OPTIONS={options}"));
            }
            super::append_docker_node_preload(&mut create, "/court-jester/instrumentation.cjs");
            assert_eq!(create.last().unwrap(), "image");
            let options = create
                .iter()
                .filter(|value| value.starts_with("NODE_OPTIONS="))
                .collect::<Vec<_>>();
            assert_eq!(options.len(), 1);
            assert!(options[0].ends_with("--require=/court-jester/instrumentation.cjs"));
            if let Some(existing) = existing {
                assert!(options[0].contains(existing));
            }
            assert!(create.iter().any(|value| value == "HOME=/tmp"));
        }
    }
    #[cfg(target_os = "macos")]
    use super::docker_runtime_user;
    use super::{
        configure_docker_node_loader, configure_docker_typescript_loader,
        copy_materialization_tree, create_network_guard, create_node_package_resolver,
        docker_dependency_mapping, docker_image_for_harness, docker_path_mapping,
        docker_project_module_path, harness_diagnostics, harness_extension_compatible,
        has_typescript_type_only_relative_imports, insert_docker_environment,
        isolated_typescript_barrel_redirects, resolve_typescript_runtime_reexport,
        typescript_virtual_type_imports, uv_python_runtime, virtual_env_bin,
        vitest_project_entrypoint, which_binary,
    };
    use crate::types::{
        DiagnosticComponent, DiagnosticImpact, ExecutionContext, ExecutionLimits, ExecutionResult,
        FailureDomain, FailureKind, NetworkPolicy, RuntimeProfile, SourceContext, SourceMode,
        TestAdapter,
    };

    #[test]
    fn default_typescript_image_selects_bun_for_bun_harnesses() {
        assert_eq!(
            docker_image_for_harness(
                crate::types::DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
                &crate::types::HarnessRuntime::BunTest,
            ),
            crate::types::DEFAULT_BUN_DOCKER_IMAGE
        );
        assert_eq!(
            docker_image_for_harness(
                "custom-typescript:latest",
                &crate::types::HarnessRuntime::BunTest,
            ),
            "custom-typescript:latest"
        );
        assert_eq!(
            docker_image_for_harness(
                crate::types::DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
                &crate::types::HarnessRuntime::NodeScript,
            ),
            crate::types::DEFAULT_TYPESCRIPT_DOCKER_IMAGE
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn docker_node_dependency_bind_uses_read_only_root_access_on_macos() {
        assert_eq!(docker_runtime_user(true), "0:0");
        assert_ne!(docker_runtime_user(false), "0:0");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn isolated_runtime_temporary_directories_use_docker_shared_home() {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return;
        };
        let directory = super::runtime_tempdir(RuntimeProfile::Isolated).unwrap();
        let expected_parent =
            std::path::PathBuf::from(home).join("Library/Caches/court-jester/runtime");

        assert!(directory.path().starts_with(expected_parent));
        assert!(directory.path().is_dir());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn isolated_standalone_temporary_directories_use_docker_shared_home() {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return;
        };
        let directory = super::standalone_runtime_tempdir(RuntimeProfile::Isolated).unwrap();
        let expected_parent =
            std::path::PathBuf::from(home).join("Library/Caches/court-jester/runtime");

        assert!(directory.path().starts_with(expected_parent));
        assert!(directory.path().is_dir());
    }

    #[test]
    fn typescript_loader_supplies_json_module_semantics_without_import_attributes() {
        let project = tempfile::tempdir().unwrap();
        let entrypoint = project.path().join("entry.mjs");
        let data = project.path().join("tenant.json");
        let typescript = project.path().join("typescript.mjs");
        std::fs::write(
            &entrypoint,
            "import tenant from './tenant.json';\nconsole.log(tenant.marker);\n",
        )
        .unwrap();
        std::fs::write(&data, r#"{"marker":"loader-json-ok"}"#).unwrap();
        std::fs::write(
            &typescript,
            "export default { findConfigFile() {}, sys: {}, ModuleKind: {}, ScriptTarget: {}, ModuleResolutionKind: {}, ImportsNotUsedAsValues: {}, transpileModule(source) { return { outputText: source }; } };\n",
        )
        .unwrap();
        let guard = create_network_guard(RuntimeProfile::LocalTrusted, None).unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&guard.typescript_loader)
            .arg(&entrypoint)
            .env("COURT_JESTER_TYPESCRIPT_MODULE", &typescript)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "loader-json-ok"
        );
    }

    #[test]
    fn combined_project_loaders_preserve_nested_json_without_import_attributes() {
        let project = tempfile::tempdir().unwrap();
        let overlay = project.path().join("overlay");
        let entrypoint = overlay.join("packages/app/entry.mjs");
        let nested_module = overlay.join("packages/config/index.mjs");
        let data = overlay.join("packages/config/tenant.json");
        let typescript = project.path().join("typescript.mjs");
        for path in [&entrypoint, &nested_module, &data, &typescript] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &entrypoint,
            "import { marker } from '../config/index.mjs';\nconsole.log(marker);\n",
        )
        .unwrap();
        std::fs::write(
            &nested_module,
            "import tenant from './tenant.json';\nexport const marker = tenant.marker;\n",
        )
        .unwrap();
        std::fs::write(&data, r#"{"marker":"nested-loader-json-ok"}"#).unwrap();
        std::fs::write(
            &typescript,
            "export default { findConfigFile() {}, sys: {}, ModuleKind: {}, ScriptTarget: {}, ModuleResolutionKind: {}, ImportsNotUsedAsValues: {}, transpileModule(source) { return { outputText: source }; } };\n",
        )
        .unwrap();
        let resolver = create_node_package_resolver(RuntimeProfile::LocalTrusted).unwrap();
        let guard = create_network_guard(RuntimeProfile::LocalTrusted, None).unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&guard.typescript_loader)
            .arg("--experimental-loader")
            .arg(&resolver.loader)
            .arg(&entrypoint)
            .env("COURT_JESTER_TYPESCRIPT_MODULE", &typescript)
            .env("COURT_JESTER_NODE_RESOLUTION_MODE", "generated")
            .env("COURT_JESTER_NODE_OVERLAY_ROOT", &overlay)
            .env("COURT_JESTER_NODE_SOURCE_ROOT", &overlay)
            .env(
                "COURT_JESTER_NODE_TARGET_ROOT",
                overlay.join("packages/app"),
            )
            .env(
                "COURT_JESTER_NODE_OVERLAY_TARGET_ROOT",
                overlay.join("packages/app"),
            )
            .env("COURT_JESTER_NODE_GENERATED_ARTIFACT", &entrypoint)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "nested-loader-json-ok"
        );
    }

    #[cfg(unix)]
    #[test]
    fn vitest_project_entrypoint_uses_manifest_ownership_for_npm_alias() {
        let dir = tempfile::tempdir().unwrap();
        let node_modules = dir.path().join("node_modules");
        let package_dir = node_modules.join("vitest-alias");
        let tool_dir = node_modules.join(".bin");
        std::fs::create_dir_all(package_dir.join("dist")).unwrap();
        std::fs::create_dir_all(&tool_dir).unwrap();
        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"vitest","version":"3.2.4","bin":{"vitest":"./dist/vitest.mjs"}}"#,
        )
        .unwrap();
        std::fs::write(package_dir.join("dist/vitest.mjs"), "process.exit(0);\n").unwrap();
        let launcher = tool_dir.join("vitest");
        std::os::unix::fs::symlink("../vitest-alias/dist/vitest.mjs", &launcher).unwrap();

        let (entrypoint, legacy_threads) = vitest_project_entrypoint(&launcher).unwrap();

        assert_eq!(
            entrypoint,
            std::fs::canonicalize(package_dir.join("dist/vitest.mjs")).unwrap()
        );
        assert!(!legacy_threads);

        std::fs::write(
            package_dir.join("package.json"),
            r#"{"name":"not-vitest","version":"3.2.4","bin":{"vitest":"./dist/vitest.mjs"}}"#,
        )
        .unwrap();
        assert!(
            vitest_project_entrypoint(&launcher).is_err(),
            "the manifest name must establish Vitest package ownership"
        );
    }

    #[test]
    fn vitest_pretty_printed_json_failure_is_a_target_assertion() {
        let process = ExecutionResult {
            stdout: r#"{
  "testResults": [
    {
      "assertionResults": [
        {
          "status": "failed"
        }
      ]
    }
  ]
}"#
            .into(),
            stderr: String::new(),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::LocalTrusted,
            network_policy: NetworkPolicy::Allow,
        };

        let diagnostics = harness_diagnostics(Some(TestAdapter::VitestJson), &process, &limits);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].domain, FailureDomain::TargetCode);
        assert_eq!(diagnostics[0].kind, FailureKind::AssertionFailure);
        assert_eq!(
            diagnostics[0].component,
            DiagnosticComponent::AuthoritativeTestRunner
        );
        assert_eq!(diagnostics[0].impact, DiagnosticImpact::Gating);

        let logged_process = ExecutionResult {
            stdout: format!(
                "setup log before reporter\n{}\nwarning after reporter",
                process.stdout
            ),
            ..process.clone()
        };
        for adapter in [TestAdapter::VitestJson, TestAdapter::JestJson] {
            let diagnostics = harness_diagnostics(Some(adapter), &logged_process, &limits);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
            assert_eq!(diagnostics[0].domain, FailureDomain::TargetCode);
            assert_eq!(diagnostics[0].kind, FailureKind::AssertionFailure);
            assert_eq!(diagnostics[0].impact, DiagnosticImpact::Gating);
        }

        for stdout in ["not json", r#"{"message":"runner crashed"}"#] {
            let non_result = ExecutionResult {
                stdout: stdout.into(),
                ..process.clone()
            };
            let diagnostics =
                harness_diagnostics(Some(TestAdapter::VitestJson), &non_result, &limits);

            assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
            assert_eq!(diagnostics[0].domain, FailureDomain::VerifierHarness);
            assert_eq!(diagnostics[0].kind, FailureKind::HarnessProtocol);
            assert_eq!(diagnostics[0].impact, DiagnosticImpact::Blocking);
        }
    }
    #[test]
    fn vitest_spawn_policy_denial_is_environmental_but_ordinary_assertions_are_target_code() {
        let reporter_output = |message: &str| {
            serde_json::json!({
                "numTotalTestSuites": 1,
                "numFailedTestSuites": 1,
                "numTotalTests": 1,
                "numFailedTests": 1,
                "success": false,
                "testResults": [{
                    "assertionResults": [{
                        "status": "failed",
                        "failureMessages": [message],
                    }],
                    "status": "failed",
                    "message": message,
                }],
            })
            .to_string()
        };
        let process = |message: &str| ExecutionResult {
            stdout: reporter_output(message),
            stderr: String::new(),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::LocalTrusted,
            network_policy: NetworkPolicy::Deny,
        };

        let denied = harness_diagnostics(
            Some(TestAdapter::VitestJson),
            &process(
                "pdf-inspector process could not be started: court-jester process spawn denied",
            ),
            &limits,
        );
        assert_eq!(denied.len(), 1, "{denied:#?}");
        assert_eq!(denied[0].domain, FailureDomain::Environment);
        assert_eq!(denied[0].kind, FailureKind::ProcessSpawnDenied);
        assert_eq!(denied[0].component, DiagnosticComponent::Sandbox);
        assert_eq!(denied[0].impact, DiagnosticImpact::Blocking);

        let assertion = harness_diagnostics(
            Some(TestAdapter::VitestJson),
            &process("AssertionError: expected 1 to be 2"),
            &limits,
        );
        assert_eq!(assertion.len(), 1, "{assertion:#?}");
        assert_eq!(assertion[0].domain, FailureDomain::TargetCode);
        assert_eq!(assertion[0].kind, FailureKind::AssertionFailure);
        assert_eq!(assertion[0].impact, DiagnosticImpact::Gating);
    }

    #[test]
    fn node_tap_preserves_child_policy_diagnostics_without_assertion_promotion() {
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::LocalTrusted,
            network_policy: NetworkPolicy::Deny,
        };
        for (message, expected) in [
            (
                "court-jester process spawn denied",
                FailureKind::ProcessSpawnDenied,
            ),
            (
                "court-jester network access denied",
                FailureKind::NetworkDenied,
            ),
            ("ordinary assertion failure", FailureKind::AssertionFailure),
        ] {
            let process = ExecutionResult {
                stdout: format!("TAP version 13\n# Error: {message}\nnot ok 1 - entrypoint\n"),
                stderr: String::new(),
                exit_code: Some(1),
                duration_ms: 1,
                timed_out: false,
                memory_error: false,
                termination: None,
                diagnostics: vec![],
            };
            let diagnostics = harness_diagnostics(Some(TestAdapter::NodeTap), &process, &limits);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
            assert_eq!(diagnostics[0].kind, expected);
            assert_eq!(
                diagnostics[0].impact,
                if expected == FailureKind::AssertionFailure {
                    DiagnosticImpact::Gating
                } else {
                    DiagnosticImpact::Blocking
                }
            );
            assert!(harness_diagnostics(Some(TestAdapter::Opaque), &process, &limits).is_empty());
            assert!(
                process.stderr.is_empty(),
                "transport decoding must not rewrite recorded raw streams"
            );
        }
    }

    #[test]
    fn vitest_initialization_exception_without_collected_tests_is_environmental() {
        let process = ExecutionResult {
            stdout: r#"{
  "numTotalTestSuites": 1,
  "numFailedTestSuites": 1,
  "numTotalTests": 0,
  "numFailedTests": 0,
  "success": false,
  "testResults": [
    {
      "assertionResults": [],
      "status": "passed",
      "message": "Cannot read properties of undefined (reading 'customEqualityTesters')",
      "name": "/workspace/packages/app/src/example.test.ts"
    }
  ]
}"#
            .into(),
            stderr: String::new(),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
        };

        let diagnostics = harness_diagnostics(Some(TestAdapter::VitestJson), &process, &limits);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].domain, FailureDomain::Environment);
        assert_eq!(diagnostics[0].kind, FailureKind::ModuleLoad);
        assert_eq!(diagnostics[0].component, DiagnosticComponent::ModuleLoader);
        assert_eq!(diagnostics[0].impact, DiagnosticImpact::Blocking);
        assert!(diagnostics[0].message.contains("customEqualityTesters"));
    }

    #[test]
    fn vitest_missing_global_after_config_fallback_is_environmental_only() {
        let process = ExecutionResult {
            stdout: r#"{
  "numTotalTestSuites": 1,
  "numFailedTestSuites": 1,
  "numTotalTests": 0,
  "numFailedTests": 0,
  "success": false,
  "testResults": [{
    "assertionResults": [],
    "status": "failed",
    "message": "ReferenceError: describe is not defined\n    at /workspace/packages/utils/src/featureFlags.test.ts:3:1",
    "name": "/workspace/packages/utils/src/featureFlags.test.ts"
  }]
}"#
            .into(),
            stderr: "court-jester project config requires a host-incompatible native dependency; retrying without project config\n".into(),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
        };

        let diagnostics = harness_diagnostics(Some(TestAdapter::VitestJson), &process, &limits);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].domain, FailureDomain::Environment);
        assert_eq!(diagnostics[0].kind, FailureKind::ModuleLoad);
        assert_eq!(diagnostics[0].component, DiagnosticComponent::ModuleLoader);
        assert_eq!(diagnostics[0].impact, DiagnosticImpact::Blocking);
        assert!(diagnostics[0].message.contains("describe is not defined"));
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.domain != FailureDomain::TargetCode
                && diagnostic.kind != FailureKind::AssertionFailure
                && diagnostic.impact != DiagnosticImpact::Gating
        }));

        let non_vitest = harness_diagnostics(Some(TestAdapter::JestJson), &process, &limits);
        assert_eq!(non_vitest.len(), 1, "{non_vitest:#?}");
        assert_eq!(non_vitest[0].domain, FailureDomain::Environment);
        assert_eq!(non_vitest[0].kind, FailureKind::ModuleLoad);
        assert_eq!(non_vitest[0].impact, DiagnosticImpact::Blocking);
    }

    #[test]
    fn zero_test_transform_failure_is_environmental() {
        let process = ExecutionResult {
            stdout: r#"{
  "numTotalTestSuites": 1,
  "numFailedTestSuites": 1,
  "numTotalTests": 0,
  "numFailedTests": 0,
  "success": false,
  "testResults": [{
    "assertionResults": [],
    "status": "failed",
    "message": "failed to resolve \"extends\":\"./.nuxt/tsconfig.json\" in /workspace/apps/client-app/tsconfig.json",
    "name": "/workspace/packages/db-entities/src/ProductConfiguration.test.ts"
  }]
}"#
            .into(),
            stderr: String::new(),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
        };

        let diagnostics = harness_diagnostics(Some(TestAdapter::VitestJson), &process, &limits);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].domain, FailureDomain::Environment);
        assert_eq!(diagnostics[0].kind, FailureKind::ModuleLoad);
        assert_eq!(diagnostics[0].impact, DiagnosticImpact::Blocking);
    }

    #[test]
    fn dependency_permission_error_is_environmental_and_preserves_path() {
        let inaccessible = "/court-jester/dependencies/node_modules/.pnpm/tinypool/dist/worker.js";
        let process = ExecutionResult {
            stdout: r#"{
  "numTotalTestSuites": 1,
  "numFailedTestSuites": 1,
  "numTotalTests": 1,
  "numFailedTests": 1,
  "success": false,
  "testResults": [{
    "assertionResults": [{"status": "failed"}],
    "status": "failed",
    "message": "dependency import failed"
  }]
}"#
            .into(),
            stderr: format!("Error: EACCES: permission denied, open '{inaccessible}'"),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let limits = ExecutionLimits {
            timeout_seconds: 1.0,
            memory_mb: 128,
            runtime_profile: RuntimeProfile::Isolated,
            network_policy: NetworkPolicy::Deny,
        };

        let diagnostics = harness_diagnostics(Some(TestAdapter::VitestJson), &process, &limits);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].domain, FailureDomain::Environment);
        assert_eq!(diagnostics[0].kind, FailureKind::ModuleLoad);
        assert_eq!(diagnostics[0].component, DiagnosticComponent::ModuleLoader);
        assert_eq!(diagnostics[0].impact, DiagnosticImpact::Blocking);
        assert!(diagnostics[0].message.contains(inaccessible));
    }

    #[test]
    fn generated_harness_resolver_prefers_target_owned_package_over_ambient_ancestor() {
        let directory = tempfile::tempdir().unwrap();
        let target_root = directory.path().join("target-package");
        let project_mirror = directory.path().join("generated-project-mirror");
        let ambient_package = directory.path().join("node_modules/fixture-package");
        let target_package = target_root.join("node_modules/fixture-package");
        let harness = project_mirror.join(".court-jester/generated/execute.mjs");
        std::fs::create_dir_all(&ambient_package).unwrap();
        std::fs::create_dir_all(&target_package).unwrap();
        std::fs::create_dir_all(harness.parent().unwrap()).unwrap();
        for package in [&ambient_package, &target_package] {
            std::fs::write(
                package.join("package.json"),
                r#"{"name":"fixture-package","type":"module","exports":"./index.js"}"#,
            )
            .unwrap();
        }
        std::fs::write(
            ambient_package.join("index.js"),
            "export const marker = 'ambient-ancestor';\n",
        )
        .unwrap();
        std::fs::write(
            target_package.join("index.js"),
            "export const marker = 'target-owned';\n",
        )
        .unwrap();
        std::fs::write(
            &harness,
            "import { marker } from 'fixture-package';\nconsole.log(marker);\n",
        )
        .unwrap();

        let resolver = create_node_package_resolver(RuntimeProfile::LocalTrusted).unwrap();
        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&resolver.loader)
            .arg(&harness)
            .env("COURT_JESTER_NODE_RESOLUTION_MODE", "generated")
            .env("COURT_JESTER_NODE_OVERLAY_ROOT", &project_mirror)
            .env("COURT_JESTER_NODE_SOURCE_ROOT", directory.path())
            .env("COURT_JESTER_NODE_TARGET_ROOT", &target_root)
            .env("COURT_JESTER_NODE_OVERLAY_TARGET_ROOT", &project_mirror)
            .env("COURT_JESTER_NODE_GENERATED_ARTIFACT", &harness)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "target-owned",
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(unix)]
    #[test]
    fn resolver_supports_extensionless_workspace_package_subpaths_from_dependencies() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let overlay = directory.path().join("overlay");
        let ai_package = workspace.join("packages/ai-workflows");
        let db_package = workspace.join("packages/db-entities");
        let entrypoint = ai_package.join("src/entry.mjs");
        let dependency = db_package.join("src/marketplace/AnonymousSearchSession.mjs");
        let package_link = ai_package.join("node_modules/@fixture/db-entities");
        std::fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        std::fs::create_dir_all(dependency.parent().unwrap()).unwrap();
        std::fs::create_dir_all(package_link.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(
            &entrypoint,
            "import { marker } from '@fixture/db-entities/src/marketplace/AnonymousSearchSession';\nconsole.log(marker);\n",
        )
        .unwrap();
        std::fs::write(
            &dependency,
            "export const marker = 'workspace-subpath-ok';\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&db_package, &package_link).unwrap();
        let resolver = create_node_package_resolver(RuntimeProfile::LocalTrusted).unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&resolver.loader)
            .arg(&entrypoint)
            .env("COURT_JESTER_NODE_RESOLUTION_MODE", "generated")
            .env("COURT_JESTER_NODE_OVERLAY_ROOT", &overlay)
            .env("COURT_JESTER_NODE_SOURCE_ROOT", &workspace)
            .env("COURT_JESTER_NODE_TARGET_ROOT", &ai_package)
            .env("COURT_JESTER_NODE_OVERLAY_TARGET_ROOT", &overlay)
            .env(
                "COURT_JESTER_NODE_GENERATED_ARTIFACT",
                overlay.join("generated.mjs"),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "workspace-subpath-ok"
        );
    }

    #[test]
    fn resolver_preserves_package_exports_for_workspace_dependencies() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let overlay = directory.path().join("overlay");
        let package = workspace.join("packages/provider");
        let dependency = package.join("node_modules/eventsource-parser");
        let entrypoint = package.join("src/entry.mjs");
        std::fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        std::fs::create_dir_all(dependency.join("dist")).unwrap();
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(
            &entrypoint,
            "import { marker } from 'eventsource-parser/stream';\nconsole.log(marker);\n",
        )
        .unwrap();
        std::fs::write(
            dependency.join("package.json"),
            r#"{"type":"module","exports":{"./stream":{"import":"./dist/stream.js"}}}"#,
        )
        .unwrap();
        std::fs::write(
            dependency.join("stream.js"),
            "export const marker = 'incorrect-compatibility-shim';\n",
        )
        .unwrap();
        std::fs::write(
            dependency.join("dist/stream.js"),
            "export const marker = 'package-exports-ok';\n",
        )
        .unwrap();
        let resolver = create_node_package_resolver(RuntimeProfile::LocalTrusted).unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&resolver.loader)
            .arg(&entrypoint)
            .env("COURT_JESTER_NODE_RESOLUTION_MODE", "generated")
            .env("COURT_JESTER_NODE_OVERLAY_ROOT", &overlay)
            .env("COURT_JESTER_NODE_SOURCE_ROOT", &workspace)
            .env("COURT_JESTER_NODE_TARGET_ROOT", &package)
            .env("COURT_JESTER_NODE_OVERLAY_TARGET_ROOT", &overlay)
            .env(
                "COURT_JESTER_NODE_GENERATED_ARTIFACT",
                overlay.join("generated.mjs"),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "package-exports-ok"
        );
    }

    #[test]
    fn node_dependency_loader_retries_transient_access_denial() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let dependency = workspace.join("node_modules/transient-dependency/index.mjs");
        let harness = workspace.join("harness.mjs");
        let flaky_loader = directory.path().join("flaky-loader.mjs");
        let attempt_counter = directory.path().join("attempts");
        std::fs::create_dir_all(dependency.parent().unwrap()).unwrap();
        std::fs::write(&dependency, "export const marker = 'readable';\n").unwrap();
        std::fs::write(
            &harness,
            "import { marker } from './node_modules/transient-dependency/index.mjs';\nconsole.log(marker);\n",
        )
        .unwrap();
        std::fs::write(
            &flaky_loader,
            r#"
import fs from "node:fs";
export async function load(url, context, nextLoad) {
  if (!url.endsWith("/transient-dependency/index.mjs")) {
    return nextLoad(url, context);
  }
  const counter = process.env.COURT_JESTER_FLAKY_COUNTER;
  const attempt = fs.existsSync(counter) ? Number(fs.readFileSync(counter, "utf8")) : 0;
  if (attempt < 2) {
    fs.writeFileSync(counter, String(attempt + 1));
    const error = new Error(`EACCES: permission denied, open '${url}'`);
    error.code = "EACCES";
    throw error;
  }
  return nextLoad(url, context);
}
"#,
        )
        .unwrap();

        let resolver = create_node_package_resolver(RuntimeProfile::LocalTrusted).unwrap();
        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&flaky_loader)
            .arg("--experimental-loader")
            .arg(&resolver.loader)
            .arg(&harness)
            .env("COURT_JESTER_FLAKY_COUNTER", &attempt_counter)
            .env("COURT_JESTER_NODE_RESOLUTION_MODE", "existing")
            .env("COURT_JESTER_NODE_OVERLAY_ROOT", &workspace)
            .env("COURT_JESTER_NODE_SOURCE_ROOT", &workspace)
            .env("COURT_JESTER_NODE_TARGET_ROOT", &workspace)
            .env("COURT_JESTER_NODE_OVERLAY_TARGET_ROOT", &workspace)
            .env("COURT_JESTER_NODE_GENERATED_ARTIFACT", &harness)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "readable");
        assert_eq!(std::fs::read_to_string(attempt_counter).unwrap(), "2");
    }

    #[cfg(unix)]
    #[test]
    fn docker_paths_accept_lexical_alias_for_generated_overlay() {
        let directory = tempfile::tempdir().unwrap();
        let source_root = directory.path().join("source");
        let alias_root = directory.path().join("alias");
        std::fs::create_dir(&source_root).unwrap();
        std::os::unix::fs::symlink(&source_root, &alias_root).unwrap();
        let artifact = alias_root.join(".court-jester/doctor.py");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, "print('ok')\n").unwrap();

        let mapping =
            docker_path_mapping(&alias_root, &alias_root, &artifact, &alias_root).unwrap();

        assert_eq!(mapping.project_root, alias_root);
        assert_eq!(
            mapping.container_artifact,
            std::path::Path::new("/workspace/.court-jester/doctor.py")
        );
        assert_eq!(mapping.container_cwd, std::path::Path::new("/workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn docker_paths_reject_artifact_symlink_escape() {
        let directory = tempfile::tempdir().unwrap();
        let source_root = directory.path().join("source");
        let outside_root = directory.path().join("outside");
        std::fs::create_dir(&source_root).unwrap();
        std::fs::create_dir(&outside_root).unwrap();
        let source_root = std::fs::canonicalize(source_root).unwrap();
        let outside_root = std::fs::canonicalize(outside_root).unwrap();
        std::fs::write(outside_root.join("doctor.py"), "print('escaped')\n").unwrap();
        std::os::unix::fs::symlink(&outside_root, source_root.join("escaped")).unwrap();
        let artifact = source_root.join("escaped/doctor.py");

        let error =
            docker_path_mapping(&source_root, &source_root, &artifact, &source_root).unwrap_err();

        assert_eq!(
            error,
            "docker harness artifact is outside the project mirror"
        );
    }

    #[test]
    fn docker_dependencies_share_one_workspace_topology() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let package = workspace.join("packages/app");
        std::fs::create_dir_all(package.join("node_modules")).unwrap();
        std::fs::create_dir_all(
            workspace.join("node_modules/.pnpm/example@1.0.0/node_modules/example"),
        )
        .unwrap();

        let mapping =
            docker_dependency_mapping(&workspace, &[package.clone(), workspace.clone()]).unwrap();

        assert_eq!(
            mapping.workspace_root,
            std::fs::canonicalize(&workspace).unwrap()
        );
        assert_eq!(
            mapping.container_roots,
            vec![
                "/court-jester/dependencies/packages/app".to_string(),
                "/court-jester/dependencies".to_string(),
            ]
        );
        assert_eq!(
            mapping.node_paths,
            vec![
                "/court-jester/dependencies/packages/app/node_modules".to_string(),
                "/court-jester/dependencies/node_modules".to_string(),
            ]
        );
        assert_eq!(
            mapping.node_bin_paths,
            vec![
                "/court-jester/dependencies/packages/app/node_modules/.bin".to_string(),
                "/court-jester/dependencies/node_modules/.bin".to_string(),
            ]
        );
    }
    #[test]
    fn typescript_source_mode_accepts_module_extensions() {
        for path in [
            "target.ts",
            "target.mts",
            "target.cts",
            "target.js",
            "target.mjs",
            "target.cjs",
        ] {
            assert!(
                harness_extension_compatible(std::path::Path::new(path), SourceMode::TypeScript),
                "{path}"
            );
        }
        assert!(!harness_extension_compatible(
            std::path::Path::new("target.py"),
            SourceMode::TypeScript
        ));
    }

    #[test]
    fn docker_project_module_path_prefers_package_local_dependency() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let package = workspace.join("packages/app");
        let package_module = package.join("node_modules/vitest/dist/node.mjs");
        let workspace_module = workspace.join("node_modules/vitest/dist/node.mjs");
        for module in [&package_module, &workspace_module] {
            std::fs::create_dir_all(module.parent().unwrap()).unwrap();
            std::fs::write(module, "export const marker = true;\n").unwrap();
        }

        let module_path = docker_project_module_path(
            &workspace,
            &package,
            &[workspace.clone(), package.clone()],
            &[std::path::Path::new("vitest/dist/node.mjs")],
        );

        assert_eq!(
            module_path.as_deref(),
            Some("/court-jester/dependencies/packages/app/node_modules/vitest/dist/node.mjs")
        );
    }

    #[cfg(unix)]
    #[test]
    fn docker_project_module_path_resolves_pnpm_symlink_targets() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let package = workspace.join("packages/app");
        let store_package =
            workspace.join("node_modules/.pnpm/typescript@5.1.6/node_modules/typescript");
        std::fs::create_dir_all(store_package.join("lib")).unwrap();
        std::fs::create_dir_all(workspace.join("node_modules")).unwrap();
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            store_package.join("lib/typescript.js"),
            "export const version = '5.1.6';\n",
        )
        .unwrap();
        symlink(
            ".pnpm/typescript@5.1.6/node_modules/typescript",
            workspace.join("node_modules/typescript"),
        )
        .unwrap();

        let module_path = docker_project_module_path(
            &workspace,
            &package,
            &[workspace.clone()],
            &[std::path::Path::new("typescript/lib/typescript.js")],
        );

        assert_eq!(
            module_path.as_deref(),
            Some(
                "/court-jester/dependencies/node_modules/.pnpm/typescript@5.1.6/node_modules/typescript/lib/typescript.js"
            )
        );
    }
    #[test]
    fn portable_vitest_uses_matching_runner_after_native_toolchain_failure() {
        let directory = tempfile::tempdir().unwrap();
        let package_modules = directory
            .path()
            .join("node_modules/.pnpm/vitest@3.1.3/node_modules");
        let vitest_root = package_modules.join("vitest");
        let runner_root = vitest_root.join("node_modules/@vitest/runner");
        let vitest_module = vitest_root.join("dist/node.js");
        let runner_module = runner_root.join("dist/index.js");
        let typescript_module = directory.path().join("typescript.mjs");
        let test_file = directory.path().join("example.test.mjs");
        for root in [&vitest_root, &runner_root] {
            std::fs::create_dir_all(root.join("dist")).unwrap();
            std::fs::write(root.join("package.json"), r#"{"type":"module"}"#).unwrap();
        }
        std::fs::write(
            &vitest_module,
            r#"
export const version = "3.1.3";
export async function startVitest() {
  throw new Error("Cannot find module '@rollup/rollup-linux-arm64-gnu'");
}
"#,
        )
        .unwrap();
        std::fs::write(
            &runner_module,
            r#"
export async function startTests(files, runner) {
  await runner.importFile(files[0]);
  process.stderr.write("runner=matching-vitest\n");
  return [{
    name: files[0],
    type: "suite",
    mode: "run",
    result: { state: "pass", duration: 1 },
    tasks: [{
      name: "uses the owning package graph",
      type: "test",
      mode: "run",
      result: { state: "pass", duration: 1 },
    }],
  }];
}
"#,
        )
        .unwrap();
        std::fs::write(
            &typescript_module,
            r#"
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { ES2022: 99 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
};
"#,
        )
        .unwrap();
        std::fs::write(&test_file, "globalThis.portableVitestTestRan = true;\n").unwrap();
        let guard = create_network_guard(RuntimeProfile::LocalTrusted, None).unwrap();

        let output = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .current_dir(directory.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("runner=matching-vitest"));
        let summary: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("portable runner JSON summary");
        assert_eq!(summary["success"], true);
        assert_eq!(summary["numTotalTests"], 1);
        assert_eq!(summary["numPassedTests"], 1);

        std::fs::write(
            &runner_module,
            r#"
export async function startTests(files) {
  return [{
    name: files[0],
    type: "suite",
    mode: "run",
    result: { state: "pass", duration: 1 },
    tasks: [{
      name: "beforeAll lifecycle",
      type: "suite",
      mode: "run",
      result: { state: "fail", duration: 1 },
      tasks: [{
        name: "test body",
        type: "test",
        mode: "run",
        result: { state: "pass", duration: 1 },
      }],
    }],
  }];
}
"#,
        )
        .unwrap();
        let suite_failure = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!suite_failure.status.success());
        let failure_summary: serde_json::Value =
            serde_json::from_slice(&suite_failure.stdout).expect("suite failure JSON summary");
        assert_eq!(failure_summary["success"], false);
        assert_eq!(failure_summary["numFailedTestSuites"], 1);
        assert!(failure_summary["testResults"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("beforeAll lifecycle")));

        let filtered = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .arg("--grep")
            .arg("focused case")
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!filtered.status.success());
        let filtered_summary: serde_json::Value =
            serde_json::from_slice(&filtered.stdout).expect("filtered failure JSON summary");
        assert!(filtered_summary["testResults"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot preserve Vitest filters")));
    }

    #[test]
    fn portable_vitest_preserves_mocked_workspace_package_identity() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let package = workspace.join("packages/ai-workflows");
        let vitest_root = directory.path().join("node_modules/vitest");
        let vitest_module = vitest_root.join("dist/node.js");
        let typescript_module = directory.path().join("typescript.mjs");
        let test_file = workspace.join("target.test.ts");
        let target_file = workspace.join("packages/app/target.ts");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::create_dir_all(vitest_root.join("dist")).unwrap();
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"@fixture/ai-workflows","source":"src/index.ts"}"#,
        )
        .unwrap();
        std::fs::write(
            package.join("src/index.ts"),
            "export function generateTypedObject() { return 'real'; }\n",
        )
        .unwrap();
        std::fs::write(
            &test_file,
            "vi.mock('@fixture/ai-workflows', async () => ({ generateTypedObject: vi.fn() }));\n",
        )
        .unwrap();
        std::fs::write(vitest_root.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(
            &vitest_module,
            r#"
export const version = "3.1.3";
export async function startVitest(_kind, _filters, _options, overrides) {
  const transform = overrides.plugins.find(
    (plugin) => plugin.name === "court-jester-typescript-transform",
  );
  const source = "import { generateTypedObject } from '@fixture/ai-workflows';\n";
  const result = transform.transform(source, process.env.COURT_JESTER_TEST_TARGET);
  if (!result.code.includes("@fixture/ai-workflows")) {
    throw new Error(`mocked package identity was rewritten: ${result.code}`);
  }
  process.stdout.write(JSON.stringify({
    numTotalTestSuites: 1,
    numPassedTestSuites: 1,
    numFailedTestSuites: 0,
    numTotalTests: 1,
    numPassedTests: 1,
    numFailedTests: 0,
    success: true,
    testResults: [],
  }) + "\n");
  return true;
}
"#,
        )
        .unwrap();
        std::fs::write(
            &typescript_module,
            r#"
const ExportKeyword = 1;
function importStatement(source) {
  const moduleName = source.match(/from\s+['"]([^'"]+)['"]/)[1];
  const importedName = source.match(/import\s*\{\s*([A-Za-z0-9_]+)/)[1];
  return {
    kind: "import",
    moduleSpecifier: { kind: "string", text: moduleName },
    importClause: {
      isTypeOnly: false,
      namedBindings: {
        kind: "named-imports",
        elements: [{
          isTypeOnly: false,
          name: { text: importedName },
          propertyName: null,
        }],
      },
    },
    getStart() { return 0; },
    end: source.length,
  };
}
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { Latest: 99, ES2022: 99 },
  ScriptKind: { TS: 1, TSX: 2 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
  SyntaxKind: { ExportKeyword },
  createSourceFile(_filename, source) {
    if (source.startsWith("import")) return { statements: [importStatement(source)] };
    if (source.includes("export function generateTypedObject")) {
      return {
        statements: [{
          kind: "function",
          name: { text: "generateTypedObject" },
          modifiers: [{ kind: ExportKeyword }],
        }],
      };
    }
    return { statements: [] };
  },
  isImportDeclaration(node) { return node.kind === "import"; },
  isStringLiteral(node) { return node.kind === "string"; },
  isNamedImports(node) { return node.kind === "named-imports"; },
  isFunctionDeclaration(node) { return node.kind === "function"; },
  isClassDeclaration() { return false; },
  isEnumDeclaration() { return false; },
  isVariableStatement() { return false; },
  isExportDeclaration() { return false; },
  transpileModule(source) { return { outputText: source }; },
};
"#,
        )
        .unwrap();
        let guard = create_network_guard(RuntimeProfile::LocalTrusted, None).unwrap();

        let output = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .env("COURT_JESTER_WORKSPACE_ROOT", &workspace)
            .env("COURT_JESTER_TEST_TARGET", &target_file)
            .current_dir(&workspace)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("portable runner JSON summary");
        assert_eq!(summary["success"], true);
        assert_eq!(summary["numPassedTests"], 1);
    }

    #[test]
    fn portable_vitest_does_not_drop_project_config_for_target_failure() {
        let directory = tempfile::tempdir().unwrap();
        let vitest_root = directory.path().join("node_modules/vitest");
        let vitest_module = vitest_root.join("dist/node.js");
        let typescript_module = directory.path().join("typescript.mjs");
        let test_file = directory.path().join("target-error.test.mjs");
        std::fs::create_dir_all(vitest_root.join("dist")).unwrap();
        std::fs::write(vitest_root.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(
            &vitest_module,
            r#"
export const version = "3.1.3";
export async function startVitest() {
  process.stdout.write(JSON.stringify({
    numTotalTestSuites: 1,
    numFailedTestSuites: 1,
    numTotalTests: 0,
    numFailedTests: 0,
    success: false,
    testResults: [{
      assertionResults: [],
      status: "failed",
      message: "SyntaxError: Unexpected identifier 'ManyToOneOptions'",
      name: "target-error.test.mjs",
    }],
  }) + "\n");
  return false;
}
"#,
        )
        .unwrap();
        std::fs::write(
            &typescript_module,
            r#"
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { ES2022: 99 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
};
"#,
        )
        .unwrap();
        std::fs::write(
            &test_file,
            "throw new Error('must not execute directly');\n",
        )
        .unwrap();
        let guard = create_network_guard(RuntimeProfile::LocalTrusted, None).unwrap();

        let output = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .current_dir(directory.path())
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("retrying without project config"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("target failure JSON summary");
        assert_eq!(summary["numTotalTests"], 0);
        assert!(summary["testResults"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Unexpected identifier")));

        std::fs::write(
            &vitest_module,
            r#"
export const version = "3.1.3";
let attempt = 0;
export async function startVitest() {
  attempt += 1;
  if (attempt === 1) {
    throw new Error("Cannot find module '@rollup/rollup-linux-arm64-gnu'");
  }
  process.stdout.write(JSON.stringify({
    numTotalTestSuites: 1,
    numFailedTestSuites: 1,
    numTotalTests: 0,
    numFailedTests: 0,
    success: false,
    testResults: [{
      assertionResults: [],
      status: "failed",
      message: "Error: Cannot find package 'jsdom' imported from Vitest",
      name: "target-error.test.mjs",
    }],
  }) + "\n");
  return false;
}
"#,
        )
        .unwrap();
        let second_failure = std::process::Command::new("node")
            .arg(&guard.portable_vitest_coordinator)
            .arg(&vitest_module)
            .arg(&typescript_module)
            .arg(&test_file)
            .arg(&guard.node_preload)
            .arg(&guard.instrumentation_preload)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!second_failure.status.success());
        let second_stderr = String::from_utf8_lossy(&second_failure.stderr);
        assert!(second_stderr.contains("retrying without project config"));
        assert!(!second_stderr.contains("using the matching package runner"));
        let second_summary: serde_json::Value = serde_json::from_slice(&second_failure.stdout)
            .expect("config-free environment failure JSON summary");
        assert!(second_summary["testResults"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Cannot find package 'jsdom'")));
    }

    #[test]
    fn typescript_loader_transpiles_instrumented_payload() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.ts");
        let typescript_module = directory.path().join("typescript.mjs");
        std::fs::write(&target, "throw new Error('original source executed');\n").unwrap();
        std::fs::write(
            &typescript_module,
            r#"
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { ES2022: 99 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
  transpileModule(source) {
    return { outputText: source.replace(": number", "") };
  },
};
"#,
        )
        .unwrap();
        let guard = create_network_guard(
            RuntimeProfile::LocalTrusted,
            Some("const value: number = 3;\nconsole.log(value);\n"),
        )
        .unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&guard.typescript_loader)
            .arg("--require")
            .arg(&guard.instrumentation_preload)
            .arg(&target)
            .env("COURT_JESTER_TYPESCRIPT_MODULE", &typescript_module)
            .env("COURT_JESTER_INSTRUMENT_TARGET", &target)
            .env(
                "COURT_JESTER_INSTRUMENT_PAYLOAD",
                guard.instrumented_source.as_ref().unwrap(),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "3");
    }

    #[test]
    fn typescript_loader_preserves_instrumented_javascript_payload() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.mjs");
        let typescript_module = directory.path().join("typescript.mjs");
        std::fs::write(&target, "throw new Error('original source executed');\n").unwrap();
        std::fs::write(
            &typescript_module,
            r#"
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { ES2022: 99 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
};
"#,
        )
        .unwrap();
        let guard = create_network_guard(
            RuntimeProfile::LocalTrusted,
            Some("console.log('instrumented-javascript');\n"),
        )
        .unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&guard.typescript_loader)
            .arg("--require")
            .arg(&guard.instrumentation_preload)
            .arg(&target)
            .env("COURT_JESTER_TYPESCRIPT_MODULE", &typescript_module)
            .env("COURT_JESTER_INSTRUMENT_TARGET", &target)
            .env(
                "COURT_JESTER_INSTRUMENT_PAYLOAD",
                guard.instrumented_source.as_ref().unwrap(),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "instrumented-javascript"
        );
    }

    #[test]
    fn typescript_loader_transpiles_instrumented_jsx_payload() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.jsx");
        let typescript_module = directory.path().join("typescript.mjs");
        std::fs::write(&target, "throw new Error('original source executed');\n").unwrap();
        std::fs::write(
            &typescript_module,
            r#"
export default {
  findConfigFile() { return undefined; },
  sys: { fileExists() { return false; }, readFile() { return ""; } },
  ModuleKind: { ESNext: 99 },
  ScriptTarget: { ES2022: 99 },
  ModuleResolutionKind: { Bundler: 99 },
  ImportsNotUsedAsValues: { Remove: 0 },
  transpileModule(source) {
    return { outputText: source.replace("<Widget />", "'jsx'") };
  },
};
"#,
        )
        .unwrap();
        let guard = create_network_guard(
            RuntimeProfile::LocalTrusted,
            Some("console.log(<Widget />);\n"),
        )
        .unwrap();

        let output = std::process::Command::new("node")
            .arg("--no-warnings")
            .arg("--experimental-loader")
            .arg(&guard.typescript_loader)
            .arg("--require")
            .arg(&guard.instrumentation_preload)
            .arg(&target)
            .env("COURT_JESTER_TYPESCRIPT_MODULE", &typescript_module)
            .env("COURT_JESTER_INSTRUMENT_TARGET", &target)
            .env(
                "COURT_JESTER_INSTRUMENT_PAYLOAD",
                guard.instrumented_source.as_ref().unwrap(),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "jsx");
    }

    #[test]
    fn docker_tsx_loader_uses_node_options_before_cli_startup() {
        let mut command = vec![
            "tsx".to_string(),
            "/workspace/.court-jester/execute.tsx".to_string(),
        ];

        let environment = configure_docker_node_loader(
            crate::types::HarnessRuntime::TsxScript,
            "/court-jester/package-resolver.mjs",
            &mut command,
        );

        assert_eq!(
            environment.as_deref(),
            Some("NODE_OPTIONS=--experimental-loader=/court-jester/package-resolver.mjs")
        );
        assert_eq!(command, vec!["tsx", "/workspace/.court-jester/execute.tsx"]);

        let mut create = vec!["create".to_string(), "node:24-bookworm-slim".to_string()];
        insert_docker_environment(&mut create, environment.unwrap());
        assert_eq!(
            create,
            vec![
                "create",
                "-e",
                "NODE_OPTIONS=--experimental-loader=/court-jester/package-resolver.mjs",
                "node:24-bookworm-slim",
            ]
        );
    }

    #[test]
    fn docker_vitest_loader_wraps_portable_coordinator() {
        let mut command = vec![
            "node".to_string(),
            "/court-jester/portable-vitest-coordinator.mjs".to_string(),
        ];

        let environment = configure_docker_node_loader(
            crate::types::HarnessRuntime::Vitest,
            "/court-jester/package-resolver.mjs",
            &mut command,
        );

        assert_eq!(
            environment.as_deref(),
            Some("NODE_OPTIONS=--experimental-loader=/court-jester/package-resolver.mjs")
        );
        assert_eq!(
            command,
            vec![
                "node",
                "--experimental-loader",
                "/court-jester/package-resolver.mjs",
                "/court-jester/portable-vitest-coordinator.mjs",
            ]
        );
    }

    #[test]
    fn docker_vitest_package_loader_wraps_typescript_loader() {
        let mut command = vec![
            "node".to_string(),
            "/court-jester/portable-vitest-coordinator.mjs".to_string(),
        ];
        configure_docker_node_loader(
            crate::types::HarnessRuntime::Vitest,
            "/court-jester/package-resolver.mjs",
            &mut command,
        );
        configure_docker_typescript_loader("/court-jester/typescript-loader.mjs", &mut command);

        assert_eq!(
            command,
            vec![
                "node",
                "--experimental-loader",
                "/court-jester/typescript-loader.mjs",
                "--experimental-loader",
                "/court-jester/package-resolver.mjs",
                "/court-jester/portable-vitest-coordinator.mjs",
            ]
        );
    }

    #[test]
    fn docker_node_loader_precedes_script_arguments() {
        let mut command = vec![
            "node".to_string(),
            "--no-warnings".to_string(),
            "/workspace/.court-jester/execute.ts".to_string(),
        ];

        let environment = configure_docker_node_loader(
            crate::types::HarnessRuntime::NodeScript,
            "/court-jester/package-resolver.mjs",
            &mut command,
        );

        assert!(environment.is_none());
        assert_eq!(
            command,
            vec![
                "node",
                "--experimental-loader",
                "/court-jester/package-resolver.mjs",
                "--no-warnings",
                "/workspace/.court-jester/execute.ts",
            ]
        );
    }

    #[test]
    fn which_binary_finds_existing_binary_on_path() {
        let path_env = "/missing:/bin:/usr/bin";
        assert_eq!(which_binary(path_env, "sh"), Some("/bin/sh".to_string()));
    }

    #[test]
    fn which_binary_returns_none_for_missing_binary() {
        let path_env = "/missing:/also-missing";
        assert_eq!(which_binary(path_env, "definitely-not-a-real-binary"), None);
    }

    #[test]
    fn detects_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(
            &helper_path,
            "export type PathValue = string | number;\nexport const value = 7;\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue, value } from \"./internals.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn ignores_plain_value_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("helper.ts");
        std::fs::write(&helper_path, "export const value = 7;\n").unwrap();
        std::fs::write(
            &source_path,
            "import { value } from \"./helper.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(!has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_multiline_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &source_path,
            "import {\n  PathValue,\n} from \"./internals.ts\";\nconsole.log('ok');\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_transitive_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("test.ts");
        let object_path = dir.path().join("object.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &object_path,
            "import { PathValue } from \"./internals.ts\";\nexport function pick(path: PathValue): string { return String(path); }\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { pick } from \"./object.ts\";\nconsole.log(pick(\"x\"));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_type_only_relative_reexports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export type { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_value_reexports_of_type_only_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn identifies_type_only_bindings_without_dropping_runtime_siblings() {
        let imports = typescript_virtual_type_imports(
            "import { AISettings } from '@resin8/db-entities';\n\
             import { Registration } from './registration';\n\
             import { runtimeValue, RuntimeType } from '@example/runtime';\n\
             export function upload(settings: AISettings, registration: Registration, value: RuntimeType) {\n\
               return runtimeValue(settings, registration, value);\n\
             }\n",
        );

        assert_eq!(
            imports.get("@resin8/db-entities"),
            Some(&vec!["AISettings".to_string()])
        );
        assert_eq!(
            imports.get("./registration"),
            Some(&vec!["Registration".to_string()])
        );
        assert_eq!(
            imports.get("@example/runtime"),
            Some(&vec!["RuntimeType".to_string()])
        );
    }

    #[test]
    fn resolves_selected_runtime_export_through_wildcard_barrel() {
        let dir = tempfile::tempdir().unwrap();
        let barrel = dir.path().join("index.ts");
        let selected = dir.path().join("Selected.ts");
        let unrelated = dir.path().join("Unrelated.ts");
        std::fs::write(
            &barrel,
            "export * from './Unrelated';\nexport * from './Selected';\n",
        )
        .unwrap();
        std::fs::write(&selected, "export const selected = 'ok';\n").unwrap();
        std::fs::write(&unrelated, "export const unrelated = 'skip';\n").unwrap();

        assert_eq!(
            resolve_typescript_runtime_reexport(
                &barrel,
                "selected",
                &mut std::collections::HashSet::new(),
            )
            .unwrap(),
            std::fs::canonicalize(selected).unwrap()
        );
    }

    #[test]
    fn rejects_ambiguous_wildcard_runtime_reexports() {
        let dir = tempfile::tempdir().unwrap();
        let barrel = dir.path().join("index.ts");
        let first = dir.path().join("First.ts");
        let second = dir.path().join("Second.ts");
        std::fs::write(
            &barrel,
            "export * from './First';\nexport * from './Second';\n",
        )
        .unwrap();
        std::fs::write(&first, "export const selected = 'first';\n").unwrap();
        std::fs::write(&second, "export const selected = 'second';\n").unwrap();

        assert_eq!(
            resolve_typescript_runtime_reexport(
                &barrel,
                "selected",
                &mut std::collections::HashSet::new(),
            ),
            None
        );
    }

    #[test]
    fn rejects_barrel_redirects_with_unchecked_specifier_uses() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("main.ts");
        let barrel = dir.path().join("index.ts");
        let selected = dir.path().join("Selected.ts");
        std::fs::write(&barrel, "export * from './Selected';\n").unwrap();
        std::fs::write(&selected, "export const selected = 'ok';\n").unwrap();

        let context = |source_file: &std::path::Path| ExecutionContext {
            invocation_dir: dir.path().to_path_buf(),
            workspace_root: dir.path().to_path_buf(),
            materialization_source_root: None,
            target_package_root: dir.path().to_path_buf(),
            test_package_root: None,
            dependency_roots: Vec::new(),
            target_source: SourceContext {
                language: crate::types::Language::TypeScript,
                mode: SourceMode::TypeScript,
                source_file: Some(source_file.to_path_buf()),
                virtual_file_path: None,
            },
            test_source: None,
        };

        std::fs::write(
            &source,
            "import fallback, { selected } from './index';\nconsole.log(fallback, selected);\n",
        )
        .unwrap();
        assert!(isolated_typescript_barrel_redirects(&context(&source)).is_empty());

        std::fs::write(
            &source,
            "import { selected } from './index';\nvoid import('./index');\nconsole.log(selected);\n",
        )
        .unwrap();
        assert!(isolated_typescript_barrel_redirects(&context(&source)).is_empty());

        std::fs::write(
            &source,
            "import { selected } from './index';\nimport './index';\nconsole.log(selected);\n",
        )
        .unwrap();
        assert!(isolated_typescript_barrel_redirects(&context(&source)).is_empty());
    }

    #[test]
    fn ignores_generated_benchmark_results() {
        let source = tempfile::tempdir().unwrap();
        let benchmark_dir = source.path().join("bench");
        let results_dir = benchmark_dir.join("results");
        std::fs::create_dir_all(&results_dir).unwrap();
        std::fs::write(source.path().join("keep.py"), "value = 1\n").unwrap();
        std::fs::write(benchmark_dir.join("keep.py"), "value = 2\n").unwrap();
        std::fs::write(results_dir.join("report.json"), "{}\n").unwrap();

        let destination = tempfile::tempdir().unwrap();
        copy_materialization_tree(source.path(), destination.path(), None).unwrap();

        assert!(destination.path().join("keep.py").is_file());
        assert!(destination.path().join("bench/keep.py").is_file());
        assert!(!destination.path().join("bench/results").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ignores_dangling_symlinks_when_materializing_project_overlay() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("keep.ts"), "export const keep = true;\n").unwrap();
        std::os::unix::fs::symlink(
            source.path().join("missing-target"),
            source.path().join("dangling-link"),
        )
        .unwrap();

        let destination = tempfile::tempdir().unwrap();
        copy_materialization_tree(source.path(), destination.path(), None).unwrap();

        assert!(destination.path().join("keep.ts").is_file());
        assert!(!destination.path().join("dangling-link").exists());
    }

    #[cfg(unix)]
    #[test]
    fn materializes_only_explicitly_trusted_external_overlay_links() {
        let overlay = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let allowed_destination = tempfile::tempdir().unwrap();
        let denied_destination = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("packages/app")).unwrap();
        std::fs::write(
            workspace.path().join("packages/app/index.ts"),
            "export const value = 1;\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(
            workspace.path().join("packages"),
            overlay.path().join("packages"),
        )
        .unwrap();

        copy_materialization_tree(
            overlay.path(),
            allowed_destination.path(),
            Some(workspace.path()),
        )
        .unwrap();
        assert!(allowed_destination
            .path()
            .join("packages/app/index.ts")
            .is_file());

        let error =
            copy_materialization_tree(overlay.path(), denied_destination.path(), None).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn active_virtual_environment_contributes_runtime_bin() {
        let root = std::path::Path::new("/tmp/project-environment");
        assert_eq!(
            virtual_env_bin(Some(root.as_os_str())),
            Some(root.join(if cfg!(windows) { "Scripts" } else { "bin" }))
        );
        assert_eq!(virtual_env_bin(None), None);
    }

    #[cfg(unix)]
    #[test]
    fn locked_uv_project_uses_isolated_offline_python_runtime() {
        use std::os::unix::fs::PermissionsExt;

        let project = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("pyproject.toml"),
            "[project]\nname='demo'\n",
        )
        .unwrap();
        std::fs::write(project.path().join("uv.lock"), "version = 1\n").unwrap();
        let uv = tools.path().join("uv");
        std::fs::write(&uv, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(&uv).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&uv, permissions).unwrap();
        let context = ExecutionContext {
            invocation_dir: project.path().to_path_buf(),
            workspace_root: project.path().to_path_buf(),
            materialization_source_root: None,
            target_package_root: project.path().to_path_buf(),
            test_package_root: None,
            dependency_roots: vec![project.path().to_path_buf()],
            target_source: SourceContext {
                language: crate::types::Language::Python,
                mode: SourceMode::Python,
                source_file: Some(project.path().join("target.py")),
                virtual_file_path: None,
            },
            test_source: None,
        };

        let (executable, args) =
            uv_python_runtime(tools.path().to_str().unwrap(), &context, false).unwrap();
        assert_eq!(executable, uv);
        assert_eq!(
            args,
            vec![
                "run",
                "--isolated",
                "--frozen",
                "--offline",
                "--project",
                project.path().to_str().unwrap(),
                "python3",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
        );

        std::fs::create_dir_all(project.path().join(".venv/bin")).unwrap();
        std::fs::write(project.path().join(".venv/bin/python3"), "").unwrap();
        assert!(uv_python_runtime(tools.path().to_str().unwrap(), &context, false).is_none());
        assert!(uv_python_runtime(tools.path().to_str().unwrap(), &context, true).is_none());
    }
}
