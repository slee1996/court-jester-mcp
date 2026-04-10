use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::*;

pub struct LintOptions<'a> {
    pub source_file: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub config_path: Option<&'a str>,
    pub virtual_file_path: Option<&'a str>,
}

impl Default for LintOptions<'_> {
    fn default() -> Self {
        Self {
            source_file: None,
            project_dir: None,
            config_path: None,
            virtual_file_path: None,
        }
    }
}

/// Build a PATH that includes common tool install locations (uv, pip, homebrew, cargo).
fn extended_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let base = std::env::var("PATH").unwrap_or_default();
    format!(
        "{base}:{home}/.local/bin:{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
    )
}

fn find_binary_on_path(path_env: &str, binary: &str) -> Option<String> {
    for dir in path_env.split(':') {
        if let Some(candidate) = find_binary_in_dir(Path::new(dir), binary) {
            return Some(candidate);
        }
    }
    None
}

fn candidate_binary_names(binary: &str) -> Vec<String> {
    let mut names = vec![binary.to_string()];
    if cfg!(windows) {
        names.push(format!("{binary}.exe"));
        names.push(format!("{binary}.cmd"));
    }
    names
}

fn find_binary_in_dir(dir: &Path, binary: &str) -> Option<String> {
    for name in candidate_binary_names(binary) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn find_binary_near_exe_dir(exe_dir: Option<&Path>, binary: &str) -> Option<String> {
    let dir = exe_dir?;
    find_binary_in_dir(dir, binary)
}

fn find_project_local_binary(project_dir: Option<&str>, binary: &str) -> Option<String> {
    let project_dir = project_dir.map(Path::new)?;
    let candidates: &[&str] = match binary {
        "ruff" => &[".venv/bin", "venv/bin", ".venv/Scripts", "venv/Scripts"],
        "biome" => &["node_modules/.bin"],
        _ => &[],
    };

    for relative_dir in candidates {
        if let Some(path) = find_binary_in_dir(&project_dir.join(relative_dir), binary) {
            return Some(path);
        }
    }
    None
}

fn resolve_binary(
    path_env: &str,
    binary: &str,
    exe_dir: Option<&Path>,
    project_dir: Option<&str>,
) -> Option<String> {
    find_project_local_binary(project_dir, binary)
        .or_else(|| find_binary_near_exe_dir(exe_dir, binary))
        .or_else(|| find_binary_on_path(path_env, binary))
}

pub async fn lint(code: &str, language: &Language) -> LintResult {
    lint_with_options(code, language, LintOptions::default()).await
}

pub async fn lint_with_options(
    code: &str,
    language: &Language,
    opts: LintOptions<'_>,
) -> LintResult {
    match language {
        Language::Python => lint_python(code, &opts).await,
        Language::TypeScript => lint_typescript(code, &opts).await,
    }
}

fn tool_failure_message(
    tool: &str,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
) -> String {
    let exit = exit_status_label(status);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        signal_only_failure_hint(tool, status).unwrap_or_else(|| "no output".to_string())
    };
    format!("{tool} failed with {exit}: {detail}")
}

fn exit_status_label(status: std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit status {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return format!("signal {signal}{}", signal_name(signal));
        }
    }
    "terminated by signal".to_string()
}

#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGKILL => " (SIGKILL)",
        libc::SIGTERM => " (SIGTERM)",
        libc::SIGABRT => " (SIGABRT)",
        libc::SIGSEGV => " (SIGSEGV)",
        libc::SIGBUS => " (SIGBUS)",
        _ => "",
    }
    .to_string()
}

#[cfg(not(unix))]
fn signal_name(_signal: i32) -> String {
    String::new()
}

fn signal_only_failure_hint(tool: &str, status: std::process::ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if cfg!(target_os = "macos") && status.signal() == Some(libc::SIGKILL) {
            return Some(format!(
                "no output. macOS may have blocked the bundled {tool} binary (Gatekeeper/quarantine); try `xattr -dr com.apple.quarantine <bundle-dir>` or install {tool} in the project"
            ));
        }
    }
    None
}

