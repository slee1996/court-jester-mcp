/// Describe the in-memory instrumentation overlay used for authoritative tests.
/// The original source is never rewritten; unsupported runner/import modes are
/// explicit and must be treated as inconclusive by verification.
pub fn build_instrumentation_overlay(
    language: &Language,
    runner: TestRunner,
    source_file: &str,
    surfaces: &[String],
) -> InstrumentationOverlay {
    let (mode, supported) = match language {
        Language::Python => (InstrumentationMode::PythonSitecustomize, true),
        Language::TypeScript => match runner {
            TestRunner::Bun => (InstrumentationMode::BunPreload, true),
            TestRunner::Node | TestRunner::Auto => (InstrumentationMode::NodeModuleRegister, true),
            TestRunner::RepoNative => (InstrumentationMode::Unsupported, false),
        },
    };
    InstrumentationOverlay {
        mode,
        source_file: source_file.to_string(),
        surfaces: surfaces.to_vec(),
        supported,
        reason: (!supported)
            .then(|| "repo-native runner does not expose a module transform hook".into()),
    }
}

use std::time::Instant;
use tokio::process::Command;

use crate::types::*;

#[cfg(target_os = "macos")]
fn get_rss_bytes(pid: u32) -> u64 {
    use std::mem;
    const PROC_PIDTASKINFO: i32 = 4;

    #[repr(C)]
    struct ProcTaskInfo {
        pti_virtual_size: u64,
        pti_resident_size: u64,
        pti_total_user: u64,
        pti_total_system: u64,
        pti_threads_user: u64,
        pti_threads_system: u64,
        pti_policy: i32,
        pti_faults: i32,
        pti_pageins: i32,
        pti_cow_faults: i32,
        pti_messages_sent: i32,
        pti_messages_received: i32,
        pti_syscalls_mach: i32,
        pti_syscalls_unix: i32,
        pti_csw: i32,
        pti_threadnum: i32,
        pti_numrunning: i32,
        pti_priority: i32,
    }

    unsafe {
        let mut info: ProcTaskInfo = mem::zeroed();
        let size = mem::size_of::<ProcTaskInfo>() as i32;
        unsafe extern "C" {
            fn proc_pidinfo(
                pid: i32,
                flavor: i32,
                arg: u64,
                buffer: *mut libc::c_void,
                buffersize: i32,
            ) -> i32;
        }
        let ret = proc_pidinfo(
            pid as i32,
            PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret > 0 {
            info.pti_resident_size
        } else {
            0
        }
    }
}

#[cfg(target_os = "macos")]
fn get_process_group_rss_bytes(pgid: u32) -> u64 {
    unsafe {
        unsafe extern "C" {
            fn proc_listpgrppids(
                pgrpid: libc::pid_t,
                buffer: *mut libc::c_void,
                buffersize: i32,
            ) -> i32;
        }

        let mut pids = vec![0i32; 256];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
        let filled = proc_listpgrppids(pgid as i32, pids.as_mut_ptr() as *mut libc::c_void, bytes);
        if filled <= 0 {
            return 0;
        }

        let pid_count = (filled as usize).min(pids.len());
        pids[..pid_count]
            .iter()
            .filter_map(|pid| (*pid > 0).then_some(*pid as u32))
            .map(get_rss_bytes)
            .sum()
    }
}

#[cfg(target_os = "linux")]
fn get_rss_bytes(pid: u32) -> u64 {
    let status = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn get_process_group_rss_bytes(pgid: u32) -> u64 {
    let mut total = 0;
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let pid = match entry.file_name().to_string_lossy().parse::<u32>() {
            Ok(pid) => pid,
            Err(_) => continue,
        };
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(_) => continue,
        };
        let (_, rest) = match stat.split_once(") ") {
            Some(parts) => parts,
            None => continue,
        };
        let mut fields = rest.split_whitespace();
        let _state = fields.next();
        let _ppid = fields.next();
        let row_pgid = match fields.next().and_then(|value| value.parse::<u32>().ok()) {
            Some(row_pgid) => row_pgid,
            None => continue,
        };
        if row_pgid == pgid {
            total += get_rss_bytes(pid);
        }
    }

    total
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_process_group_rss_bytes(_pgid: u32) -> u64 {
    0
}

/// Find a binary on the given PATH, returning its absolute path if found.
fn which_binary(path_env: &str, binary: &str) -> Option<String> {
    for dir in path_env.split(':') {
        let candidate = std::path::Path::new(dir).join(binary);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

/// RAII guard that deletes files on drop (for sibling fuzz files).
struct CleanupFiles {
    paths: Vec<std::path::PathBuf>,
}

impl Drop for CleanupFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Check if code contains Python relative imports (from .module import ...).
fn has_python_relative_imports(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("from .") && trimmed.contains("import")
    })
}

fn is_valid_python_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn has_typescript_relative_imports(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("from \"./")
            || trimmed.contains("from './")
            || trimmed.contains("from \"../")
            || trimmed.contains("from '../")
    })
}

fn has_typescript_module_dependencies(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("import ")
            || trimmed.contains("require(")
            || trimmed.starts_with("export * from ")
            || (trimmed.starts_with("export {") && trimmed.contains(" from "))
    })
}

