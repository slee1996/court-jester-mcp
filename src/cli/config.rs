//! Versioned repository defaults. Replay intentionally uses recorded context.

use super::args::{parse_flags, validate_runtime_flags, CliArgs};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryConfig {
    schema_version: u32,
    #[serde(default)]
    defaults: Defaults,
    #[serde(default)]
    targets: Vec<TargetConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetConfig {
    source: String,
    test_files: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct TargetTests {
    source: PathBuf,
    tests: Vec<String>,
}

pub(super) fn selected_settings(cmd: &str, args: &CliArgs) -> serde_json::Value {
    let _timeout = super::environment::VerifyTimeoutEnv::install(args.timeout_seconds);
    serde_json::json!({
        "schema_version": 1,
        "command": cmd,
        "execution_started": false,
        "config_path": args.repo_config,
        "config_state": "working_tree",
        "discovery_disabled": args.no_repo_config,
        "source_file": args.file,
        "language": args.language,
        "project_dir_override": args.project_dir,
        "test_files": args.test_files,
        "source_test_mappings": args.config_targets,
        "test_runner": args.test_runner,
        "suppressions_file": args.suppressions_file,
        "limits": {
            "memory_mb": args.verification_memory_mb(),
            "timeouts": court_jester::tools::verify::verification_timeouts(),
            "scope": "ordinary_verification; specialized adapters may use different budgets"
        },
        "runtime_profile": args.runtime_profile,
        "network": args.network,
        "coverage_gate": args.coverage_gate,
        "execute_gate": args.execute_gate,
        "inferred_oracle_gate": args.inferred_oracle_gate,
        "test_quality_max_mutants": args.test_quality_max_mutants,
        "readiness_checked": false
    })
}

pub(super) fn mapped_tests<'a>(
    targets: &'a [TargetTests],
    source: &Path,
) -> Result<Option<&'a [String]>, String> {
    if targets.is_empty() {
        return Ok(None);
    }
    let source = std::fs::canonicalize(source)
        .map_err(|error| format!("mapped source unavailable: {error}"))?;
    Ok(targets
        .iter()
        .find(|target| target.source == source)
        .map(|target| target.tests.as_slice()))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    timeout_seconds: Option<f64>,
    memory_mb: Option<u64>,
    runtime_profile: Option<String>,
    test_runner: Option<String>,
    coverage_gate: Option<String>,
    execute_gate: Option<String>,
    inferred_oracle_gate: Option<String>,
    #[serde(default)]
    test_files: Vec<String>,
    suppressions_file: Option<String>,
}

