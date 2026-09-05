use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::*;

#[derive(Default)]
pub struct LintOptions<'a> {
    pub source_file: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub config_path: Option<&'a str>,
    pub virtual_file_path: Option<&'a str>,
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

/// Resolve the exact linter used by both verification and readiness checks.
pub fn resolve_linter(language: &Language, project_dir: Option<&str>) -> Option<String> {
    let binary = match language {
        Language::Python => "ruff",
        Language::TypeScript => "biome",
    };
    resolve_binary(
        &extended_path(),
        binary,
        current_exe_dir().as_deref(),
        project_dir,
    )
}

/// Probe a resolved linter with the shared process-group timeout and cleanup rules.
pub async fn probe_linter_version(
    program: &str,
    cwd: &Path,
    timeout: f64,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .arg("--version")
        .current_dir(cwd)
        .env("PATH", extended_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let result = crate::tools::sandbox::run_command_with_limits(
        command,
        timeout,
        128,
        RuntimeProfile::LocalTrusted,
        NetworkPolicy::Deny,
        true,
        "failed to launch linter probe",
    )
    .await;
    if result.exit_code != Some(0) || result.timed_out || result.memory_error {
        return Err(format!(
            "{program} readiness probe failed (exit {:?}): {}",
            result.exit_code,
            result.stderr.trim()
        ));
    }
    let version = result.stdout.trim().to_string();
    if version.is_empty() {
        return Err(format!("{program} returned no version evidence"));
    }
    Ok(version)
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
    binary_path: &str,
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
        signal_only_failure_hint(tool, binary_path, status)
            .unwrap_or_else(|| "no output".to_string())
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

fn signal_only_failure_hint(
    tool: &str,
    binary_path: &str,
    status: std::process::ExitStatus,
) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if cfg!(target_os = "macos") && status.signal() == Some(libc::SIGKILL) {
            return Some(format!(
                "no output from '{}'. macOS may have blocked this {tool} executable (Gatekeeper/quarantine); try `xattr -dr com.apple.quarantine {binary_path}` or install {tool} in the project",
                binary_path
            ));
        }
    }
    None
}

fn signal_only_unavailable_message(
    tool: &str,
    binary_path: &str,
    status: std::process::ExitStatus,
    _stdout: &str,
    _stderr: &str,
) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if status.signal().is_some() {
            let exit = exit_status_label(status);
            let remediation = if cfg!(target_os = "macos") && status.signal() == Some(libc::SIGKILL)
            {
                format!(
                        "macOS may have blocked this executable (Gatekeeper/quarantine); try `xattr -dr com.apple.quarantine {binary_path}` or install {tool} in the project"
                    )
            } else {
                format!(
                        "reinstall {tool} or inspect the host launcher and resource limits before retrying"
                    )
            };
            return Some(format!(
                "{tool} unavailable: '{binary_path}' was terminated with {exit}. This is an environment failure, not a lint violation; {remediation}"
            ));
        }
    }

    None
}

struct LintInvocation<'a> {
    binary_path: &'a str,
    arguments: &'a [String],
    target: &'a str,
    cwd: Option<&'a Path>,
}

fn signal_unavailable_message_with_invocation(
    tool: &str,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
    invocation: &LintInvocation<'_>,
) -> Option<String> {
    let failure =
        signal_only_unavailable_message(tool, invocation.binary_path, status, stdout, stderr)?;
    Some(format!(
        "{failure}. Invocation: {}. {}",
        lint_invocation_context(invocation),
        lint_invocation_remediation()
    ))
}

fn lint_invocation_context(invocation: &LintInvocation<'_>) -> String {
    let cwd = invocation
        .cwd
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unavailable>".to_string());
    format!(
        "executable='{}', arguments={:?}, target='{}', cwd='{cwd}'",
        invocation.binary_path, invocation.arguments, invocation.target
    )
}

fn lint_invocation_remediation() -> &'static str {
    "Re-run this invocation in the shown cwd to distinguish a blocked or damaged executable from host resource limits; stdin content and environment variables are intentionally omitted."
}

