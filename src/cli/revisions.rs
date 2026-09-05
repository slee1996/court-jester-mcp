//! Materialize committed bytes without export attributes or checkout filters.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn git(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("--no-replace-objects").current_dir(repo);
    command
}

pub(super) fn resolve_revision(repo: &Path, revision: &str) -> Result<String, String> {
    let output = git(repo)
        .args([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "cannot resolve committed revision {revision}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let oid = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    if !valid_oid(&oid) {
        return Err("git returned an invalid commit identity".into());
    }
    Ok(oid)
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct Entry {
    mode: String,
    oid: String,
    path: PathBuf,
}

fn entries(repo: &Path, revision: &str) -> Result<Vec<Entry>, String> {
    let output = git(repo)
        .args(["ls-tree", "-r", "-z", "--full-tree", revision])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "cannot read revision tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or("invalid git tree record")?;
            let fields = std::str::from_utf8(&record[..tab])
                .map_err(|error| error.to_string())?
                .split(' ')
                .collect::<Vec<_>>();
            let path = PathBuf::from(OsString::from_vec(record[tab + 1..].to_vec()));
            if path.as_os_str().is_empty()
                || path.components().any(
                    |component| !matches!(component, Component::Normal(name) if name != ".git"),
                )
            {
                return Err("revision contains an unsafe tree path".into());
            }
            if fields.len() != 3
                || fields[1] != "blob"
                || !matches!(fields[0], "100644" | "100755" | "120000")
                || !valid_oid(fields[2])
            {
                return Err(format!(
                    "unsupported revision entry {} (submodules must be materialized explicitly)",
                    path.display()
                ));
            }
            Ok(Entry {
                mode: fields[0].into(),
                oid: fields[2].into(),
                path,
            })
        })
        .collect()
}

fn safe_link(path: &Path, target: &Path) -> bool {
    let mut depth = path
        .parent()
        .map(|parent| parent.components().count())
        .unwrap_or(0);
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    !target.as_os_str().is_empty()
}

pub(super) fn materialize(repo: &Path, revision: &str) -> Result<TempDir, String> {
    materialize_at(repo, revision, None)
}

