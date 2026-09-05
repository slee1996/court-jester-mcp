//! Select one committed workspace before repository configuration is loaded.

use super::args::{CandidateState, CliArgs};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub(super) struct Candidate {
    pub(super) root: PathBuf,
    pub(super) flags: Vec<String>,
    directory: Option<TempDir>,
}

pub(super) fn repo_root(cwd: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("CI requires a Git working tree".into());
    }
    let path = String::from_utf8(output.stdout).map_err(|_| "repository root must be UTF-8")?;
    std::fs::canonicalize(path.strip_suffix('\n').unwrap_or(&path))
        .map_err(|error| error.to_string())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !result.pop() {
                    return Err("path escapes filesystem root".into());
                }
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}

fn mapped(cwd: &Path, repo: &Path, root: &Path, value: &str) -> Result<String, String> {
    let original = normalize(&cwd.join(value))?;
    let relative = original
        .strip_prefix(repo)
        .map_err(|_| "committed inputs must be paths within the invocation repository")?;
    root.join(relative)
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "candidate path must be UTF-8".into())
}

pub(super) fn prepare(
    cmd: &str,
    rest: &[String],
    args: &CliArgs,
) -> Result<Option<Candidate>, String> {
    let positions = super::args::parse_flags_indexed(rest)?.1;
    if args.candidate_state != CandidateState::Committed {
        if cmd != "ci"
            && positions
                .iter()
                .any(|index| rest[*index] == "--candidate-state")
        {
            return Err("--candidate-state supports ci only".into());
        }
        return Ok(None);
    }
    if cmd != "ci" {
        return Err("--candidate-state supports ci only".into());
    }
    if args.repo_config.is_some() && args.no_repo_config {
        return Err("--repo-config conflicts with --no-repo-config".into());
    }
    if !args.show_config {
        super::args::require_base(args)?;
    }
    let cwd = std::fs::canonicalize(std::env::current_dir().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let repo = repo_root(&cwd)?;
    let head = super::revisions::resolve_revision(&repo, args.head.as_deref().unwrap_or("HEAD"))?;
    let base = args
        .base
        .as_deref()
        .map(|base| super::revisions::resolve_revision(&repo, base))
        .transpose()?;
    let output = args
        .output_dir
        .as_deref()
        .map(|path| normalize(&cwd.join(path)))
        .transpose()?;
    if output.is_none() && !args.show_config {
        return Err(
            "committed candidates require --output-dir to preserve the workspace for replay".into(),
        );
    }
    let parent = if args.show_config {
        None
    } else {
        let parent = output.as_ref().unwrap().join("candidate-workspaces");
        std::fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
        Some(parent)
    };
    let directory = super::revisions::materialize_at(&repo, &head, parent.as_deref())?;
    let root = std::fs::canonicalize(directory.path()).map_err(|error| error.to_string())?;
    let mut flags = rest.to_vec();
    for index in positions {
        if matches!(
            flags[index].as_str(),
            "--repo-config"
                | "--project-dir"
                | "--test-file"
                | "--suppressions-file"
                | "--config-path"
                | "--diff-file"
                | "--file"
                | "--base-file"
                | "--base-project-dir"
        ) {
            flags[index + 1] = mapped(&cwd, &repo, &root, &flags[index + 1])?;
        } else if flags[index] == "--output-dir" {
            flags[index + 1] = output.as_ref().unwrap().to_string_lossy().into_owned();
        }
    }
    if args.repo_config.is_none() && !args.no_repo_config {
        let start = if let Some(project) = &args.project_dir {
            PathBuf::from(mapped(&cwd, &repo, &root, project)?)
        } else {
            root.join(cwd.strip_prefix(&repo).map_err(|error| error.to_string())?)
        };
        let mut selected = None;
        for directory in start.ancestors() {
            let config = directory.join(".court-jester.json");
            if std::fs::symlink_metadata(&config).is_ok() {
                selected = Some(config);
                break;
            }
            if args.project_dir.is_some() || directory == root {
                break;
            }
        }
        if let Some(path) = selected {
            flags.extend(["--repo-config".into(), path.to_string_lossy().into_owned()]);
        } else {
            flags.push("--no-repo-config".into());
        }
    }
    // Without config, use the selected source workspace as the runtime root.
    if args.project_dir.is_none() && super::args::parse_flags(&flags)?.repo_config.is_none() {
        flags.extend(["--project-dir".into(), root.to_string_lossy().into_owned()]);
    }
    flags.extend(["--head".into(), head]);
    if let Some(base) = base {
        flags.extend(["--base".into(), base]);
    }
    Ok(Some(Candidate {
        root,
        flags,
        directory: Some(directory),
    }))
}

impl Candidate {
    pub(super) fn validate(&self, args: &CliArgs) -> Result<(), String> {
        for path in args
            .repo_config
            .iter()
            .chain(args.project_dir.iter())
            .chain(args.config_path.iter())
            .chain(args.suppressions_file.iter())
            .chain(args.test_files.iter())
        {
            let resolved = std::fs::canonicalize(path)
                .map_err(|error| format!("committed input unavailable {path}: {error}"))?;
            if !resolved.starts_with(&self.root) {
                return Err(
                    "committed configuration and inputs must stay inside the candidate workspace"
                        .into(),
                );
            }
        }
        super::config::validate_target_boundary(&args.config_targets, &self.root)
    }

    pub(super) fn persist(&mut self) {
        if let Some(directory) = self.directory.take() {
            let _ = directory.keep();
        }
    }
}