pub fn typescript_code_requires_bun_runtime(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("Bun.")
            || trimmed.contains("typeof Bun")
            || trimmed.contains("instanceof Bun")
            || trimmed.contains("from \"bun\"")
            || trimmed.contains("from 'bun'")
            || trimmed.contains("from \"bun:")
            || trimmed.contains("from 'bun:")
            || trimmed.contains("import \"bun\"")
            || trimmed.contains("import 'bun'")
            || trimmed.contains("require(\"bun\")")
            || trimmed.contains("require('bun')")
    })
}

fn dir_declares_bun_package_manager(dir: &std::path::Path) -> bool {
    let package_json = dir.join("package.json");
    std::fs::read_to_string(package_json)
        .map(|text| text.contains("\"packageManager\"") && text.contains("bun@"))
        .unwrap_or(false)
}

pub fn detect_repo_typescript_runner(
    project_dir: Option<&str>,
    source_file: Option<&str>,
) -> Option<String> {
    let mut starts = Vec::new();
    if let Some(dir) = project_dir {
        starts.push(std::path::PathBuf::from(dir));
    }
    if let Some(source_file) = source_file {
        if let Some(parent) = std::path::Path::new(source_file).parent() {
            starts.push(parent.to_path_buf());
        }
    }

    for start in starts {
        let mut dir = start.as_path();
        loop {
            if dir.join("bun.lock").exists()
                || dir.join("bun.lockb").exists()
                || dir_declares_bun_package_manager(dir)
            {
                return Some("bun".into());
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }
    }

    None
}

fn parse_quoted_path(input: &str) -> Option<String> {
    let quote = input.chars().find(|c| *c == '"' || *c == '\'')?;
    let start = input.find(quote)? + 1;
    let end = start + input[start..].find(quote)?;
    Some(input[start..end].to_string())
}

fn resolve_typescript_import_file(
    source_file: &str,
    import_path: &str,
) -> Option<std::path::PathBuf> {
    let source_dir = std::path::Path::new(source_file).parent()?;
    let base = source_dir.join(import_path);

    if base.exists() {
        return Some(base);
    }
    for ext in [".ts", ".tsx", "/index.ts", "/index.tsx"] {
        let candidate = std::path::PathBuf::from(format!("{}{}", base.display(), ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
fn extract_typescript_named_relative_imports(code: &str) -> Vec<(String, Vec<String>)> {
    let mut imports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("import ") || trimmed.starts_with("import type ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };
        let names = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry = entry.strip_prefix("type ").unwrap_or(entry);
                let export_name = entry
                    .split_once(" as ")
                    .map(|(name, _)| name)
                    .unwrap_or(entry)
                    .trim();
                (!export_name.is_empty()).then(|| export_name.to_string())
            })
            .collect::<Vec<_>>();
        if !names.is_empty() {
            imports.push((path, names));
        }
    }

    imports
}

#[derive(Clone, Debug)]
struct RelativeReexportSpecifier {
    source_name: String,
    exported_name: String,
    type_only: bool,
}

fn extract_typescript_named_relative_reexports(
    code: &str,
) -> Vec<(String, Vec<RelativeReexportSpecifier>)> {
    let mut reexports = Vec::new();

    for statement in code.split(';') {
        let normalized = statement.replace('\n', " ");
        let trimmed = normalized.trim();
        if !trimmed.starts_with("export ") {
            continue;
        }
        let (clause, from_clause) = match trimmed[7..].split_once(" from ") {
            Some(parts) => parts,
            None => continue,
        };
        let path = match parse_quoted_path(from_clause) {
            Some(path) if path.starts_with("./") || path.starts_with("../") => path,
            _ => continue,
        };

        let clause = clause.trim();
        let statement_type_only = clause.starts_with("type ");
        let open = match clause.find('{') {
            Some(index) => index,
            None => continue,
        };
        let close = match clause.rfind('}') {
            Some(index) => index,
            None => continue,
        };

        let specifiers = clause[open + 1..close]
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let entry_type_only = statement_type_only || entry.starts_with("type ");
                let entry = entry.strip_prefix("type ").unwrap_or(entry).trim();
                let (source_name, exported_name) = entry
                    .split_once(" as ")
                    .map(|(source_name, exported_name)| (source_name.trim(), exported_name.trim()))
                    .unwrap_or((entry, entry));
                if source_name.is_empty() || exported_name.is_empty() {
                    return None;
                }
                Some(RelativeReexportSpecifier {
                    source_name: source_name.to_string(),
                    exported_name: exported_name.to_string(),
                    type_only: entry_type_only,
                })
            })
            .collect::<Vec<_>>();
        if !specifiers.is_empty() {
            reexports.push((path, specifiers));
        }
    }

    reexports
}

fn source_key(source_file: &str) -> String {
    std::fs::canonicalize(source_file)
        .unwrap_or_else(|_| std::path::PathBuf::from(source_file))
        .to_string_lossy()
        .to_string()
}

fn target_exports_name_only_as_type(
    code: &str,
    source_file: &str,
    name: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let visit_key = format!("{}::{name}", source_key(source_file));
    if !visited.insert(visit_key) {
        return false;
    }

    let type_alias_prefix = format!("export type {name}");
    let interface_prefix = format!("export interface {name}");

    if code.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(&type_alias_prefix) || trimmed.starts_with(&interface_prefix)
    }) {
        return true;
    }

    extract_typescript_named_relative_reexports(code)
        .into_iter()
        .any(|(import_path, specifiers)| {
            specifiers.into_iter().any(|specifier| {
                if specifier.exported_name != name {
                    return false;
                }
                if specifier.type_only {
                    return true;
                }
                let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                    Some(path) => path,
                    None => return false,
                };
                let imported_code = match std::fs::read_to_string(&resolved) {
                    Ok(code) => code,
                    Err(_) => return false,
                };
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    &specifier.source_name,
                    visited,
                )
            })
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports_inner(
    code: &str,
    source_file: &str,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let source_key = source_key(source_file);
    if !visited.insert(source_key) {
        return false;
    }

    extract_typescript_named_relative_imports(code)
        .into_iter()
        .any(|(import_path, names)| {
            let resolved = match resolve_typescript_import_file(source_file, &import_path) {
                Some(path) => path,
                None => return false,
            };
            let imported_code = match std::fs::read_to_string(&resolved) {
                Ok(code) => code,
                Err(_) => return false,
            };
            let mut export_visited = std::collections::HashSet::new();
            names.iter().any(|name| {
                target_exports_name_only_as_type(
                    &imported_code,
                    resolved.to_str().unwrap_or_default(),
                    name,
                    &mut export_visited,
                )
            }) || has_typescript_type_only_relative_imports_inner(
                &imported_code,
                resolved.to_str().unwrap_or_default(),
                visited,
            )
        })
}

