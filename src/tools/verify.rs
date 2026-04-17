use std::time::Instant;

use crate::tools::{analyze, diff, lint, sandbox, synthesize};
use crate::types::*;

/// Options for the verify pipeline to avoid parameter sprawl.
pub struct VerifyOptions<'a> {
    pub test_code: Option<&'a str>,
    pub test_source_file: Option<&'a str>,
    pub tests_only: bool,
    pub complexity_threshold: Option<usize>,
    pub project_dir: Option<&'a str>,
    pub lint_config_path: Option<&'a str>,
    pub lint_virtual_file_path: Option<&'a str>,
    pub diff: Option<&'a str>,
    /// Original source file path — when set, fuzz code is written as a sibling
    /// so relative imports resolve correctly.
    pub source_file: Option<&'a str>,
    pub output_dir: Option<&'a str>,
}

/// Default execute-stage timeout for synthesized Python fuzz harnesses (seconds).
/// Overridable via `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS`.
const DEFAULT_PYTHON_EXEC_TIMEOUT: f64 = 10.0;

/// Default execute-stage timeout for synthesized TypeScript fuzz harnesses (seconds).
/// TypeScript is slower to boot (bun/tsx startup + transpile), so it gets a longer
/// default. Overridable via `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS`.
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

/// Run the full verification pipeline: parse → complexity → lint → synthesize+execute → test.
pub async fn verify(
    code: &str,
    language: &Language,
    opts: VerifyOptions<'_>,
) -> VerificationReport {
    let mut stages = vec![];
    let mut overall_ok = true;

    // Stage 1: Parse / Analyze
    let start = Instant::now();
    let analysis = analyze::analyze(code, language);
    let parse_ms = start.elapsed().as_millis() as u64;

    if analysis.parse_error {
        stages.push(VerificationStage {
            name: "parse".into(),
            ok: false,
            duration_ms: parse_ms,
            detail: Some(serde_json::to_value(&analysis).unwrap()),
            error: Some("Code contains syntax errors".into()),
        });

        // Persist report if output_dir is set
        let report_path = if let Some(dir) = opts.output_dir {
            write_report(dir, &stages, false, opts.source_file, language)
        } else {
            None
        };

        return VerificationReport {
            stages,
            overall_ok: false,
            report_path,
        };
    }

    stages.push(VerificationStage {
        name: "parse".into(),
        ok: true,
        duration_ms: parse_ms,
        detail: Some(serde_json::to_value(&analysis).unwrap()),
        error: None,
    });

    // Stage 2: Complexity threshold (optional)
    if let Some(threshold) = opts.complexity_threshold {
        let start = Instant::now();
        let (functions_checked, diff_scoped) = if let Some(diff_str) = opts.diff {
            let changed_ranges = opts
                .source_file
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            (
                analyze::filter_changed_functions(&analysis, &changed_ranges),
                true,
            )
        } else {
            (analysis.functions.clone(), false)
        };
        let violations =
            analyze::check_complexity_threshold_for_functions(&functions_checked, threshold);
        let complexity_ms = start.elapsed().as_millis() as u64;
        let complexity_ok = violations.is_empty();
        if !complexity_ok {
            overall_ok = false;
        }
        stages.push(VerificationStage {
            name: "complexity".into(),
            ok: complexity_ok,
            duration_ms: complexity_ms,
            detail: Some(serde_json::json!({
                "violations": serde_json::to_value(&violations).unwrap(),
                "threshold": threshold,
                "checked_functions": functions_checked.len(),
                "diff_scoped": diff_scoped,
                "complexity_ok": complexity_ok,
            })),
            error: if complexity_ok {
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
            source_file: opts.source_file,
            project_dir: opts.project_dir,
            config_path: opts.lint_config_path,
            virtual_file_path: opts.lint_virtual_file_path,
        },
    )
    .await;
    let lint_ms = start.elapsed().as_millis() as u64;

    // Filter out false positives only when linting anonymous inline snippets.
    if opts.source_file.is_none() && opts.lint_virtual_file_path.is_none() {
        lint_result.diagnostics.retain(|d| {
            !matches!(
                d.rule.as_str(),
                "lint/correctness/noUnusedVariables" | "F401" | "F841"
            )
        });
    }

    let lint_runner_failed = lint_result.error.is_some() && !lint_result.unavailable;
    let lint_ok = !lint_runner_failed;

    stages.push(VerificationStage {
        name: "lint".into(),
        ok: lint_ok,
        duration_ms: lint_ms,
        detail: Some(serde_json::to_value(&lint_result).unwrap()),
        error: if lint_runner_failed {
            lint_result.error.clone()
        } else {
            None
        },
    });

    if opts.tests_only && opts.test_code.is_none() {
        overall_ok = false;
        stages.push(VerificationStage {
            name: "test".into(),
            ok: false,
            duration_ms: 0,
            detail: None,
            error: Some("tests_only mode requires an authoritative test".into()),
        });
        let report_path = if let Some(dir) = opts.output_dir {
            write_report(dir, &stages, overall_ok, opts.source_file, language)
        } else {
            None
        };
        return VerificationReport {
            stages,
            overall_ok,
            report_path,
        };
    }

    // Stage 4: Synthesize + Execute
    if !opts.tests_only && !analysis.functions.is_empty() {
        // Determine which functions to fuzz
        let functions_to_fuzz: Vec<FunctionInfo> = if let Some(diff_str) = opts.diff {
            let changed_ranges = opts
                .source_file
                .map(|path| diff::parse_changed_lines_for_file(diff_str, path))
                .unwrap_or_else(|| diff::parse_changed_lines(diff_str));
            analyze::filter_changed_functions(&analysis, &changed_ranges)
        } else {
            analysis.functions.clone()
        };

        // Resolve imported types so the fuzzer can construct proper objects
        let mut all_classes = analysis.classes.clone();
        let mut all_aliases = analysis.aliases.clone();
        if let Some(src) = opts.source_file {
            let referenced_names = analyze::referenced_type_names_for_functions(&functions_to_fuzz);
            let imported = analyze::resolve_imported_types_for_names(
                &analysis,
                src,
                language,
                &referenced_names,
            );
            all_classes.extend(imported.classes);
            all_aliases.extend(imported.aliases);
        }

        let synth_code = synthesize::synthesize_calls_for(
            &functions_to_fuzz,
            &all_classes,
            &all_aliases,
            language,
        );

        if !synth_code.is_empty() {
            let full_code = format!("{code}\n{synth_code}");
            let execute_timeout = execute_timeout_for(language);

            let start = Instant::now();
            let exec_result = sandbox::execute(
                &full_code,
                language,
                execute_timeout,
                512,
                opts.project_dir,
                opts.source_file,
            )
            .await;
            let exec_ms = start.elapsed().as_millis() as u64;

            let exec_ok = exec_result.exit_code == Some(0)
                && !exec_result.timed_out
                && !exec_result.memory_error;
            if !exec_ok {
                overall_ok = false;
            }

            let mut detail = serde_json::to_value(&exec_result).unwrap();
            if let Some(failures) = parse_fuzz_failures(&exec_result.stdout) {
                detail["fuzz_failures"] = serde_json::to_value(&failures).unwrap();
            }

            stages.push(VerificationStage {
                name: "execute".into(),
                ok: exec_ok,
                duration_ms: exec_ms,
                detail: Some(detail),
                error: if exec_ok {
                    None
                } else {
                    Some(exec_result.stderr.clone())
                },
            });
        }
    }

    // Stage 5: Test (if test_code provided) — this IS authoritative
    if let Some(tests) = opts.test_code {
        let has_import_statements = test_code_has_imports(tests, language);
        let mut test_file_for_execution = opts.test_source_file.or(opts.source_file);
        if !has_import_statements {
            if let Some(source_file) = opts.source_file {
                test_file_for_execution = Some(source_file);
            }
        }

        // Test files in this benchmark can include direct symbol assertions against the
        // module under test (e.g., `displayInitials(...)`). In those cases, include the
        // candidate source so the assertions execute in a valid lexical scope.
        let test_input = if !has_import_statements {
            format!("{code}\n\n{tests}")
        } else {
            tests.to_string()
        };

        let start = Instant::now();
        let test_result = sandbox::execute(
            &test_input,
            language,
            test_timeout(),
            512,
            opts.project_dir,
            test_file_for_execution,
        )
        .await;
        let test_ms = start.elapsed().as_millis() as u64;

        let has_assertion_failure = test_result.stderr.contains("Assertion failed")
            || test_result.stderr.contains("AssertionError");
        let test_ok =
            test_result.exit_code == Some(0) && !test_result.timed_out && !has_assertion_failure;
        if !test_ok {
            overall_ok = false;
        }

        stages.push(VerificationStage {
            name: "test".into(),
            ok: test_ok,
            duration_ms: test_ms,
            detail: Some(serde_json::to_value(&test_result).unwrap()),
            error: if test_ok {
                None
            } else {
                Some(test_result.stderr.clone())
            },
        });
    }

    // Persist report if output_dir is set
    let report_path = if let Some(dir) = opts.output_dir {
        write_report(dir, &stages, overall_ok, opts.source_file, language)
    } else {
        None
    };

    VerificationReport {
        stages,
        overall_ok,
        report_path,
    }
}

fn write_report(
    output_dir: &str,
    stages: &[VerificationStage],
    overall_ok: bool,
    source_file: Option<&str>,
    language: &Language,
) -> Option<String> {
    use chrono::Utc;

    // Ensure output dir exists
    let _ = std::fs::create_dir_all(output_dir);

    // Compute summary from stages
    let mut summary = ReportSummary {
        functions_analyzed: 0,
        functions_fuzzed: 0,
        fuzz_pass: 0,
        fuzz_crash: 0,
        lint_issues: 0,
        complexity_violations: 0,
    };

    let mut total_duration = 0u64;

    for stage in stages {
        total_duration += stage.duration_ms;
        if let Some(detail) = &stage.detail {
            match stage.name.as_str() {
                "parse" => {
                    if let Some(funcs) = detail.get("functions") {
                        if let Some(arr) = funcs.as_array() {
                            summary.functions_analyzed = arr.len();
                        }
                    }
                }
                "execute" => {
                    // Count fuzz results from stdout
                    if let Some(stdout) = detail.get("stdout").and_then(|v| v.as_str()) {
                        for line in stdout.lines() {
                            if line.starts_with("FUZZ ") {
                                summary.functions_fuzzed += 1;
                                if line.contains("CRASHED") {
                                    summary.fuzz_crash += 1;
                                } else if line.contains("nothing tested") {
                                    // Rejected-only runs are verifier failures, not passes.
                                } else {
                                    summary.fuzz_pass += 1;
                                }
                            }
                        }
                    }
                }
                "lint" => {
                    if let Some(diags) = detail.get("diagnostics") {
                        if let Some(arr) = diags.as_array() {
                            summary.lint_issues = arr.len();
                        }
                    }
                }
                "complexity" => {
                    if let Some(violations) = detail.get("violations") {
                        if let Some(arr) = violations.as_array() {
                            summary.complexity_violations = arr.len();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let now = Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let file_timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();

    let report = PersistedReport {
        meta: ReportMeta {
            source_file: source_file.map(|s| s.to_string()),
            language: format!("{:?}", language).to_lowercase(),
            timestamp,
            duration_ms: total_duration,
        },
        stages: stages.to_vec(),
        overall_ok,
        summary,
    };

    // Derive filename
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

    match serde_json::to_string_pretty(&report) {
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

/// Parse structured fuzz failure JSON from stdout using the sentinel marker.
pub fn parse_fuzz_failures(stdout: &str) -> Option<Vec<FuzzFailure>> {
    let marker = "__COURT_JESTER_FUZZ_JSON__";
    let idx = stdout.rfind(marker)?;
    let json_str = stdout[idx + marker.len()..].trim();
    serde_json::from_str(json_str).ok()
}