fn tool_unavailable_message(tool: &str) -> String {
    format!("{tool} not available in project, on PATH, or next to court-jester-mcp")
}

fn working_dir(opts: &LintOptions<'_>) -> Option<PathBuf> {
    opts.project_dir.map(PathBuf::from).or_else(|| {
        opts.source_file
            .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
    })
}

fn lint_target_path(language: &Language, opts: &LintOptions<'_>) -> String {
    opts.source_file
        .or(opts.virtual_file_path)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| match language {
            Language::Python => "snippet.py".to_string(),
            Language::TypeScript => "snippet.ts".to_string(),
        })
}

async fn run_command(command: &mut Command, stdin_input: Option<&str>) -> std::io::Result<Output> {
    match stdin_input {
        Some(input) => {
            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());

            let mut child = command.spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(input.as_bytes()).await?;
            }
            child.wait_with_output().await
        }
        None => {
            command.stdin(Stdio::null());
            command.output().await
        }
    }
}

struct PreparedLintFile {
    path: PathBuf,
    cleanup_dirs: Vec<PathBuf>,
    _tempdir: Option<tempfile::TempDir>,
    remove_on_drop: bool,
}

impl PreparedLintFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PreparedLintFile {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }

        for dir in self.cleanup_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

fn missing_dirs_to_create(path: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        missing.push(current.to_path_buf());
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    missing.reverse();
    missing
}

fn prepare_typescript_inline_file(
    code: &str,
    opts: &LintOptions<'_>,
) -> Result<PreparedLintFile, String> {
    let relative_target = opts.virtual_file_path.unwrap_or("snippet.ts");

    if let Some(project_dir) = opts.project_dir {
        let project_root = Path::new(project_dir);
        let relative_path = Path::new(relative_target);
        let full_path = if relative_path.is_absolute() {
            relative_path.to_path_buf()
        } else {
            project_root.join(relative_path)
        };

        if full_path.exists() {
            return Err(format!(
                "Cannot materialize inline TypeScript lint file at existing path '{}'",
                full_path.display()
            ));
        }

        let parent = full_path.parent().ok_or_else(|| {
            format!(
                "Inline TypeScript lint path '{}' has no parent",
                full_path.display()
            )
        })?;
        let cleanup_dirs = missing_dirs_to_create(parent);
        for dir in &cleanup_dirs {
            std::fs::create_dir(dir).map_err(|e| {
                format!(
                    "Failed to create inline TypeScript lint directory '{}': {e}",
                    dir.display()
                )
            })?;
        }
        std::fs::write(&full_path, code)
            .map_err(|e| format!("Failed to write inline TypeScript lint file: {e}"))?;

        return Ok(PreparedLintFile {
            path: full_path,
            cleanup_dirs,
            _tempdir: None,
            remove_on_drop: true,
        });
    }

    let tempdir =
        tempfile::tempdir().map_err(|e| format!("Failed to create temp dir for lint: {e}"))?;
    let file_path = tempdir.path().join(relative_target);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create temp lint directory: {e}"))?;
    }
    std::fs::write(&file_path, code).map_err(|e| format!("Failed to write temp lint file: {e}"))?;

    Ok(PreparedLintFile {
        path: file_path,
        cleanup_dirs: Vec::new(),
        _tempdir: Some(tempdir),
        remove_on_drop: false,
    })
}

async fn lint_python(code: &str, opts: &LintOptions<'_>) -> LintResult {
    let path = extended_path();
    let exe_dir = current_exe_dir();
    let Some(ruff) = resolve_binary(&path, "ruff", exe_dir.as_deref(), opts.project_dir) else {
        return LintResult {
            diagnostics: vec![],
            error: Some(tool_unavailable_message("ruff")),
            unavailable: true,
        };
    };

    let mut command = Command::new(&ruff);
    command.args(["check", "--output-format=json"]);
    if let Some(config_path) = opts.config_path {
        command.args(["--config", config_path]);
    }
    let stdin_input = if let Some(source_file) = opts.source_file {
        command.arg(source_file);
        None
    } else {
        let target_path = lint_target_path(&Language::Python, opts);
        command.args(["--stdin-filename", &target_path, "-"]);
        Some(code)
    };
    command.env("PATH", &path);
    if let Some(dir) = working_dir(opts) {
        command.current_dir(dir);
    }

    let output = run_command(&mut command, stdin_input).await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let mut result = parse_ruff_output(&stdout);
            if result.error.is_some() || (!out.status.success() && result.diagnostics.is_empty()) {
                result.error = Some(tool_failure_message("ruff", out.status, &stdout, &stderr));
            }
            result
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("ruff not available: {e}")),
            unavailable: true,
        },
    }
}