pub(super) fn apply(cmd: &str, rest: &[String], explicit: CliArgs) -> Result<CliArgs, String> {
    if explicit.no_repo_config && explicit.repo_config.is_some() {
        return Err("--repo-config conflicts with --no-repo-config".into());
    }
    if !matches!(cmd, "verify" | "ci" | "doctor") {
        return if explicit.repo_config.is_some() {
            Err("--repo-config supports verify, ci, and doctor".into())
        } else {
            Ok(explicit)
        };
    }
    let selected = if let Some(path) = &explicit.repo_config {
        Some(PathBuf::from(path))
    } else if explicit.no_repo_config {
        None
    } else {
        discover(cmd, &explicit)?
    };
    let Some(path) = selected else {
        return Ok(explicit);
    };
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("repository config unavailable: {error}"))?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("repository config unreadable: {error}"))?;
    let config: RepositoryConfig = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid repository config {}: {error}", path.display()))?;
    if config.schema_version != 1 {
        return Err(format!(
            "unsupported repository config schema_version {} (expected 1)",
            config.schema_version
        ));
    }
    let root = path
        .parent()
        .ok_or("repository config has no parent directory")?;
    let mut targets = Vec::new();
    let mut sources = std::collections::HashSet::new();
    for target in config.targets {
        let source = std::fs::canonicalize(configured_path(root, &target.source)?)
            .map_err(|error| format!("configured target {} unavailable: {error}", target.source))?;
        if !source.is_file() {
            return Err("configured target must be a source file".into());
        }
        if !sources.insert(source.clone()) {
            return Err("duplicate configured source target".into());
        }
        if target.test_files.is_empty() {
            return Err("configured target must specify at least one test file".into());
        }
        let tests = target
            .test_files
            .iter()
            .map(|file| configured_path(root, file))
            .collect::<Result<Vec<_>, _>>()?;
        targets.push(TargetTests { source, tests });
    }
    let defaults = config.defaults;
    let mut flags = Vec::new();
    let mut flag = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            flags.extend([name.to_string(), value]);
        }
    };
    flag(
        "--timeout-seconds",
        defaults.timeout_seconds.map(|value| value.to_string()),
    );
    flag(
        "--memory-mb",
        defaults.memory_mb.map(|value| value.to_string()),
    );
    flag("--runtime-profile", defaults.runtime_profile);
    flag("--test-runner", defaults.test_runner);
    flag("--coverage-gate", defaults.coverage_gate);
    flag("--execute-gate", defaults.execute_gate);
    flag("--inferred-oracle-gate", defaults.inferred_oracle_gate);
    // Validate defaults even when the CLI would override them. Typos and invalid
    // budgets must not become latent failures on a subsequent invocation.
    let parsed =
        parse_flags(&flags).map_err(|error| format!("invalid repository defaults: {error}"))?;
    validate_runtime_flags(cmd, &parsed)
        .map_err(|error| format!("invalid repository defaults: {error}"))?;
    if explicit.project_dir.is_none() {
        flags.extend(["--project-dir".into(), path_text(root)?]);
    }
    if explicit.test_files.is_empty() {
        let mapped = if matches!(cmd, "verify" | "doctor") {
            explicit
                .file
                .as_deref()
                .map(|file| mapped_tests(&targets, Path::new(file)))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let tests = if let Some(tests) = mapped {
            tests.to_vec()
        } else {
            defaults
                .test_files
                .iter()
                .map(|file| configured_path(root, file))
                .collect::<Result<Vec<_>, _>>()?
        };
        for file in tests {
            flags.extend(["--test-file".into(), file]);
        }
    }
    if explicit.suppressions_file.is_none() {
        if let Some(file) = defaults.suppressions_file {
            flags.extend(["--suppressions-file".into(), configured_path(root, &file)?]);
        }
    }
    flags.extend_from_slice(rest);
    let mut effective = parse_flags(&flags)?;
    effective.repo_config = Some(path_text(&path)?);
    // Explicit test flags override every mapping, including CI per-file routing.
    if explicit.test_files.is_empty() {
        effective.config_targets = targets;
    }
    Ok(effective)
}

fn discover(cmd: &str, args: &CliArgs) -> Result<Option<PathBuf>, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    // An explicit dependency root is a hard discovery boundary. In CI the
    // selected Git repository remains the invocation repository.
    let start = if let Some(project) = &args.project_dir {
        cwd.join(project)
    } else if matches!(cmd, "verify" | "doctor") {
        args.file
            .as_deref()
            .map(|file| cwd.join(file))
            .and_then(|file| file.parent().map(Path::to_path_buf))
            .unwrap_or(cwd)
    } else {
        cwd
    };
    // Discovery must not replace source/project validation with an unrelated
    // config error when there is no directory in which to discover anything.
    if !start.is_dir() {
        return Ok(None);
    }
    let start = std::fs::canonicalize(start)
        .map_err(|error| format!("repository config discovery root unavailable: {error}"))?;
    let boundary = if args.project_dir.is_some() {
        start.as_path()
    } else {
        start
            .ancestors()
            .find(|path| path.join(".git").exists())
            .unwrap_or(&start)
    };
    for directory in start.ancestors() {
        let candidate = directory.join(".court-jester.json");
        if std::fs::symlink_metadata(&candidate).is_ok() {
            return Ok(Some(candidate));
        }
        if directory == boundary {
            break;
        }
    }
    Ok(None)
}

fn configured_path(root: &Path, value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("repository config paths must not be empty".into());
    }
    path_text(&root.join(value))
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "repository config path must be UTF-8".into())
}
