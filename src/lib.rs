pub mod tools;
pub mod types;

use std::path::{Component, Path, PathBuf};

use crate::types::{
    ContextError, ContextRequest, ExecutionContext, Language, SourceContext, SourceMode,
};

/// Build a uniform JSON error response for pre-tool validation failures.
/// The CLI prints these payloads directly so agents can rely on a stable
/// `{"error": "...", "error_kind": "..."}` shape.
pub fn tool_error(kind: &str, message: impl AsRef<str>) -> String {
    let value = serde_json::json!({
        "error": message.as_ref(),
        "error_kind": kind,
    });
    serde_json::to_string_pretty(&value).expect("serde_json::to_string_pretty on json! never fails")
}

pub fn resolve_code(code: &str, file_path: Option<&str>) -> Result<String, String> {
    match (code.is_empty(), file_path) {
        (false, None) => Ok(code.to_string()),
        (true, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| tool_error("read_failed", format!("Cannot read '{}': {}", path, e))),
        (false, Some(_)) => Err(tool_error(
            "ambiguous_input",
            "Provide either 'code' or 'file_path', not both",
        )),
        (true, None) => Err(tool_error(
            "missing_input",
            "Must provide 'code' or 'file_path'",
        )),
    }
}

pub fn parse_language(s: &str) -> Result<Language, String> {
    Language::parse(s).ok_or_else(|| {
        tool_error(
            "unsupported_language",
            format!(
                "Unsupported language '{}'. Use 'python' or 'typescript'.",
                s
            ),
        )
    })
}

/// Walk up from a file path to find the nearest project root with dependencies.
/// Prefers directories with actual node_modules/.venv over bare package markers,
/// which helps in monorepos with hoisted dependencies.
pub fn detect_project_dir(file_path: &str) -> Option<String> {
    let path = std::path::Path::new(file_path);
    let mut dir = path.parent()?;
    let mut fallback: Option<String> = None;
    loop {
        if dir.join("node_modules").is_dir() || dir.join(".venv").is_dir() {
            return Some(dir.to_string_lossy().to_string());
        }
        if fallback.is_none()
            && (dir.join("package.json").is_file() || dir.join("pyproject.toml").is_file())
        {
            fallback = Some(dir.to_string_lossy().to_string());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    fallback
}

fn canonicalize_existing(path: &Path, base: &Path) -> Result<PathBuf, ContextError> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    std::fs::canonicalize(&resolved).map_err(|error| {
        ContextError::MissingSourceFile(format!("cannot resolve '{}': {error}", resolved.display()))
    })
}

fn canonicalize_directory(
    path: &Path,
    invocation_dir: &Path,
    invalid: impl FnOnce(String) -> ContextError,
) -> Result<PathBuf, ContextError> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        invocation_dir.join(path)
    };
    if !resolved.is_dir() {
        return Err(invalid(format!(
            "project directory '{}' is not a directory",
            resolved.display()
        )));
    }
    std::fs::canonicalize(&resolved).map_err(|error| {
        invalid(format!(
            "cannot canonicalize '{}': {error}",
            resolved.display()
        ))
    })
}
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn ancestors(start: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut current = start;
    loop {
        result.push(current.to_path_buf());
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => break,
        }
    }
    result
}
fn shared_ancestor(left: &Path, right: &Path) -> PathBuf {
    ancestors(left)
        .into_iter()
        .find(|dir| right.starts_with(dir))
        .unwrap_or_else(|| left.to_path_buf())
}

fn has_package_marker(dir: &Path) -> bool {
    dir.join("package.json").is_file() || dir.join("pyproject.toml").is_file()
}

fn dependency_marker(dir: &Path) -> bool {
    dir.join("node_modules").is_dir() || dir.join(".venv").is_dir()
}

fn nearest_package_root(start: &Path, stop: &Path) -> PathBuf {
    ancestors(start)
        .into_iter()
        .take_while(|dir| dir.starts_with(stop))
        .find(|dir| has_package_marker(dir))
        .unwrap_or_else(|| start.to_path_buf())
}

fn dependency_roots(
    starts: &[&Path],
    workspace_root: &Path,
    target_package_root: &Path,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for start in starts {
        for dir in ancestors(start) {
            if !dir.starts_with(workspace_root) {
                break;
            }
            if dependency_marker(&dir) && !roots.iter().any(|existing| existing == &dir) {
                roots.push(dir);
            }
        }
    }
    if roots.is_empty() {
        roots.push(target_package_root.to_path_buf());
    }
    roots
}

fn nearest_jsx_setting(source_path: &Path, workspace_root: &Path) -> Option<bool> {
    let start = source_path.parent().unwrap_or(source_path);
    for dir in ancestors(start) {
        if !dir.starts_with(workspace_root) {
            break;
        }
        for filename in ["tsconfig.json", "jsconfig.json"] {
            let config = dir.join(filename);
            if !config.is_file() {
                continue;
            }
            let text = match std::fs::read_to_string(config) {
                Ok(text) => text,
                Err(_) => continue,
            };
            let value = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => value,
                Err(_) => return Some(false),
            };
            let jsx = value
                .get("compilerOptions")
                .and_then(|options| options.get("jsx"));
            if jsx.is_some() {
                return Some(true);
            }
        }
    }
    None
}