fn parse_ruff_output(output: &str) -> LintResult {
    if output.trim().is_empty() {
        return LintResult {
            diagnostics: vec![],
            error: None,
            unavailable: false,
        };
    }

    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(output);
    match parsed {
        Ok(items) => {
            let diagnostics = items
                .iter()
                .filter_map(|item| {
                    let rule = item.get("code")?.as_str()?.to_string();
                    let message = item.get("message")?.as_str()?.to_string();
                    let location = item.get("location")?;
                    let line = location.get("row")?.as_u64()? as usize;
                    let column = location.get("column")?.as_u64()? as usize;
                    Some(LintDiagnostic {
                        rule,
                        message,
                        line,
                        column,
                        severity: "warning".to_string(),
                    })
                })
                .collect();
            LintResult {
                diagnostics,
                error: None,
                unavailable: false,
            }
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to parse ruff output: {e}")),
            unavailable: false,
        },
    }
}

async fn lint_typescript(code: &str, opts: &LintOptions<'_>) -> LintResult {
    let path = extended_path();
    let exe_dir = current_exe_dir();
    let Some(biome) = resolve_binary(&path, "biome", exe_dir.as_deref(), opts.project_dir) else {
        return LintResult {
            diagnostics: vec![],
            error: Some(tool_unavailable_message("biome")),
            unavailable: true,
        };
    };

    let input_file = match opts.source_file {
        Some(path) => PreparedLintFile {
            path: PathBuf::from(path),
            cleanup_dirs: Vec::new(),
            _tempdir: None,
            remove_on_drop: false,
        },
        None => match prepare_typescript_inline_file(code, opts) {
            Ok(file) => file,
            Err(e) => {
                return LintResult {
                    diagnostics: vec![],
                    error: Some(e),
                    unavailable: false,
                }
            }
        },
    };

    let mut command = Command::new(&biome);
    command.args(["lint", "--reporter=json"]);
    if let Some(config_path) = opts.config_path {
        command.args(["--config-path", config_path]);
    }
    command.arg(input_file.path());
    command.env("PATH", &path);
    if let Some(dir) = working_dir(opts) {
        command.current_dir(dir);
    }

    let output = run_command(&mut command, None).await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            // biome may output to stdout or stderr depending on version
            let text = if !stdout.trim().is_empty() {
                stdout.to_string()
            } else {
                stderr.to_string()
            };
            let mut result = parse_biome_output(&text);
            if result.error.is_some() || (!out.status.success() && result.diagnostics.is_empty()) {
                result.error = Some(tool_failure_message("biome", out.status, &stdout, &stderr));
            }
            result
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("biome not available: {e}")),
            unavailable: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_binary_near_exe_dir, find_project_local_binary, resolve_binary,
        tool_unavailable_message,
    };
    use std::fs;

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn resolve_binary_prefers_sibling_executable() {
        let sibling_dir = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let sibling = sibling_dir.path().join("biome");
        let on_path = path_dir.path().join("biome");
        fs::write(&sibling, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&on_path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            make_executable(&sibling);
            make_executable(&on_path);
        }

        let resolved = resolve_binary(
            path_dir.path().to_str().unwrap(),
            "biome",
            Some(sibling_dir.path()),
            None,
        )
        .expect("biome should resolve");

        assert_eq!(resolved, sibling.to_string_lossy());
    }

    #[test]
    fn resolve_ruff_prefers_sibling_executable() {
        let sibling_dir = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let sibling = sibling_dir.path().join("ruff");
        let on_path = path_dir.path().join("ruff");
        fs::write(&sibling, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&on_path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            make_executable(&sibling);
            make_executable(&on_path);
        }

        let resolved = resolve_binary(
            path_dir.path().to_str().unwrap(),
            "ruff",
            Some(sibling_dir.path()),
            None,
        )
        .expect("ruff should resolve");

        assert_eq!(resolved, sibling.to_string_lossy());
    }

    #[test]
    fn resolve_binary_prefers_project_local_executable() {
        let project_dir = tempfile::tempdir().unwrap();
        let sibling_dir = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let project_bin_dir = project_dir.path().join(".venv").join("bin");
        fs::create_dir_all(&project_bin_dir).unwrap();

        let project = project_bin_dir.join("ruff");
        let sibling = sibling_dir.path().join("ruff");
        let on_path = path_dir.path().join("ruff");
        fs::write(&project, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&sibling, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&on_path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            make_executable(&project);
            make_executable(&sibling);
            make_executable(&on_path);
        }

        let resolved = resolve_binary(
            path_dir.path().to_str().unwrap(),
            "ruff",
            Some(sibling_dir.path()),
            Some(project_dir.path().to_str().unwrap()),
        )
        .expect("ruff should resolve");

        assert_eq!(resolved, project.to_string_lossy());
    }

    #[test]
    fn find_binary_near_exe_dir_returns_none_when_missing() {
        let sibling_dir = tempfile::tempdir().unwrap();
        let resolved = find_binary_near_exe_dir(Some(sibling_dir.path()), "biome");
        assert!(resolved.is_none());
    }

    #[test]
    fn find_project_local_binary_returns_none_when_missing() {
        let project_dir = tempfile::tempdir().unwrap();
        assert!(
            find_project_local_binary(Some(project_dir.path().to_str().unwrap()), "biome")
                .is_none()
        );
    }

    #[test]
    fn unavailable_message_mentions_project_context() {
        assert_eq!(
            tool_unavailable_message("ruff"),
            "ruff not available in project, on PATH, or next to court-jester-mcp"
        );
    }
}

