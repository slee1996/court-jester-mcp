use std::env;
use std::process::ExitCode;

use court_jester_mcp::types::Language;
use court_jester_mcp::{detect_project_dir, parse_language, tools};

const USAGE: &str = "\
court-jester — code verification CLI for Python and TypeScript

USAGE:
  court-jester verify   [OPTIONS]   Verify a file and print a JSON report
  court-jester analyze  [OPTIONS]   Run tree-sitter analysis
  court-jester lint     [OPTIONS]   Run Ruff or Biome
  court-jester execute  [OPTIONS]   Run code in the sandbox
  court-jester --help               Print this help
  court-jester --version            Print the version

COMMON OPTIONS:
  --file <PATH>              Source file (required for all subcommands)
  --language <LANG>          python | typescript (required)
  --project-dir <PATH>       venv / node_modules root (auto-detected if omitted)
  --config-path <PATH>       Explicit Ruff/Biome config path for lint + verify
  --virtual-file-path <PATH> Virtual lint path for temp or generated source files

VERIFY OPTIONS:
  --test-file <PATH>         Test file to include as an authoritative stage
  --tests-only               Skip fuzz-execute and run only the authoritative test stage
  --output-dir <PATH>        Directory to write persistent JSON reports
  --diff-file <PATH>         Unified-diff file — only inspect changed functions
  --complexity-threshold <N> Fail if any function exceeds this complexity (changed functions only when --diff-file is set)

EXECUTE OPTIONS:
  --timeout-seconds <F>      Sandbox timeout (default 10)
  --memory-mb <N>            Sandbox memory cap MB (default 128)

ENVIRONMENT:
  COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS      Python fuzz-exec timeout (default 10)
  COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS  TS fuzz-exec timeout (default 25)
  COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS        Test stage timeout (default 30)

EXAMPLES:
  court-jester verify --file src/profile.py --language python
  court-jester verify --file src/semver.ts --language typescript \\
      --test-file tests/semver.test.ts --output-dir .court-jester/reports
  court-jester lint --file src/parser.py --language python --config-path pyproject.toml
";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        eprint!("{}", USAGE);
        return ExitCode::from(2);
    }

    match args[0].as_str() {
        "-h" | "--help" => {
            print!("{}", USAGE);
            ExitCode::SUCCESS
        }
        "-V" | "--version" => {
            println!("court-jester {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "verify" | "analyze" | "lint" | "execute" => {
            match run_subcommand(&args[0], &args[1..]).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {}", e);
                    ExitCode::from(2)
                }
            }
        }
        other => {
            eprintln!("error: unknown subcommand '{}'\n", other);
            eprint!("{}", USAGE);
            ExitCode::from(2)
        }
    }
}

#[derive(Default)]
struct CliArgs {
    file: Option<String>,
    language: Option<String>,
    project_dir: Option<String>,
    config_path: Option<String>,
    virtual_file_path: Option<String>,
    test_file: Option<String>,
    tests_only: bool,
    output_dir: Option<String>,
    diff_file: Option<String>,
    complexity_threshold: Option<usize>,
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
}

fn parse_flags(rest: &[String]) -> Result<CliArgs, String> {
    let mut out = CliArgs::default();
    let mut i = 0;
    while i < rest.len() {
        let flag = rest[i].as_str();
        let take_value = |i: &mut usize| -> Result<String, String> {
            if *i + 1 >= rest.len() {
                return Err(format!("flag {} requires a value", flag));
            }
            *i += 1;
            Ok(rest[*i].clone())
        };
        match flag {
            "--file" => out.file = Some(take_value(&mut i)?),
            "--language" => out.language = Some(take_value(&mut i)?),
            "--project-dir" => out.project_dir = Some(take_value(&mut i)?),
            "--config-path" => out.config_path = Some(take_value(&mut i)?),
            "--virtual-file-path" => out.virtual_file_path = Some(take_value(&mut i)?),
            "--test-file" => out.test_file = Some(take_value(&mut i)?),
            "--tests-only" => out.tests_only = true,
            "--output-dir" => out.output_dir = Some(take_value(&mut i)?),
            "--diff-file" => out.diff_file = Some(take_value(&mut i)?),
            "--complexity-threshold" => {
                let raw = take_value(&mut i)?;
                out.complexity_threshold = Some(raw.parse::<usize>().map_err(|_| {
                    format!(
                        "--complexity-threshold must be a non-negative integer, got '{}'",
                        raw
                    )
                })?);
            }
            "--timeout-seconds" => {
                let raw = take_value(&mut i)?;
                out.timeout_seconds =
                    Some(raw.parse::<f64>().map_err(|_| {
                        format!("--timeout-seconds must be a number, got '{}'", raw)
                    })?);
            }
            "--memory-mb" => {
                let raw = take_value(&mut i)?;
                out.memory_mb = Some(raw.parse::<u64>().map_err(|_| {
                    format!("--memory-mb must be a positive integer, got '{}'", raw)
                })?);
            }
            "-h" | "--help" => {
                print!("{}", USAGE);
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag '{}'", other)),
        }
        i += 1;
    }
    Ok(out)
}

fn require_file(args: &CliArgs) -> Result<&str, String> {
    args.file
        .as_deref()
        .ok_or_else(|| "--file is required".to_string())
}

fn require_language(args: &CliArgs) -> Result<Language, String> {
    let raw = args
        .language
        .as_deref()
        .ok_or_else(|| "--language is required".to_string())?;
    parse_language(raw).map_err(|json_error| {
        serde_json::from_str::<serde_json::Value>(&json_error)
            .ok()
            .and_then(|value| value["error"].as_str().map(ToOwned::to_owned))
            .unwrap_or(json_error)
    })
}

fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {}", path, e))
}