#[cfg(test)]
fn has_typescript_type_only_relative_imports(code: &str, source_file: Option<&str>) -> bool {
    let source_file = match source_file {
        Some(source_file) => source_file,
        None => return false,
    };

    has_typescript_type_only_relative_imports_inner(
        code,
        source_file,
        &mut std::collections::HashSet::new(),
    )
}

/// Remove only imports proven to contain type-only exports. Node 24's native
/// type stripping handles syntax, but it must not evaluate a type-only binding
/// from a runtime module. Unknown bindings are left untouched.
fn strip_typescript_type_only_imports(code: &str, source_file: Option<&str>) -> String {
    let Some(source_file) = source_file else {
        return code.to_string();
    };
    let mut out = String::with_capacity(code.len());
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import type ") {
            continue;
        }
        if trimmed.starts_with("import {") && trimmed.contains("} from") {
            let Some((left, right)) = line.split_once("} from") else {
                out.push_str(line);
                out.push('\n');
                continue;
            };
            let names = left.split_once('{').map(|(_, names)| names).unwrap_or("");
            let import_path = parse_quoted_path(right).unwrap_or_default();
            if !(import_path.starts_with("./") || import_path.starts_with("../")) {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let resolved = resolve_typescript_import_file(source_file, &import_path);
            let kept = names
                .split(',')
                .filter_map(|entry| {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        return None;
                    }
                    let source_name = entry
                        .split_once(" as ")
                        .map(|(name, _)| name.trim())
                        .unwrap_or(entry);
                    let is_type = resolved
                        .as_ref()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .map(|text| {
                            let mut visited = std::collections::HashSet::new();
                            target_exports_name_only_as_type(
                                &text,
                                resolved.as_ref().and_then(|p| p.to_str()).unwrap_or(""),
                                source_name,
                                &mut visited,
                            )
                        })
                        .unwrap_or(false);
                    (!is_type).then(|| entry.to_string())
                })
                .collect::<Vec<_>>();
            if kept.is_empty() {
                continue;
            }
            let prefix = &line[..line.find('{').unwrap_or(0)];
            out.push_str(prefix);
            out.push_str("{ ");
            out.push_str(&kept.join(", "));
            out.push_str(" } from");
            out.push_str(right);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

fn should_retry_typescript_with_repo_runtime(result: &ExecutionResult) -> bool {
    !result.timed_out
        && !result.memory_error
        && result.exit_code != Some(0)
        && (result.stderr.contains("Bun is not defined")
            || result.stderr.contains("ERR_MODULE_NOT_FOUND")
            || result.stderr.contains("ERR_IMPORT_ATTRIBUTE_MISSING")
            || result.stderr.contains("needs an import attribute"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TypeScriptRuntimeMode {
    Auto,
    ForceNode,
    ForceBun,
    ForceRepoNative,
}

fn source_matches_disk(code: &str, source_file: Option<&str>) -> Option<std::path::PathBuf> {
    let source_file = source_file?;
    let disk_code = std::fs::read_to_string(source_file).ok()?;
    if disk_code != code {
        return None;
    }
    Some(
        std::fs::canonicalize(source_file)
            .unwrap_or_else(|_| std::path::PathBuf::from(source_file)),
    )
}

#[allow(clippy::too_many_arguments)]
async fn run_sandbox_process(
    interpreter: &str,
    extra_args: &[String],
    file_path: &std::path::Path,
    python_module_run: Option<(&std::path::Path, &str)>,
    source_file: Option<&str>,
    project_dir: Option<&str>,
    path_env: &str,
    extra_envs: &[(String, String)],
    timeout_seconds: f64,
    memory_mb: u64,
    language: &Language,
) -> ExecutionResult {
    let mut cmd = Command::new(interpreter);
    for arg in extra_args {
        cmd.arg(arg);
    }

    if let Some((pkg_root_parent, module_path)) = python_module_run {
        cmd.arg("-m");
        cmd.arg(module_path);
        cmd.current_dir(pkg_root_parent);
    } else {
        cmd.arg(file_path);
    }

    if python_module_run.is_none() {
        if let Some(src) = source_file {
            if let Some(parent) = std::path::Path::new(src).parent() {
                cmd.current_dir(parent);
            }
        } else if let Some(dir) = project_dir {
            cmd.current_dir(dir);
        }
    }

    cmd.env_clear();
    cmd.env("PATH", path_env);
    let home_dir = project_dir
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
    cmd.env("HOME", &home_dir);
    for (k, v) in extra_envs {
        cmd.env(k, v);
    }

    let mem_bytes = memory_mb * 1024 * 1024;
    let cpu_secs = timeout_seconds.ceil() as u64;
    let is_typescript = matches!(language, Language::TypeScript);
    unsafe {
        cmd.pre_exec(move || {
            use nix::sys::resource::{setrlimit, Resource};

            libc::setsid();

            if !is_typescript {
                let _ = setrlimit(Resource::RLIMIT_AS, mem_bytes, mem_bytes);
                let _ = setrlimit(Resource::RLIMIT_DATA, mem_bytes, mem_bytes);
                setrlimit(Resource::RLIMIT_CPU, cpu_secs, cpu_secs)
                    .map_err(std::io::Error::from)?;
            }
            let ten_mb = 10 * 1024 * 1024;
            let _ = setrlimit(Resource::RLIMIT_FSIZE, ten_mb, ten_mb);

            Ok(())
        });
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());
    cmd.kill_on_drop(true);

    let start = Instant::now();

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExecutionResult {
                stdout: String::new(),
                stderr: format!("Failed to spawn process: {e}"),
                exit_code: None,
                duration_ms: start.elapsed().as_millis() as u64,
                timed_out: false,
                memory_error: false,
            };
        }
    };

    let child_pid = child.id().unwrap_or(0);
    let mem_limit_bytes = mem_bytes;

    let memory_killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let memory_killed_clone = memory_killed.clone();

    let monitor_handle = if child_pid > 0 {
        Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let rss = get_process_group_rss_bytes(child_pid);
                if rss > mem_limit_bytes {
                    memory_killed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                    unsafe {
                        libc::kill(-(child_pid as i32), libc::SIGKILL);
                        libc::kill(child_pid as i32, libc::SIGKILL);
                    }
                    break;
                }
            }
        }))
    } else {
        None
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs_f64(timeout_seconds),
        child.wait_with_output(),
    )
    .await;

    if let Some(handle) = monitor_handle {
        handle.abort();
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let was_memory_killed = memory_killed.load(std::sync::atomic::Ordering::SeqCst);

    match result {
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let memory_error = was_memory_killed
                || stderr.contains("MemoryError")
                || stderr.contains("Cannot allocate memory")
                || stderr.contains("ENOMEM")
                || stderr.contains("JavaScript heap out of memory");

            ExecutionResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: if was_memory_killed && stderr.is_empty() {
                    format!("Killed: memory limit exceeded ({memory_mb} MB)")
                } else {
                    stderr
                },
                exit_code: output.status.code(),
                duration_ms,
                timed_out: false,
                memory_error,
            }
        }
        Ok(Err(e)) => ExecutionResult {
            stdout: String::new(),
            stderr: format!("Failed to spawn process: {e}"),
            exit_code: None,
            duration_ms,
            timed_out: false,
            memory_error: false,
        },
        Err(_) => ExecutionResult {
            stdout: String::new(),
            stderr: if was_memory_killed {
                format!("Killed: memory limit exceeded ({memory_mb} MB)")
            } else {
                "Process timed out".to_string()
            },
            exit_code: None,
            duration_ms,
            timed_out: !was_memory_killed,
            memory_error: was_memory_killed,
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_local_trusted(
    interpreter: &str,
    extra_args: &[String],
    file_path: &std::path::Path,
    python_module_run: Option<(&std::path::Path, &str)>,
    source_file: Option<&str>,
    project_dir: Option<&str>,
    path_env: &str,
    extra_envs: &[(String, String)],
    timeout_seconds: f64,
    memory_mb: u64,
    language: &Language,
) -> ExecutionResult {
    run_sandbox_process(
        interpreter,
        extra_args,
        file_path,
        python_module_run,
        source_file,
        project_dir,
        path_env,
        extra_envs,
        timeout_seconds,
        memory_mb,
        language,
    )
    .await
}

/// Walk up from a directory to find the Python package root.
/// Returns the deepest ancestor that has __init__.py in it,
/// plus the path from there to the starting dir.
fn find_python_package_root(start_dir: &std::path::Path) -> Option<(std::path::PathBuf, String)> {
    // Walk up while __init__.py exists
    let mut dir = start_dir.to_path_buf();
    let mut parts: Vec<String> = vec![];

    loop {
        if !dir.join("__init__.py").exists() {
            break;
        }
        parts.push(dir.file_name()?.to_str()?.to_string());
        dir = dir.parent()?.to_path_buf();
    }

    if parts.is_empty() {
        return None;
    }

    parts.reverse();
    let module_prefix = parts.join(".");
    // dir is now the parent of the package root
    Some((dir, module_prefix))
}

fn module_run_for_python_source(
    source_path: &std::path::Path,
) -> Option<(std::path::PathBuf, String)> {
    let parent = source_path.parent()?;
    let stem = source_path.file_stem()?.to_str()?;
    if !is_valid_python_module_name(stem) {
        return None;
    }

    let (pkg_root_parent, module_prefix) = find_python_package_root(parent)?;
    Some((pkg_root_parent, format!("{module_prefix}.{stem}")))
}

pub async fn docker_daemon_ready() -> Result<(), String> {
    let output = docker_output(&["info"]).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub async fn docker_image_id(image: &str) -> Result<String, String> {
    if image.trim().is_empty() || image.starts_with('-') {
        return Err("docker image must be non-empty and must not begin with '-'".into());
    }
    let output = docker_output(&["image", "inspect", image]).await?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v.get(0)?.get("Id")?.as_str().map(str::to_owned))
        .ok_or_else(|| "docker image inspect returned no image id".into())
}

async fn docker_output(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("docker")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("docker unavailable: {e}"))
}

