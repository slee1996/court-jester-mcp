pub mod tools;
pub mod types;

use crate::types::Language;

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