fn source_mode_for_path(
    language: &Language,
    path: Option<&Path>,
    workspace_root: &Path,
) -> SourceMode {
    if matches!(language, Language::Python) {
        return SourceMode::Python;
    }
    let extension = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("tsx") | Some("jsx") => SourceMode::Tsx,
        Some("ts") => SourceMode::TypeScript,
        _ => {
            if nearest_jsx_setting(path.unwrap_or(workspace_root), workspace_root) == Some(true) {
                SourceMode::Tsx
            } else {
                SourceMode::TypeScript
            }
        }
    }
}

fn build_source_context(
    language: Language,
    source_file: Option<PathBuf>,
    virtual_file_path: Option<PathBuf>,
    workspace_root: &Path,
) -> SourceContext {
    let mode_path = source_file.as_deref().or(virtual_file_path.as_deref());
    SourceContext {
        mode: source_mode_for_path(&language, mode_path, workspace_root),
        language,
        source_file,
        virtual_file_path,
    }
}

pub fn resolve_execution_context(
    request: ContextRequest<'_>,
) -> Result<ExecutionContext, ContextError> {
    let invocation_dir = std::fs::canonicalize(request.invocation_dir).map_err(|error| {
        ContextError::InvalidInvocationDirectory(format!(
            "cannot canonicalize '{}': {error}",
            request.invocation_dir.display()
        ))
    })?;
    if !invocation_dir.is_dir() {
        return Err(ContextError::InvalidInvocationDirectory(format!(
            "'{}' is not a directory",
            invocation_dir.display()
        )));
    }

    let explicit_root = request
        .explicit_project_dir
        .map(|path| {
            canonicalize_directory(path, &invocation_dir, ContextError::InvalidProjectDirectory)
        })
        .transpose()?;

    let source_file = request
        .target_file
        .map(|path| canonicalize_existing(path, &invocation_dir))
        .transpose()?;
    let test_file = request
        .test_file
        .map(|path| canonicalize_existing(path, &invocation_dir))
        .transpose()?;

    let target_parent = source_file
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&invocation_dir);

    let workspace_root = if let Some(root) = explicit_root {
        root
    } else {
        let target_dependencies = ancestors(target_parent)
            .into_iter()
            .find(|dir| dependency_marker(dir));
        target_dependencies
            .or_else(|| {
                ancestors(target_parent)
                    .into_iter()
                    .find(|dir| has_package_marker(dir))
            })
            .unwrap_or_else(|| {
                if request.target_file.is_some() {
                    if let Some(test_parent) = test_file.as_deref().and_then(Path::parent) {
                        shared_ancestor(target_parent, test_parent)
                    } else {
                        target_parent.to_path_buf()
                    }
                } else {
                    invocation_dir.clone()
                }
            })
    };

    if let Some(file) = source_file.as_deref() {
        if !file.starts_with(&workspace_root) {
            return Err(ContextError::SourceOutsideProject {
                source: file.display().to_string(),
                project: workspace_root.display().to_string(),
            });
        }
    }
    if let Some(file) = test_file.as_deref() {
        if !file.starts_with(&workspace_root) {
            return Err(ContextError::TestOutsideProject {
                test: file.display().to_string(),
                project: workspace_root.display().to_string(),
            });
        }
    }

    let target_package_root = nearest_package_root(target_parent, &workspace_root);
    let test_package_root = test_file
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| nearest_package_root(parent, &workspace_root));
    let mut dependency_starts = vec![target_parent];
    if let Some(parent) = test_file.as_deref().and_then(Path::parent) {
        dependency_starts.push(parent);
    }
    let dependencies = dependency_roots(&dependency_starts, &workspace_root, &target_package_root);

    let virtual_file = request.virtual_file_path.map(|path| {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        normalize_lexical(&resolved)
    });
    if let Some(path) = virtual_file.as_deref() {
        if !path.starts_with(&workspace_root) {
            return Err(ContextError::InvalidVirtualPath(format!(
                "virtual file '{}' is outside project '{}'",
                path.display(),
                workspace_root.display()
            )));
        }
        let mut existing = path;
        while !existing.exists() {
            let Some(parent) = existing.parent() else {
                break;
            };
            if parent == existing {
                break;
            }
            existing = parent;
        }
        if existing.exists() {
            let canonical = std::fs::canonicalize(existing).map_err(|error| {
                ContextError::InvalidVirtualPath(format!(
                    "cannot resolve virtual file '{}': {error}",
                    path.display()
                ))
            })?;
            if !canonical.starts_with(&workspace_root) {
                return Err(ContextError::InvalidVirtualPath(format!(
                    "virtual file '{}' escapes project '{}'",
                    path.display(),
                    workspace_root.display()
                )));
            }
        }
    }

    let target_source =
        build_source_context(request.language, source_file, virtual_file, &workspace_root);
    let test_source = test_file
        .map(|file| build_source_context(request.language, Some(file), None, &workspace_root));

    Ok(ExecutionContext {
        invocation_dir,
        workspace_root,
        target_package_root,
        test_package_root,
        dependency_roots: dependencies,
        target_source,
        test_source,
    })
}

pub fn resolve_verification_context(
    candidate: ContextRequest<'_>,
    base: Option<ContextRequest<'_>>,
) -> Result<types::VerificationContext, ContextError> {
    let candidate = resolve_execution_context(candidate)?;
    let base = base.map(resolve_execution_context).transpose()?;
    Ok(types::VerificationContext { candidate, base })
}