fn read_optional_file(path: Option<&str>) -> Result<Option<String>, String> {
    match path {
        Some(path) => Ok(Some(read_file(path)?)),
        None => Ok(None),
    }
}

async fn run_subcommand(cmd: &str, rest: &[String]) -> Result<(), String> {
    let args = parse_flags(rest)?;
    let file = require_file(&args)?.to_string();
    let language = require_language(&args)?;
    let code = read_file(&file)?;
    let project_dir = args
        .project_dir
        .clone()
        .or_else(|| detect_project_dir(&file));

    match cmd {
        "verify" => {
            let test_code = read_optional_file(args.test_file.as_deref())?;
            let diff = read_optional_file(args.diff_file.as_deref())?;
            let opts = tools::verify::VerifyOptions {
                test_code: test_code.as_deref(),
                test_source_file: args.test_file.as_deref(),
                complexity_threshold: args.complexity_threshold,
                project_dir: project_dir.as_deref(),
                lint_config_path: args.config_path.as_deref(),
                lint_virtual_file_path: args.virtual_file_path.as_deref(),
                diff: diff.as_deref(),
                source_file: Some(file.as_str()),
                output_dir: args.output_dir.as_deref(),
                tests_only: args.tests_only,
            };
            let report = tools::verify::verify(&code, &language, opts).await;
            let json = serde_json::to_string_pretty(&report)
                .map_err(|e| format!("failed to serialize verify report: {}", e))?;
            println!("{}", json);
            if !report.overall_ok {
                std::process::exit(1);
            }
            Ok(())
        }
        "analyze" => {
            let analysis = tools::analyze::analyze(&code, &language);
            let mut value = serde_json::to_value(&analysis)
                .map_err(|e| format!("failed to serialize analysis: {}", e))?;
            if let Some(diff) = read_optional_file(args.diff_file.as_deref())? {
                let changed_ranges = tools::diff::parse_changed_lines_for_file(&diff, &file);
                let changed_fns =
                    tools::analyze::filter_changed_functions(&analysis, &changed_ranges);
                value["changed_functions"] = serde_json::to_value(&changed_fns).unwrap();
            }
            if let Some(threshold) = args.complexity_threshold {
                let violations = tools::analyze::check_complexity_threshold(&analysis, threshold);
                value["complexity_violations"] = serde_json::to_value(&violations).unwrap();
                value["complexity_ok"] = serde_json::Value::Bool(violations.is_empty());
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value)
                    .map_err(|e| format!("failed to serialize analysis: {}", e))?
            );
            Ok(())
        }
        "lint" => {
            let result = tools::lint::lint_with_options(
                &code,
                &language,
                tools::lint::LintOptions {
                    source_file: Some(file.as_str()),
                    project_dir: project_dir.as_deref(),
                    config_path: args.config_path.as_deref(),
                    virtual_file_path: args.virtual_file_path.as_deref(),
                },
            )
            .await;
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| format!("failed to serialize lint result: {}", e))?;
            println!("{}", json);
            if result.error.is_some() {
                std::process::exit(1);
            }
            Ok(())
        }
        "execute" => {
            let timeout = args.timeout_seconds.unwrap_or(10.0);
            let memory = args.memory_mb.unwrap_or(128);
            let result = tools::sandbox::execute(
                &code,
                &language,
                timeout,
                memory,
                project_dir.as_deref(),
                Some(file.as_str()),
            )
            .await;
            let json = serde_json::to_string_pretty(&result)
                .map_err(|e| format!("failed to serialize execute result: {}", e))?;
            println!("{}", json);
            if result.exit_code != Some(0) || result.timed_out || result.memory_error {
                std::process::exit(1);
            }
            Ok(())
        }
        _ => unreachable!("unhandled subcommand '{}'", cmd),
    }
}
