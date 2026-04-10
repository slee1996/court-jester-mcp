use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::types::*;

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
        let candidate = std::path::Path::new(dir).join(binary);
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
    let candidate = dir.join(binary);
    if candidate.is_file() {
        Some(candidate.to_string_lossy().to_string())
    } else {
        None
    }
}

fn resolve_binary(path_env: &str, binary: &str, exe_dir: Option<&Path>) -> Option<String> {
    find_binary_near_exe_dir(exe_dir, binary).or_else(|| find_binary_on_path(path_env, binary))
}

pub async fn lint(code: &str, language: &Language) -> LintResult {
    let tmpdir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return LintResult {
                diagnostics: vec![],
                error: Some(format!("Failed to create temp dir: {e}")),
                unavailable: false,
            }
        }
    };

    match language {
        Language::Python => lint_python(code, &tmpdir).await,
        Language::TypeScript => lint_typescript(code, &tmpdir).await,
    }
}

fn tool_failure_message(
    tool: &str,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
) -> String {
    let exit = status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "no output"
    };
    format!("{tool} failed with exit status {exit}: {detail}")
}

async fn lint_python(code: &str, tmpdir: &tempfile::TempDir) -> LintResult {
    let file_path = tmpdir.path().join("snippet.py");
    if let Err(e) = std::fs::write(&file_path, code) {
        return LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to write temp file: {e}")),
            unavailable: false,
        };
    }

    let output = Command::new("ruff")
        .args(["check", "--output-format=json", file_path.to_str().unwrap()])
        .env("PATH", extended_path())
        .stdin(Stdio::null())
        .output()
        .await;

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

async fn lint_typescript(code: &str, tmpdir: &tempfile::TempDir) -> LintResult {
    let file_path = tmpdir.path().join("snippet.ts");
    if let Err(e) = std::fs::write(&file_path, code) {
        return LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to write temp file: {e}")),
            unavailable: false,
        };
    }

    let path = extended_path();
    let exe_dir = current_exe_dir();
    let Some(biome) = resolve_binary(&path, "biome", exe_dir.as_deref()) else {
        return LintResult {
            diagnostics: vec![],
            error: Some("biome not available on PATH or next to court-jester-mcp".to_string()),
            unavailable: true,
        };
    };

    let output = Command::new(&biome)
        .args(["lint", "--reporter=json", file_path.to_str().unwrap()])
        .env("PATH", &path)
        .stdin(Stdio::null())
        .output()
        .await;

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
    use super::{find_binary_near_exe_dir, resolve_binary};
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
        )
        .expect("biome should resolve");

        assert_eq!(resolved, sibling.to_string_lossy());
    }

    #[test]
    fn find_binary_near_exe_dir_returns_none_when_missing() {
        let sibling_dir = tempfile::tempdir().unwrap();
        let resolved = find_binary_near_exe_dir(Some(sibling_dir.path()), "biome");
        assert!(resolved.is_none());
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