pub(super) fn materialize_at(
    repo: &Path,
    revision: &str,
    parent: Option<&Path>,
) -> Result<TempDir, String> {
    let revision = resolve_revision(repo, revision)?;
    let entries = entries(repo, &revision)?;
    let directory = match parent {
        Some(parent) => tempfile::Builder::new()
            .prefix("candidate-")
            .tempdir_in(parent),
        None => tempfile::tempdir(),
    }
    .map_err(|error| error.to_string())?;
    let mut child = git(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start git blob reader: {error}"))?;
    let mut input = child.stdin.take().ok_or("git blob reader has no input")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("git blob reader has no output")?);
    let result = (|| -> Result<(), String> {
        let mut links = Vec::new();
        for entry in entries {
            writeln!(input, "{}", entry.oid).map_err(|error| error.to_string())?;
            input.flush().map_err(|error| error.to_string())?;
            let mut header = String::new();
            output
                .read_line(&mut header)
                .map_err(|error| error.to_string())?;
            let parts = header.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 3 || parts[0] != entry.oid || parts[1] != "blob" {
                return Err("git blob reader returned an unexpected object".into());
            }
            let size: u64 = parts[2].parse().map_err(|_| "invalid git blob size")?;
            let path = directory.path().join(&entry.path);
            std::fs::create_dir_all(path.parent().ok_or("tree entry has no parent")?)
                .map_err(|error| error.to_string())?;
            if entry.mode == "120000" {
                let mut bytes = Vec::new();
                let count = output
                    .by_ref()
                    .take(size)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if count as u64 != size {
                    return Err("truncated git symlink blob".into());
                }
                let target = PathBuf::from(OsString::from_vec(bytes));
                if !safe_link(&entry.path, &target) || target.as_os_str().as_bytes().contains(&0) {
                    return Err(format!(
                        "revision symlink escapes its workspace: {}",
                        entry.path.display()
                    ));
                }
                links.push((path, target));
            } else {
                let mut file = std::fs::File::create(&path).map_err(|error| error.to_string())?;
                let count = std::io::copy(&mut output.by_ref().take(size), &mut file)
                    .map_err(|error| error.to_string())?;
                if count != size {
                    return Err("truncated git file blob".into());
                }
                file.set_permissions(std::fs::Permissions::from_mode(if entry.mode == "100755" {
                    0o755
                } else {
                    0o644
                }))
                .map_err(|error| error.to_string())?;
            }
            let mut newline = [0];
            output
                .read_exact(&mut newline)
                .map_err(|error| error.to_string())?;
            if newline != [b'\n'] {
                return Err("invalid git blob framing".into());
            }
        }
        // No tree entry is written through a symlink, regardless of tree order.
        for (path, target) in &links {
            std::os::unix::fs::symlink(target, path).map_err(|error| error.to_string())?;
        }
        let root = std::fs::canonicalize(directory.path()).map_err(|error| error.to_string())?;
        for (path, _) in links {
            let resolved = std::fs::canonicalize(&path).map_err(|error| {
                format!(
                    "revision symlink cannot be resolved {}: {error}",
                    path.display()
                )
            })?;
            if !resolved.starts_with(&root) {
                return Err(format!(
                    "revision symlink escapes its workspace: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    })();
    drop(input);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    result?;
    if !status.success() {
        return Err("git blob reader failed".into());
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(repo: &Path, args: &[&str]) {
        let output = git(repo).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> TempDir {
        let repo = tempfile::tempdir().unwrap();
        run(repo.path(), &["init", "--quiet"]);
        run(repo.path(), &["config", "user.name", "Tests"]);
        run(repo.path(), &["config", "user.email", "tests@example.com"]);
        repo
    }

    fn commit(repo: &Path) {
        run(repo, &["add", "."]);
        run(repo, &["commit", "--quiet", "-m", "snapshot"]);
    }

    #[test]
    fn committed_snapshot_preserves_bytes_names_modes_and_internal_links() {
        let repo = repository();
        let root = repo.path();
        let odd = OsString::from("space\tline\nλ.py");
        let bytes = vec![0xa5; 700_000];
        std::fs::write(root.join(&odd), &bytes).unwrap();
        std::fs::write(root.join("tool"), "original\n").unwrap();
        std::fs::set_permissions(root.join("tool"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::os::unix::fs::symlink("../tool", root.join("nested/link")).unwrap();
        commit(root);
        let revision = resolve_revision(root, "HEAD").unwrap();
        std::fs::write(root.join("tool"), "working tree\n").unwrap();
        std::fs::write(root.join("untracked.py"), "untracked\n").unwrap();
        let snapshot = materialize(&root.join("nested"), &revision).unwrap();
        assert_eq!(std::fs::read(snapshot.path().join(&odd)).unwrap(), bytes);
        assert_eq!(
            std::fs::read(snapshot.path().join("tool")).unwrap(),
            b"original\n"
        );
        assert_eq!(
            std::fs::metadata(snapshot.path().join("tool"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::read_link(snapshot.path().join("nested/link")).unwrap(),
            Path::new("../tool")
        );
        assert_eq!(
            std::fs::read(snapshot.path().join("nested/link")).unwrap(),
            b"original\n"
        );
        assert!(!snapshot.path().join("untracked.py").exists());
        assert_eq!(std::fs::read(root.join("tool")).unwrap(), b"working tree\n");
    }

    #[test]
    fn unresolved_and_escaping_links_are_explicit_errors() {
        for target in ["../outside", "/tmp", "missing", "link"] {
            let repo = repository();
            std::os::unix::fs::symlink(target, repo.path().join("link")).unwrap();
            commit(repo.path());
            assert!(materialize(repo.path(), "HEAD")
                .unwrap_err()
                .contains("symlink"));
        }
        let repo = repository();
        std::os::unix::fs::symlink(".", repo.path().join("alias")).unwrap();
        std::os::unix::fs::symlink("alias/..", repo.path().join("escape")).unwrap();
        commit(repo.path());
        assert!(materialize(repo.path(), "HEAD")
            .unwrap_err()
            .contains("escapes"));
    }

    #[test]
    fn revisions_require_commits_and_submodules_do_not_silently_disappear() {
        let repo = repository();
        std::fs::write(repo.path().join("source.py"), "source\n").unwrap();
        commit(repo.path());
        assert!(materialize(repo.path(), "HEAD:source.py").is_err());
        assert!(materialize(repo.path(), "--help").is_err());
        let revision = resolve_revision(repo.path(), "HEAD").unwrap();
        run(
            repo.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{revision},module"),
            ],
        );
        run(repo.path(), &["commit", "--quiet", "-m", "submodule"]);
        assert!(materialize(repo.path(), "HEAD")
            .unwrap_err()
            .contains("submodules"));
    }
}