async fn run_docker_isolated(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    let image = options.docker_image.unwrap_or_default();
    let started = Instant::now();
    let _inspected = match docker_output(&["image", "inspect", image]).await {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return err_result(&format!(
                "docker image inspect failed for {image}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
        Err(error) => return err_result(&format!("docker setup failed: {error}")),
    };
    let harness = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => return err_result(&format!("docker setup failed creating harness: {e}")),
    };
    let ext = if matches!(language, Language::Python) {
        "py"
    } else {
        "ts"
    };
    let harness_file = harness.path().join(format!("snippet.{ext}"));
    if let Err(e) = std::fs::write(&harness_file, code) {
        return err_result(&format!("docker setup failed writing harness: {e}"));
    }
    let synthetic_project = if options.project_dir.is_none() && options.source_file.is_none() {
        Some(match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(e) => return err_result(&format!("docker setup failed creating project: {e}")),
        })
    } else {
        None
    };
    let project = options
        .project_dir
        .map(std::path::PathBuf::from)
        .or_else(|| {
            options.source_file.and_then(|f| {
                std::path::Path::new(f)
                    .parent()
                    .map(std::path::Path::to_path_buf)
            })
        })
        .or_else(|| synthetic_project.as_ref().map(|d| d.path().to_path_buf()));
    let project = match project {
        Some(path) if path.exists() => std::fs::canonicalize(&path).unwrap_or(path),
        _ => return err_result("docker setup failed: project directory does not exist"),
    };
    let relative_source = options.source_file.and_then(|source| {
        let source = std::fs::canonicalize(source).ok()?;
        source
            .strip_prefix(&project)
            .ok()?
            .to_str()
            .map(str::to_owned)
    });
    let container = format!(
        "court-jester-{}-{}",
        std::process::id(),
        started.elapsed().as_nanos()
    );
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let translated_cwd = "/workspace";
    let mut create: Vec<String> = vec![
        "create".into(),
        "--name".into(),
        container.clone(),
        "--pull=never".into(),
        "--network".into(),
        "none".into(),
        "--cpus".into(),
        "1".into(),
        "--memory".into(),
        format!("{}m", options.memory_mb),
        "--memory-swap".into(),
        format!("{}m", options.memory_mb),
        "--pids-limit".into(),
        "128".into(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp:rw,nosuid,nodev,noexec,size=64m".into(),
        "--ulimit".into(),
        "fsize=10485760:10485760".into(),
        "--user".into(),
        format!("{uid}:{gid}"),
        "-e".into(),
        "HOME=/tmp".into(),
        "-e".into(),
        "PYTHONDONTWRITEBYTECODE=1".into(),
        "--mount".into(),
        format!(
            "type=bind,src={},dst=/court-jester/harness,readonly",
            harness.path().display()
        ),
        "--mount".into(),
        format!(
            "type=bind,src={},dst=/workspace,readonly",
            project.display()
        ),
        "--workdir".into(),
        translated_cwd.into(),
        image.to_owned(),
    ];
    if let Some(relative) = relative_source.as_deref() {
        let disk_matches = options
            .source_file
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|disk| disk == code)
            .unwrap_or(false);
        if !disk_matches {
            create.insert(create.len() - 1, "--mount".into());
            create.insert(
                create.len() - 1,
                format!(
                    "type=bind,src={},dst=/workspace/{relative},readonly",
                    harness_file.display()
                ),
            );
        }
    }
    let command = match language {
        Language::Python => vec![
            "python3".to_string(),
            "/court-jester/harness/snippet.py".to_string(),
        ],
        Language::TypeScript => vec![
            "node".into(),
            "--no-warnings".into(),
            "--experimental-transform-types".into(),
            "/court-jester/harness/snippet.ts".into(),
        ],
    };
    create.extend(command);
    let create_args: Vec<&str> = create.iter().map(String::as_str).collect();
    let _created = match docker_output(&create_args).await {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return err_result(&format!(
                "docker create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
        Err(error) => return err_result(&format!("docker create failed: {error}")),
    };
    let cleanup = || async {
        let _ = docker_output(&["rm", "-f", &container]).await;
    };
    let started_ok = match docker_output(&["start", &container]).await {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let _ = cleanup().await;
            return err_result(&format!(
                "docker start failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(error) => {
            let _ = cleanup().await;
            return err_result(&format!("docker start failed: {error}"));
        }
    };
    if !started_ok {
        let _ = cleanup().await;
        return err_result("docker start failed");
    }
    let wait_args = ["wait", container.as_str()];
    let wait_future = docker_output(&wait_args);
    let wait_result = tokio::time::timeout(
        std::time::Duration::from_secs_f64(options.timeout_seconds),
        wait_future,
    )
    .await;
    let timed_out = wait_result.is_err();
    if timed_out {
        let _ = docker_output(&["kill", &container]).await;
    }
    let state_output = docker_output(&["inspect", "--format", "{{json .State}}", &container])
        .await
        .ok();
    let logs = docker_output(&["logs", &container]).await.ok();
    let state = state_output
        .as_ref()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok());
    let oom = state
        .as_ref()
        .and_then(|s| s.get("OOMKilled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let exit_code = state
        .as_ref()
        .and_then(|s| s.get("ExitCode"))
        .and_then(serde_json::Value::as_i64)
        .map(|n| n as i32);
    let stderr = logs
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
        .unwrap_or_default();
    let stdout = logs
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let _ = cleanup().await;
    ExecutionResult {
        stdout,
        stderr: if timed_out {
            "Process timed out".into()
        } else if oom {
            format!("Killed: memory limit exceeded ({} MB)", options.memory_mb)
        } else {
            stderr
        },
        exit_code: if timed_out { None } else { exit_code },
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        memory_error: oom,
    }
}

/// Execute code in a sandboxed subprocess with resource limits.
/// When `source_file` is provided, the code is written as a sibling file so that
/// sibling Python imports and relative imports resolve correctly.
/// When `project_dir` is provided, set cwd to that directory and detect venvs.
/// Execute code using the validated runtime request.
pub async fn execute(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_with_typescript_mode(code, language, options, TypeScriptRuntimeMode::Auto, "run").await
}

pub async fn execute_typescript_node(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_with_typescript_mode(
        code,
        &Language::TypeScript,
        options,
        TypeScriptRuntimeMode::ForceNode,
        "run",
    )
    .await
}

pub async fn execute_typescript_bun(code: &str, options: SandboxOptions<'_>) -> ExecutionResult {
    execute_with_typescript_mode(
        code,
        &Language::TypeScript,
        options,
        TypeScriptRuntimeMode::ForceBun,
        "run",
    )
    .await
}

pub async fn execute_typescript_bun_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> ExecutionResult {
    execute_with_typescript_mode(
        code,
        &Language::TypeScript,
        options,
        TypeScriptRuntimeMode::ForceBun,
        "test",
    )
    .await
}

pub async fn execute_typescript_repo_native(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_with_typescript_mode(
            code,
            &Language::TypeScript,
            options,
            TypeScriptRuntimeMode::ForceRepoNative,
            "run",
        )
        .await,
    )
}

pub async fn execute_typescript_repo_native_test(
    code: &str,
    options: SandboxOptions<'_>,
) -> Option<ExecutionResult> {
    detect_repo_typescript_runner(options.project_dir, options.source_file)?;
    Some(
        execute_with_typescript_mode(
            code,
            &Language::TypeScript,
            options,
            TypeScriptRuntimeMode::ForceRepoNative,
            "test",
        )
        .await,
    )
}

async fn execute_with_typescript_mode(
    code: &str,
    language: &Language,
    options: SandboxOptions<'_>,
    ts_mode: TypeScriptRuntimeMode,
    bun_subcommand: &str,
) -> ExecutionResult {
    if let Err(message) = options.validate() {
        return err_result(&message);
    }
    let timeout_seconds = options.timeout_seconds;
    let memory_mb = options.memory_mb;
    let project_dir = options.project_dir;
    let source_file = options.source_file;
    let transformed_code = matches!(language, Language::TypeScript)
        .then(|| strip_typescript_type_only_imports(code, source_file));
    let code_for_exec = transformed_code.as_deref().unwrap_or(code);
    // Decide where to write the code file:
    // - If source_file is set for Python, write as a sibling so same-directory imports resolve.
    // - If source_file is set for TypeScript and code has module dependencies, write as a sibling.
    // - Otherwise, write to a temp directory (isolated, avoids polluting stdlib dirs etc.)
    let has_relative_imports = match language {
        Language::Python => has_python_relative_imports(code_for_exec),
        Language::TypeScript => has_typescript_relative_imports(code_for_exec),
    };
    let use_sibling = match language {
        Language::Python => {
            source_file.is_some() && options.runtime_profile != RuntimeProfile::Isolated
        }
        Language::TypeScript => {
            source_file.is_some()
                && options.runtime_profile != RuntimeProfile::Isolated
                && has_typescript_module_dependencies(code_for_exec)
        }
    };
    let tmpdir_holder;
    let rand_id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // For Python with relative imports, we need to run as a module (`python -m`)
    // so track the module path and package root for command construction later.
    let mut python_module_run: Option<(std::path::PathBuf, String)> = None;
    let mut direct_source_file = match language {
        Language::TypeScript => source_matches_disk(code_for_exec, source_file),
        Language::Python => source_matches_disk(code_for_exec, source_file),
    };
    if options.runtime_profile == RuntimeProfile::Isolated {
        direct_source_file = None;
    }
    if matches!(language, Language::Python) && has_relative_imports {
        python_module_run = direct_source_file
            .as_deref()
            .and_then(module_run_for_python_source);
        if python_module_run.is_none() {
            direct_source_file = None;
        }
    }

    let (file_path, _cleanup) = if let Some(source_path) = direct_source_file {
        (source_path, None)
    } else if use_sibling {
        let src = source_file.unwrap();
        let src_path = std::path::Path::new(src);
        if let Some(parent) = src_path.parent() {
            let ext = match language {
                Language::Python => "py",
                Language::TypeScript => "ts",
            };

            // For Python with relative imports, use a valid module name (no dots/hyphens)
            // and run with `python -m` from the package root
            let is_python_relative =
                matches!(language, Language::Python) && has_python_relative_imports(code_for_exec);

            let fuzz_filename = if is_python_relative {
                format!("court_jester_fuzz_{rand_id}.{ext}")
            } else {
                format!(".court-jester-fuzz-{rand_id}.{ext}")
            };

            let sibling = parent.join(&fuzz_filename);
            if let Err(e) = std::fs::write(&sibling, code_for_exec) {
                return err_result(&format!("Failed to write sibling file: {e}"));
            }
            let mut cleanup_paths = vec![sibling.clone()];

            // Set up python -m execution if needed
            if is_python_relative {
                if let Some((pkg_root_parent, module_prefix)) = find_python_package_root(parent) {
                    let module_name = format!("court_jester_fuzz_{rand_id}");
                    let full_module = format!("{module_prefix}.{module_name}");
                    python_module_run = Some((pkg_root_parent, full_module));
                }
                // Ensure __init__.py exists in the source dir (needed for package imports)
                let init_py = parent.join("__init__.py");
                if !init_py.exists() {
                    let _ = std::fs::write(&init_py, "");
                    cleanup_paths.push(init_py);
                }
            }

            let sibling = std::fs::canonicalize(&sibling).unwrap_or(sibling);
            let cleanup = CleanupFiles {
                paths: cleanup_paths,
            };
            (sibling, Some(cleanup))
        } else {
            tmpdir_holder = match tempfile::tempdir() {
                Ok(d) => d,
                Err(e) => return err_result(&format!("Failed to create temp dir: {e}")),
            };
            let ext = match language {
                Language::Python => "py",
                Language::TypeScript => "ts",
            };
            let p = tmpdir_holder.path().join(format!("snippet.{ext}"));
            if let Err(e) = std::fs::write(&p, code_for_exec) {
                return err_result(&format!("Failed to write temp file: {e}"));
            }
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            (p, None)
        }
    } else {
        tmpdir_holder = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => return err_result(&format!("Failed to create temp dir: {e}")),
        };
        let ext = match language {
            Language::Python => "py",
            Language::TypeScript => "ts",
        };
        let p = tmpdir_holder.path().join(format!("snippet.{ext}"));
        if let Err(e) = std::fs::write(&p, code_for_exec) {
            return err_result(&format!("Failed to write temp file: {e}"));
        }
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        (p, None)
    };
    if options.runtime_profile == RuntimeProfile::Isolated {
        return run_docker_isolated(code_for_exec, language, options).await;
    }

    // Detect interpreter and build environment
    let home = std::env::var("HOME").unwrap_or_default();
    let base_path = format!(
        "{}/.bun/bin:/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin",
        home
    );

    let (path_env, extra_envs, interpreter, extra_args, _repo_fallback) = match language {
        Language::Python => {
            let mut python = "python3".to_string();
            let mut path = base_path.to_string();
            let mut envs: Vec<(String, String)> = vec![];

            if let Some(dir) = project_dir {
                let venv_python = format!("{dir}/.venv/bin/python3");
                if std::path::Path::new(&venv_python).exists() {
                    python = venv_python;
                    path = format!("{dir}/.venv/bin:{base_path}");
                } else {
                    path = format!("{dir}/.venv/bin:{base_path}");
                }
                envs.push(("PYTHONPATH".into(), dir.to_string()));
            }

            (path, envs, python, vec![], None)
        }
        Language::TypeScript => {
            let inherited_path = std::env::var("PATH").unwrap_or_default();
            let mut path = if inherited_path.is_empty() {
                base_path.to_string()
            } else {
                format!("{inherited_path}:{base_path}")
            };
            let mut envs: Vec<(String, String)> = vec![];
            if let Some(dir) = project_dir {
                path = format!("{dir}/node_modules/.bin:{path}");
                envs.push(("NODE_PATH".into(), format!("{dir}/node_modules")));
            }
            let node_path = which_binary(&path, "node");
            let bun_path = which_binary(&path, "bun");
            let repo_runner = detect_repo_typescript_runner(project_dir, source_file);
            let bun_repo = repo_runner.as_deref() == Some("bun");
            let repo_fallback = if bun_repo {
                bun_path
                    .clone()
                    .map(|p| (p, vec![bun_subcommand.to_string()]))
            } else {
                None
            };
            let prefer_repo_native = matches!(ts_mode, TypeScriptRuntimeMode::ForceRepoNative)
                || (matches!(ts_mode, TypeScriptRuntimeMode::Auto)
                    && bun_repo
                    && typescript_code_requires_bun_runtime(code));
            if matches!(ts_mode, TypeScriptRuntimeMode::ForceBun) {
                if let Some(bun_path) = bun_path.clone() {
                    (
                        path,
                        envs,
                        bun_path,
                        vec![bun_subcommand.to_string()],
                        repo_fallback,
                    )
                } else {
                    return err_result("bun runtime requested for TypeScript execution, but `bun` was not found on PATH");
                }
            } else if prefer_repo_native {
                if let Some((bun_path, bun_args)) = repo_fallback.clone() {
                    (path, envs, bun_path, bun_args, repo_fallback)
                } else if let Some(node_path) = node_path {
                    (
                        path,
                        envs,
                        node_path,
                        vec![
                            "--no-warnings".into(),
                            "--experimental-transform-types".into(),
                        ],
                        None,
                    )
                } else {
                    return err_result("Node 24 runtime is required for TypeScript execution; no node or bun executable was found");
                }
            } else if let Some(node_path) = node_path {
                (
                    path,
                    envs,
                    node_path,
                    vec![
                        "--no-warnings".into(),
                        "--experimental-transform-types".into(),
                    ],
                    if matches!(ts_mode, TypeScriptRuntimeMode::ForceNode) {
                        None
                    } else {
                        repo_fallback
                    },
                )
            } else if let Some(bun_path) = bun_path {
                (path, envs, bun_path, vec![bun_subcommand.to_string()], None)
            } else {
                return err_result("Node 24 runtime is required for TypeScript execution; no node or bun executable was found");
            }
        }
    };

    let python_module_run = python_module_run
        .as_ref()
        .map(|(pkg_root_parent, module_path)| (pkg_root_parent.as_path(), module_path.as_str()));
    let result = run_local_trusted(
        &interpreter,
        &extra_args,
        &file_path,
        python_module_run,
        source_file,
        project_dir,
        &path_env,
        &extra_envs,
        timeout_seconds,
        memory_mb,
        language,
    )
    .await;
    if let Some((repo_interpreter, repo_args)) = _repo_fallback {
        if should_retry_typescript_with_repo_runtime(&result) {
            return run_local_trusted(
                &repo_interpreter,
                &repo_args,
                &file_path,
                python_module_run,
                source_file,
                project_dir,
                &path_env,
                &extra_envs,
                timeout_seconds,
                memory_mb,
                language,
            )
            .await;
        }
    }

    result
}

fn err_result(msg: &str) -> ExecutionResult {
    ExecutionResult {
        stdout: String::new(),
        stderr: msg.to_string(),
        exit_code: None,
        duration_ms: 0,
        timed_out: false,
        memory_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{has_typescript_type_only_relative_imports, which_binary};

    #[test]
    fn which_binary_finds_existing_binary_on_path() {
        let path_env = "/missing:/bin:/usr/bin";
        assert_eq!(which_binary(path_env, "sh"), Some("/bin/sh".to_string()));
    }

    #[test]
    fn which_binary_returns_none_for_missing_binary() {
        let path_env = "/missing:/also-missing";
        assert_eq!(which_binary(path_env, "definitely-not-a-real-binary"), None);
    }

    #[test]
    fn detects_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(
            &helper_path,
            "export type PathValue = string | number;\nexport const value = 7;\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue, value } from \"./internals.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn ignores_plain_value_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("helper.ts");
        std::fs::write(&helper_path, "export const value = 7;\n").unwrap();
        std::fs::write(
            &source_path,
            "import { value } from \"./helper.ts\";\nconsole.log(value);\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(!has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_multiline_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &source_path,
            "import {\n  PathValue,\n} from \"./internals.ts\";\nconsole.log('ok');\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_transitive_type_only_relative_imports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("test.ts");
        let object_path = dir.path().join("object.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &object_path,
            "import { PathValue } from \"./internals.ts\";\nexport function pick(path: PathValue): string { return String(path); }\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { pick } from \"./object.ts\";\nconsole.log(pick(\"x\"));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_type_only_relative_reexports() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export type { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }

    #[test]
    fn detects_value_reexports_of_type_only_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("main.ts");
        let index_path = dir.path().join("index.ts");
        let helper_path = dir.path().join("internals.ts");
        std::fs::write(&helper_path, "export type PathValue = string | number;\n").unwrap();
        std::fs::write(
            &index_path,
            "export { PathValue } from \"./internals.ts\";\n",
        )
        .unwrap();
        std::fs::write(
            &source_path,
            "import { PathValue } from \"./index.ts\";\nconsole.log(String(\"x\" as PathValue));\n",
        )
        .unwrap();

        let code = std::fs::read_to_string(&source_path).unwrap();
        assert!(has_typescript_type_only_relative_imports(
            &code,
            Some(source_path.to_str().unwrap())
        ));
    }
}
