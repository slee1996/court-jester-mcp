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

fn tsx_loader_path(tsx_binary: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(tsx_binary).ok()?;
    let loader = canonical.parent()?.join("loader.mjs");
    loader
        .exists()
        .then(|| loader.to_string_lossy().to_string())
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

fn has_typescript_relative_imports(code: &str) -> bool {
    code.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("from \"./")
            || trimmed.contains("from './")
            || trimmed.contains("from \"../")
            || trimmed.contains("from '../")
    })
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

/// Execute code in a sandboxed subprocess with resource limits.
/// When `source_file` is provided, the code is written as a sibling file so that
/// sibling Python imports and relative imports resolve correctly.
/// When `project_dir` is provided, set cwd to that directory and detect venvs.
pub async fn execute(
    code: &str,
    language: &Language,
    timeout_seconds: f64,
    memory_mb: u64,
    project_dir: Option<&str>,
    source_file: Option<&str>,
) -> ExecutionResult {
    // Decide where to write the code file:
    // - If source_file is set for Python, write as a sibling so same-directory imports resolve.
    // - If source_file is set for TypeScript and code has relative imports, write as a sibling.
    // - Otherwise, write to a temp directory (isolated, avoids polluting stdlib dirs etc.)
    let has_relative_imports = match language {
        Language::Python => has_python_relative_imports(code),
        Language::TypeScript => has_typescript_relative_imports(code),
    };
    let use_sibling = match language {
        Language::Python => source_file.is_some(),
        Language::TypeScript => source_file.is_some() && has_relative_imports,
    };
    let tmpdir_holder;
    let rand_id: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // For Python with relative imports, we need to run as a module (`python -m`)
    // so track the module path and package root for command construction later.
    let mut python_module_run: Option<(std::path::PathBuf, String)> = None;

    let (file_path, _cleanup) = if use_sibling {
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
                matches!(language, Language::Python) && has_python_relative_imports(code);

            let fuzz_filename = if is_python_relative {
                format!("court_jester_fuzz_{rand_id}.{ext}")
            } else {
                format!(".court-jester-fuzz-{rand_id}.{ext}")
            };

            let sibling = parent.join(&fuzz_filename);
            if let Err(e) = std::fs::write(&sibling, code) {
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
            if let Err(e) = std::fs::write(&p, code) {
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
        if let Err(e) = std::fs::write(&p, code) {
            return err_result(&format!("Failed to write temp file: {e}"));
        }
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        (p, None)
    };

    // Detect interpreter and build environment
    let home = std::env::var("HOME").unwrap_or_default();
    let base_path = format!(
        "{}/.bun/bin:/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin",
        home
    );

    let (interpreter, extra_args, path_env, extra_envs) = match language {
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

            (python, vec![], path, envs)
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
            let tsx_path = which_binary(&path, "tsx");
            let tsx_loader_path = tsx_path.as_deref().and_then(tsx_loader_path);
            let prefer_node_loader = has_typescript_relative_imports(code);

            if prefer_node_loader {
                if let (Some(node_path), Some(tsx_loader_path)) =
                    (node_path.clone(), tsx_loader_path)
                {
                    // File-backed TypeScript needs full TS module semantics
                    // across imports. Use Node with tsx's loader directly so we
                    // stay on Node without paying for the tsx CLI's IPC server.
                    (
                        node_path,
                        vec!["--import".to_string(), tsx_loader_path],
                        path,
                        envs,
                    )
                } else if let Some(node_path) = node_path {
                    (
                        node_path,
                        vec![
                            "--no-warnings".to_string(),
                            "--experimental-transform-types".to_string(),
                        ],
                        path,
                        envs,
                    )
                } else if let Some(bun_path) = bun_path {
                    (bun_path, vec!["run".to_string()], path, envs)
                } else if let Some(tsx_path) = tsx_path {
                    // Last resort: tsx CLI can bootstrap TypeScript execution,
                    // but it opens an IPC server that stricter sandboxes may
                    // reject.
                    (tsx_path, vec![], path, envs)
                } else {
                    ("npx".to_string(), vec!["tsx".to_string()], path, envs)
                }
            } else if let Some(node_path) = node_path {
                // Prefer Node's built-in TypeScript transform path for
                // standalone snippets. It avoids the IPC server startup that
                // `tsx` uses, which fails under stricter sandboxing and
                // creates false positives in execute/verify.
                (
                    node_path,
                    vec![
                        "--no-warnings".to_string(),
                        "--experimental-transform-types".to_string(),
                    ],
                    path,
                    envs,
                )
            } else if let Some(bun_path) = bun_path {
                (bun_path, vec!["run".to_string()], path, envs)
            } else if let Some(tsx_path) = tsx_path {
                (tsx_path, vec![], path, envs)
            } else {
                ("npx".to_string(), vec!["tsx".to_string()], path, envs)
            }
        }
    };

    let mut cmd = Command::new(&interpreter);
    for arg in &extra_args {
        cmd.arg(arg);
    }

    // For Python module execution, use `-m module.path` instead of file path
    if let Some((ref pkg_root_parent, ref module_path)) = python_module_run {
        cmd.arg("-m");
        cmd.arg(module_path);
        cmd.current_dir(pkg_root_parent);
    } else {
        cmd.arg(&file_path);
    }

    // Set working directory (if not already set by python_module_run)
    if python_module_run.is_none() {
        if let Some(src) = source_file {
            if let Some(parent) = std::path::Path::new(src).parent() {
                cmd.current_dir(parent);
            }
        } else if let Some(dir) = project_dir {
            cmd.current_dir(dir);
        }
    }

    // Minimal environment — no API keys, etc.
    cmd.env_clear();
    cmd.env("PATH", &path_env);
    // HOME is needed by npx (cache lookup for tsx) and some packages.
    let home_dir = project_dir
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
    cmd.env("HOME", &home_dir);
    for (k, v) in &extra_envs {
        cmd.env(k, v);
    }

    // Resource limits via pre_exec (effective on Linux; best-effort on macOS)
    let mem_bytes = memory_mb * 1024 * 1024;
    let cpu_secs = timeout_seconds.ceil() as u64;
    let is_typescript = matches!(language, Language::TypeScript);
    unsafe {
        cmd.pre_exec(move || {
            use nix::sys::resource::{setrlimit, Resource};

            // Create new process group so we can kill all children
            libc::setsid();

            // Skip rlimits for TypeScript — V8 reserves GBs of virtual address
            // space (breaks RLIMIT_AS), and npx tsx burns several seconds of CPU
            // just on startup (breaks RLIMIT_CPU). Rely on tokio timeout + RSS
            // monitor instead.
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

    // Spawn the child so we can poll process-group RSS for memory enforcement.
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

    // RSS monitor: poll the whole process group so wrapper commands and spawned
    // children count against the same memory budget.
    let memory_killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let memory_killed_clone = memory_killed.clone();

    let monitor_handle = if child_pid > 0 {
        Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let rss = get_process_group_rss_bytes(child_pid);
                if rss > mem_limit_bytes {
                    memory_killed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                    // Kill entire process group (negative PID) to catch child processes
                    unsafe {
                        libc::kill(-(child_pid as i32), libc::SIGKILL);
                        // Also kill parent directly in case setsid didn't take effect
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

    // Stop the monitor
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
    use super::which_binary;

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
}