/// Extract the first top-level JSON object from a string that may contain
/// trailing non-JSON text (biome prints a human-readable summary after the JSON).
fn extract_json_object(s: &str) -> &str {
    let start = match s.find('{') {
        Some(i) => i,
        None => return s,
    };
    let mut depth = 0i32;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &s[start..start + i + 1];
                }
            }
            _ => {}
        }
    }
    s
}

fn parse_biome_output(output: &str) -> LintResult {
    if output.trim().is_empty() {
        return LintResult {
            diagnostics: vec![],
            error: None,
            unavailable: false,
        };
    }

    // biome --reporter=json outputs JSON followed by a human-readable summary.
    // Extract just the JSON object (first `{` to its matching `}`).
    let json_str = extract_json_object(output);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
    match parsed {
        Ok(val) => {
            let diagnostics = val
                .get("diagnostics")
                .and_then(|d| d.as_array())
                .map(|diags| {
                    diags
                        .iter()
                        .filter_map(|d| {
                            let rule = d.get("category")?.as_str()?.to_string();
                            let message = d
                                .get("description")
                                .or_else(|| d.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string();
                            let severity = d
                                .get("severity")
                                .and_then(|s| s.as_str())
                                .unwrap_or("warning")
                                .to_string();
                            let (line, column) = d
                                .get("location")
                                .and_then(|loc| {
                                    let start = loc.get("start")?;
                                    let l = start.get("line")?.as_u64()? as usize;
                                    let c = start.get("column")?.as_u64()? as usize;
                                    Some((l, c))
                                })
                                .unwrap_or((0, 0));
                            Some(LintDiagnostic {
                                rule,
                                message,
                                line,
                                column,
                                severity,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            LintResult {
                diagnostics,
                error: None,
                unavailable: false,
            }
        }
        Err(_) => LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to parse biome output")),
            unavailable: false,
        },
    }
}