fn lint_launch_failure_message(
    tool: &str,
    error: &std::io::Error,
    invocation: &LintInvocation<'_>,
) -> String {
    format!(
        "{tool} unavailable: failed to launch {}: {error}. This is an environment failure, not a lint violation. {}",
        lint_invocation_context(invocation),
        lint_invocation_remediation()
    )
}

fn lint_runner_failure_message(failure: String, invocation: &LintInvocation<'_>) -> String {
    format!(
        "{failure}. Invocation: {}. This is a lint runner or configuration failure, not a lint violation. {}",
        lint_invocation_context(invocation),
        lint_invocation_remediation()
    )
}

fn tool_unavailable_message(tool: &str) -> String {
    format!("{tool} not available in project, on PATH, or next to court-jester")
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
    let Some(ruff) = resolve_linter(&Language::Python, opts.project_dir) else {
        return LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some(tool_unavailable_message("ruff")),
            unavailable: true,
            runner_failed: false,
        };
    };

    let target = lint_target_path(&Language::Python, opts);
    let mut arguments = vec!["check".to_string(), "--output-format=json".to_string()];
    if let Some(config_path) = opts.config_path {
        arguments.push("--config".to_string());
        arguments.push(config_path.to_string());
    }
    let stdin_input = if opts.source_file.is_some() {
        arguments.push(target.clone());
        None
    } else {
        arguments.push("--stdin-filename".to_string());
        arguments.push(target.clone());
        arguments.push("-".to_string());
        Some(code)
    };

    let cwd = working_dir(opts).or_else(|| std::env::current_dir().ok());
    let mut command = Command::new(&ruff);
    command.args(&arguments);
    command.env("PATH", &path);
    if let Some(dir) = cwd.as_deref() {
        command.current_dir(dir);
    }
    let invocation = LintInvocation {
        binary_path: &ruff,
        arguments: &arguments,
        target: &target,
        cwd: cwd.as_deref(),
    };

    let output = run_command(&mut command, stdin_input).await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let mut result = parse_ruff_output(&stdout);
            if let Some(message) = signal_unavailable_message_with_invocation(
                "ruff",
                out.status,
                &stdout,
                &stderr,
                &invocation,
            ) {
                result.diagnostics.clear();
                result.runner_diagnostics.clear();
                result.error = Some(message);
                result.unavailable = true;
                result.runner_failed = false;
            } else if result.error.is_some()
                || (!out.status.success() && result.diagnostics.is_empty())
            {
                let failure = tool_failure_message("ruff", &ruff, out.status, &stdout, &stderr);
                result.error = Some(lint_runner_failure_message(failure, &invocation));
                result.runner_failed = true;
            }
            result
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some(lint_launch_failure_message("ruff", &e, &invocation)),
            unavailable: true,
            runner_failed: false,
        },
    }
}

fn parse_ruff_output(output: &str) -> LintResult {
    if output.trim().is_empty() {
        return LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: None,
            unavailable: false,
            runner_failed: false,
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
                runner_diagnostics: vec![],
                error: None,
                unavailable: false,
                runner_failed: false,
            }
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some(format!("Failed to parse ruff output: {e}")),
            unavailable: false,
            runner_failed: true,
        },
    }
}

