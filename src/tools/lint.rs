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

pub async fn lint(code: &str, language: &Language) -> LintResult {
    let tmpdir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return LintResult {
                diagnostics: vec![],
                error: Some(format!("Failed to create temp dir: {e}")),
            }
        }
    };

    match language {
        Language::Python => lint_python(code, &tmpdir).await,
        Language::TypeScript => lint_typescript(code, &tmpdir).await,
    }
}

async fn lint_python(code: &str, tmpdir: &tempfile::TempDir) -> LintResult {
    let file_path = tmpdir.path().join("snippet.py");
    if let Err(e) = std::fs::write(&file_path, code) {
        return LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to write temp file: {e}")),
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
            parse_ruff_output(&stdout)
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("ruff not available: {e}")),
        },
    }
}

fn parse_ruff_output(output: &str) -> LintResult {
    if output.trim().is_empty() {
        return LintResult {
            diagnostics: vec![],
            error: None,
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
            }
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to parse ruff output: {e}")),
        },
    }
}

async fn lint_typescript(code: &str, tmpdir: &tempfile::TempDir) -> LintResult {
    let file_path = tmpdir.path().join("snippet.ts");
    if let Err(e) = std::fs::write(&file_path, code) {
        return LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to write temp file: {e}")),
        };
    }

    let output = Command::new("npx")
        .args([
            "@biomejs/biome",
            "lint",
            "--reporter=json",
            file_path.to_str().unwrap(),
        ])
        .env("PATH", extended_path())
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
            parse_biome_output(&text)
        }
        Err(e) => LintResult {
            diagnostics: vec![],
            error: Some(format!("biome not available: {e}")),
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
            error: None,
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
            }
        }
        Err(_) => LintResult {
            diagnostics: vec![],
            error: Some(format!("Failed to parse biome output")),
        },
    }
}