async fn lint_typescript(code: &str, opts: &LintOptions<'_>) -> LintResult {
    let path = extended_path();
    let Some(biome) = resolve_linter(&Language::TypeScript, opts.project_dir) else {
        return LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some(tool_unavailable_message("biome")),
            unavailable: true,
            runner_failed: false,
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
                    runner_diagnostics: vec![],
                    error: Some(e),
                    unavailable: false,
                    runner_failed: false,
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
            if !result.runner_failed
                && (result.error.is_some()
                    || (!out.status.success()
                        && result.diagnostics.is_empty()
                        && result.runner_diagnostics.is_empty()))
            {
                if let Some(message) =
                    signal_only_unavailable_message("biome", &biome, out.status, &stdout, &stderr)
                {
                    result.error = Some(message);
                    result.unavailable = true;
                    result.runner_failed = false;
                } else {
                    result.error = Some(tool_failure_message(
                        "biome", &biome, out.status, &stdout, &stderr,
                    ));
                    result.runner_failed = true;
                }
            }
            filter_safe_borrowed_has_own_property_diagnostics(code, opts, &mut result.diagnostics);
            result
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some(format!("biome not available: {e}")),
            unavailable: true,
            runner_failed: false,
        },
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
            runner_diagnostics: vec![],
            error: None,
            unavailable: false,
            runner_failed: false,
        };
    }

    // biome --reporter=json outputs JSON followed by a human-readable summary.
    // Extract just the JSON object (first `{` to its matching `}`).
    let json_str = extract_json_object(output);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
    match parsed {
        Ok(val) => {
            let diagnostics: Vec<LintDiagnostic> = val
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
            let (runner_diagnostics, diagnostics): (Vec<_>, Vec<_>) = diagnostics
                .into_iter()
                .partition(biome_runner_failure_diagnostic);
            let error = if runner_diagnostics.is_empty() {
                None
            } else {
                Some(format!(
                    "biome runner failure: {}",
                    runner_diagnostics
                        .iter()
                        .map(format_lint_diagnostic_summary)
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
            };
            let runner_failed = error.is_some();
            LintResult {
                diagnostics,
                runner_diagnostics,
                error,
                unavailable: false,
                runner_failed,
            }
        }
        Err(_) => LintResult {
            diagnostics: vec![],
            runner_diagnostics: vec![],
            error: Some("Failed to parse biome output".to_string()),
            unavailable: false,
            runner_failed: true,
        },
    }
}

fn filter_safe_borrowed_has_own_property_diagnostics(
    code: &str,
    opts: &LintOptions<'_>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    if !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule == "lint/suspicious/noPrototypeBuiltins")
    {
        return;
    }
    let mut parser = tree_sitter::Parser::new();
    let lint_path = opts
        .source_file
        .or(opts.virtual_file_path)
        .unwrap_or_default();
    let grammar = if Path::new(lint_path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tsx"))
    {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    if parser.set_language(&grammar).is_err() {
        return;
    }
    let Some(tree) = parser.parse(code, None) else {
        return;
    };

    diagnostics.retain(|diagnostic| {
        diagnostic.rule != "lint/suspicious/noPrototypeBuiltins"
            || !diagnostic_targets_safe_borrowed_call(code, tree.root_node(), diagnostic)
    });
}

fn diagnostic_targets_safe_borrowed_call(
    code: &str,
    root: tree_sitter::Node<'_>,
    diagnostic: &LintDiagnostic,
) -> bool {
    if diagnostic.line == 0 || diagnostic.column == 0 {
        return false;
    }
    let point = tree_sitter::Point::new(diagnostic.line - 1, diagnostic.column - 1);
    let Some(mut node) = root.descendant_for_point_range(point, point) else {
        return false;
    };

    loop {
        if node.kind() == "call_expression" {
            let callee = node.child_by_field_name("function");
            if callee.is_some_and(|callee| point_is_within_node(point, callee))
                && is_safe_borrowed_has_own_property_call(code, node)
            {
                return true;
            }
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}

fn point_is_within_node(point: tree_sitter::Point, node: tree_sitter::Node<'_>) -> bool {
    let start = node.start_position();
    let end = node.end_position();
    (point.row > start.row || (point.row == start.row && point.column >= start.column))
        && (point.row < end.row || (point.row == end.row && point.column < end.column))
}

fn is_safe_borrowed_has_own_property_call(code: &str, call: tree_sitter::Node<'_>) -> bool {
    let source = code.as_bytes();
    let Some(call_member) = call.child_by_field_name("function") else {
        return false;
    };
    if call_member.kind() != "member_expression"
        || call_member
            .child_by_field_name("property")
            .and_then(|node| node.utf8_text(source).ok())
            != Some("call")
    {
        return false;
    }

    let Some(has_own_member) = call_member.child_by_field_name("object") else {
        return false;
    };
    if has_own_member.kind() != "member_expression"
        || has_own_member
            .child_by_field_name("property")
            .and_then(|node| node.utf8_text(source).ok())
            != Some("hasOwnProperty")
    {
        return false;
    }

    let Some(prototype_member) = has_own_member.child_by_field_name("object") else {
        return false;
    };
    if prototype_member.kind() != "member_expression"
        || prototype_member
            .child_by_field_name("property")
            .and_then(|node| node.utf8_text(source).ok())
            != Some("prototype")
    {
        return false;
    }
    let Some(object_identifier) = prototype_member.child_by_field_name("object") else {
        return false;
    };
    object_identifier.kind() == "identifier"
        && object_identifier.utf8_text(source).ok() == Some("Object")
        && !object_binding_is_shadowed(call, source)
}

fn object_binding_is_shadowed(call: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut ancestor = call.parent();
    while let Some(scope) = ancestor {
        if is_function_scope(scope)
            && (node_field_is_object(scope, "name", source)
                || scope
                    .child_by_field_name("parameters")
                    .is_some_and(|parameters| pattern_binds_object(parameters, source))
                || scope
                    .child_by_field_name("body")
                    .is_some_and(|body| subtree_has_object_var(body, source)))
        {
            return true;
        }
        if matches!(scope.kind(), "program" | "statement_block" | "switch_body")
            && direct_scope_binds_object(scope, source)
        {
            return true;
        }
        if scope.kind() == "program" && subtree_has_object_var(scope, source) {
            return true;
        }
        if scope.kind() == "catch_clause"
            && scope
                .child_by_field_name("parameter")
                .is_some_and(|parameter| pattern_binds_object(parameter, source))
        {
            return true;
        }
        if matches!(scope.kind(), "for_statement" | "for_in_statement")
            && for_statement_binds_object(scope, source)
        {
            return true;
        }
        ancestor = scope.parent();
    }
    false
}

fn is_function_scope(node: tree_sitter::Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration"
            | "arrow_function"
            | "method_definition"
    )
}

fn node_field_is_object(node: tree_sitter::Node<'_>, field: &str, source: &[u8]) -> bool {
    node.child_by_field_name(field)
        .and_then(|child| child.utf8_text(source).ok())
        == Some("Object")
}

fn any_named_child<'tree>(
    node: tree_sitter::Node<'tree>,
    mut predicate: impl FnMut(tree_sitter::Node<'tree>) -> bool,
) -> bool {
    let mut cursor = node.walk();
    let matched = node.named_children(&mut cursor).any(&mut predicate);
    matched
}

fn direct_scope_binds_object(scope: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    any_named_child(scope, |child| {
        declaration_binds_object(child, source)
            || (scope.kind() == "switch_body"
                && matches!(child.kind(), "switch_case" | "switch_default")
                && direct_scope_binds_object(child, source))
    })
}

fn declaration_binds_object(node: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    match node.kind() {
        "lexical_declaration" | "variable_declaration" => any_named_child(node, |child| {
            child.kind() == "variable_declarator"
                && child
                    .child_by_field_name("name")
                    .is_some_and(|name| pattern_binds_object(name, source))
        }),
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "enum_declaration"
        | "module"
        | "internal_module" => node_field_is_object(node, "name", source),
        "import_alias" => node
            .named_child(0)
            .is_some_and(|binding| binding.utf8_text(source).ok() == Some("Object")),
        "import_statement" => import_binds_object(node, source),
        "ambient_declaration" | "export_statement" => {
            any_named_child(node, |child| declaration_binds_object(child, source))
        }
        _ => false,
    }
}

fn import_binds_object(import: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    if import
        .utf8_text(source)
        .is_ok_and(|text| text.trim_start().starts_with("import type "))
    {
        return false;
    }
    any_named_child(import, |child| {
        child.kind() == "import_clause" && import_clause_binds_object(child, source)
    })
}

fn import_clause_binds_object(node: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    match node.kind() {
        "import_specifier" => {
            if node
                .utf8_text(source)
                .is_ok_and(|text| text.trim_start().starts_with("type "))
            {
                return false;
            }
            node.child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name"))
                .is_some_and(|binding| binding.utf8_text(source).ok() == Some("Object"))
        }
        "identifier" => node.utf8_text(source).ok() == Some("Object"),
        "namespace_import" => {
            any_named_child(node, |child| child.utf8_text(source).ok() == Some("Object"))
        }
        _ => any_named_child(node, |child| import_clause_binds_object(child, source)),
    }
}

fn subtree_has_object_var(node: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    any_named_child(node, |child| {
        if is_function_scope(child) {
            return false;
        }
        (child.kind() == "variable_declaration" && declaration_binds_object(child, source))
            || (child.kind() == "for_in_statement"
                && child
                    .child_by_field_name("kind")
                    .and_then(|kind| kind.utf8_text(source).ok())
                    == Some("var")
                && child
                    .child_by_field_name("left")
                    .is_some_and(|left| pattern_binds_object(left, source)))
            || subtree_has_object_var(child, source)
    })
}

fn for_statement_binds_object(node: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let declared_left = node.child_by_field_name("kind").is_some()
        && node
            .child_by_field_name("left")
            .is_some_and(|left| pattern_binds_object(left, source));
    if declared_left {
        return true;
    }
    any_named_child(node, |child| declaration_binds_object(child, source))
}

fn pattern_binds_object(node: tree_sitter::Node<'_>, source: &[u8]) -> bool {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            node.utf8_text(source).ok() == Some("Object")
        }
        "pair_pattern" => node
            .child_by_field_name("value")
            .is_some_and(|value| pattern_binds_object(value, source)),
        "assignment_pattern" | "object_assignment_pattern" => node
            .child_by_field_name("left")
            .is_some_and(|left| pattern_binds_object(left, source)),
        "required_parameter" | "optional_parameter" => node
            .child_by_field_name("pattern")
            .is_some_and(|pattern| pattern_binds_object(pattern, source)),
        "object_pattern" | "array_pattern" | "rest_pattern" | "formal_parameters" => {
            any_named_child(node, |child| pattern_binds_object(child, source))
        }
        _ => false,
    }
}

fn biome_runner_failure_diagnostic(diagnostic: &LintDiagnostic) -> bool {
    let rule = diagnostic.rule.trim().to_ascii_lowercase();
    let severity = diagnostic.severity.trim().to_ascii_lowercase();
    rule.starts_with("internalerror/")
        || rule == "internalerror"
        || (severity == "fatal" && rule.starts_with("internal"))
}

fn format_lint_diagnostic_summary(diagnostic: &LintDiagnostic) -> String {
    let message = diagnostic.message.trim();
    if message.is_empty() {
        diagnostic.rule.clone()
    } else {
        format!("{}: {}", diagnostic.rule, message)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_binary_near_exe_dir, find_project_local_binary, resolve_binary,
        signal_only_unavailable_message, tool_unavailable_message,
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

    #[cfg(unix)]
    #[test]
    fn signal_kill_without_output_is_treated_as_unavailable() {
        let status = std::process::Command::new("sh")
            .args(["-c", "kill -9 $$"])
            .status()
            .unwrap();
        let message = signal_only_unavailable_message("ruff", "/tmp/ruff", status, "", "")
            .expect("expected unavailable message for Unix signal termination");

        assert!(message.contains("ruff unavailable"));
        assert!(message.contains("/tmp/ruff"));
        assert!(message.contains("signal 9 (SIGKILL)"));
        assert!(message.contains("environment failure, not a lint violation"));
        #[cfg(target_os = "macos")]
        assert!(message.contains("Gatekeeper/quarantine"));
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
            "ruff not available in project, on PATH, or next to court-jester"
        );
    }
}
