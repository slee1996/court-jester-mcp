//! Subprocess lifecycle, process-group resource monitoring, and termination evidence.

use crate::types::{
    DiagnosticComponent, DiagnosticImpact, ExecutionLimits, ExecutionResult, FailureDiagnostic,
    FailureDomain, FailureKind, LaunchPlan, NetworkPolicy, ProcessTermination,
    ProcessTerminationKind, RuntimeProfile,
};
use std::time::Instant;
use tokio::process::Command;

/// Own the process group and auxiliary tasks for the entire execution future.
/// Dropping a JoinHandle detaches it; dropping this owner must not do so.
struct ManagedProcessOwner {
    group_id: u32,
    tasks: Vec<tokio::task::AbortHandle>,
}

impl Drop for ManagedProcessOwner {
    fn drop(&mut self) {
        if self.group_id > 0 {
            unsafe {
                libc::kill(-(self.group_id as i32), libc::SIGKILL);
            }
        }
        for task in &self.tasks {
            task.abort();
        }
    }
}

pub(super) fn termination(
    kind: ProcessTerminationKind,
    exit_code: Option<i32>,
    signal: Option<i32>,
) -> ProcessTermination {
    ProcessTermination {
        kind,
        exit_code,
        signal,
        signal_name: signal.map(signal_name),
    }
}

fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGTERM => "SIGTERM",
        libc::SIGKILL => "SIGKILL",
        libc::SIGXCPU => "SIGXCPU",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        other => return format!("SIG{other}"),
    }
    .to_string()
}

fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

#[cfg(test)]
#[tokio::test]
async fn descendant_pipe_timeout_preserves_captured_output() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("orphan");
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "printf observed; printf diagnostic >&2; (sleep 1.5; echo leaked > '{}') & exit 0",
            marker.display()
        ))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let result = run_command_with_limits(
        command,
        0.5,
        128,
        RuntimeProfile::LocalTrusted,
        NetworkPolicy::Deny,
        true,
        "fixture",
    )
    .await;
    assert!(result.timed_out, "{result:?}");
    assert_eq!(result.stdout, "observed");
    assert_eq!(result.stderr, "diagnostic");
    std::thread::sleep(std::time::Duration::from_millis(1600));
    assert!(!marker.exists());
}

#[cfg(test)]
#[tokio::test]
async fn cancelling_managed_execution_terminates_its_process_group() {
    for parent_exits in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("ready");
        let orphan = root.path().join("orphan");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!(
                "(echo ready > '{}'; sleep 0.8; echo orphan > '{}') & {}",
                ready.display(),
                orphan.display(),
                if parent_exits { "exit 0" } else { "wait" }
            ))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let task = tokio::spawn(run_command_with_limits(
            command,
            10.0,
            128,
            RuntimeProfile::LocalTrusted,
            NetworkPolicy::Deny,
            true,
            "fixture",
        ));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !ready.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture did not start");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        assert!(
            !orphan.exists(),
            "cancelled execution left a descendant; parent_exits={parent_exits}"
        );
    }
}

#[cfg(test)]
#[tokio::test]
async fn completed_execution_does_not_detach_background_children() {
    let root = tempfile::tempdir().unwrap();
    let orphan = root.path().join("orphan");
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "(sleep 0.8; echo orphan > '{}') >/dev/null 2>&1 & echo completed; exit 0",
            orphan.display()
        ))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let result = run_command_with_limits(
        command,
        5.0,
        128,
        RuntimeProfile::LocalTrusted,
        NetworkPolicy::Deny,
        true,
        "fixture",
    )
    .await;
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    assert_eq!(result.stdout.trim(), "completed");
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert!(!orphan.exists());
}

#[cfg(test)]
#[tokio::test]
async fn process_owner_aborts_only_its_registered_tasks() {
    let owned = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    });
    let unrelated = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        42
    });
    let owner = ManagedProcessOwner {
        group_id: 0,
        tasks: vec![owned.abort_handle()],
    };
    drop(owner);
    assert!(owned.await.unwrap_err().is_cancelled());
    assert_eq!(unrelated.await.unwrap(), 42);
}

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

pub(super) fn launch_failure(message: impl Into<String>) -> ExecutionResult {
    let message = message.into();
    let termination = termination(ProcessTerminationKind::LaunchFailed, None, None);
    ExecutionResult {
        stdout: String::new(),
        stderr: message.clone(),
        exit_code: None,
        duration_ms: 0,
        timed_out: false,
        memory_error: false,
        termination: Some(termination.clone()),
        diagnostics: vec![FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::LauncherFailure,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message,
            process: Some(termination),
            limits: None,
        }],
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_command_with_limits(
    mut command: Command,
    timeout_seconds: f64,
    memory_mb: u64,
    runtime_profile: RuntimeProfile,
    network_policy: NetworkPolicy,
    is_typescript: bool,
    launch_error_prefix: &str,
) -> ExecutionResult {
    if !timeout_seconds.is_finite() || timeout_seconds <= 0.0 {
        return launch_failure("timeout must be finite and greater than zero");
    }
    let Some(memory_bytes) = memory_mb
        .checked_mul(1024)
        .and_then(|value| value.checked_mul(1024))
    else {
        return launch_failure("memory limit is too large");
    };

    unsafe {
        command.pre_exec(move || {
            use nix::sys::resource::{setrlimit, Resource};
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if !is_typescript {
                let _ = setrlimit(Resource::RLIMIT_AS, memory_bytes, memory_bytes);
                let _ = setrlimit(Resource::RLIMIT_DATA, memory_bytes, memory_bytes);
            }
            let ten_mb = 10 * 1024 * 1024;
            let _ = setrlimit(Resource::RLIMIT_FSIZE, ten_mb, ten_mb);
            Ok(())
        });
    }

    let started = Instant::now();
    command.kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return launch_failure(format!("{launch_error_prefix}: {error}"));
        }
    };
    let pid = child.id().unwrap_or_default();
    let mut owner = ManagedProcessOwner {
        group_id: pid,
        tasks: Vec::new(),
    };
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_stdout = stdout_bytes.clone();
    let stdout_task = tokio::spawn(async move {
        if let Some(mut pipe) = stdout_pipe {
            let mut chunk = [0; 8192];
            while let Ok(size) = tokio::io::AsyncReadExt::read(&mut pipe, &mut chunk).await {
                if size == 0 {
                    break;
                }
                captured_stdout
                    .lock()
                    .unwrap()
                    .extend_from_slice(&chunk[..size]);
            }
        }
    });
    let stderr_bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    owner.tasks.push(stdout_task.abort_handle());
    let captured_stderr = stderr_bytes.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(mut pipe) = stderr_pipe {
            let mut chunk = [0; 8192];
            while let Ok(size) = tokio::io::AsyncReadExt::read(&mut pipe, &mut chunk).await {
                if size == 0 {
                    break;
                }
                captured_stderr
                    .lock()
                    .unwrap()
                    .extend_from_slice(&chunk[..size]);
            }
        }
    });
    let memory_killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    owner.tasks.push(stderr_task.abort_handle());
    let memory_killed_clone = memory_killed.clone();
    let monitor = (pid > 0).then(|| {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if get_process_group_rss_bytes(pid) > memory_bytes {
                    memory_killed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    break;
                }
            }
        })
    });

    let mut wait = Box::pin(child.wait());
    if let Some(task) = &monitor {
        owner.tasks.push(task.abort_handle());
    }
    let mut timeout = Box::pin(tokio::time::sleep(std::time::Duration::from_secs_f64(
        timeout_seconds,
    )));
    let mut timed_out = false;
    let wait_result = tokio::select! {
        result = &mut wait => result,
        _ = &mut timeout => {
            timed_out = true;
            if pid > 0 {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            wait.await
        }
    };
    // A reaped parent does not close pipes inherited by its descendants. Keep
    // pipe collection inside the same deadline instead of waiting indefinitely.
    let stdout_abort = stdout_task.abort_handle();
    let stderr_abort = stderr_task.abort_handle();
    let mut collected = tokio::spawn(async move {
        (
            stdout_task.await.unwrap_or_default(),
            stderr_task.await.unwrap_or_default(),
        )
    });
    let remaining = (timeout_seconds - started.elapsed().as_secs_f64()).max(0.0);
    owner.tasks.push(collected.abort_handle());
    match tokio::time::timeout(
        std::time::Duration::from_secs_f64(remaining),
        &mut collected,
    )
    .await
    {
        Ok(result) => result.unwrap_or_default(),
        Err(_) => {
            timed_out = true;
            if pid > 0 {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            stdout_abort.abort();
            stderr_abort.abort();
            collected.await.unwrap_or_default()
        }
    };
    if let Some(handle) = monitor {
        handle.abort();
    }
    let stdout = String::from_utf8_lossy(&stdout_bytes.lock().unwrap()).to_string();
    let mut stderr = String::from_utf8_lossy(&stderr_bytes.lock().unwrap()).to_string();
    let duration_ms = started.elapsed().as_millis() as u64;
    let (status, wait_error) = match wait_result {
        Ok(status) => (Some(status), None),
        Err(error) => (None, Some(error)),
    };
    if let Some(error) = wait_error.as_ref() {
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&format!("failed to wait for process: {error}"));
    }
    let signal = status.as_ref().and_then(status_signal);
    let memory_limited = memory_killed.load(std::sync::atomic::Ordering::SeqCst);
    let kind = if memory_limited {
        ProcessTerminationKind::MemoryLimit
    } else if timed_out {
        ProcessTerminationKind::TimedOut
    } else if wait_error.is_some() {
        ProcessTerminationKind::WaitFailed
    } else if signal.is_some() {
        ProcessTerminationKind::Signaled
    } else {
        ProcessTerminationKind::Exited
    };
    let termination = termination(
        kind,
        status.as_ref().and_then(std::process::ExitStatus::code),
        signal,
    );
    if memory_limited && stderr.is_empty() {
        stderr = format!("Killed: memory limit exceeded ({memory_mb} MB)");
    } else if matches!(kind, ProcessTerminationKind::TimedOut) && stderr.is_empty() {
        stderr = "Process timed out".into();
    } else if matches!(kind, ProcessTerminationKind::Signaled) && stderr.is_empty() {
        stderr = format!(
            "Process terminated by {}",
            termination.signal_name.as_deref().unwrap_or("signal")
        );
    }
    let limits = ExecutionLimits {
        timeout_seconds,
        memory_mb,
        runtime_profile,
        network_policy,
    };
    let mut diagnostics = match kind {
        ProcessTerminationKind::MemoryLimit => vec![FailureDiagnostic {
            domain: FailureDomain::Resource,
            kind: FailureKind::MemoryLimit,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: stderr.clone(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        }],
        ProcessTerminationKind::TimedOut => vec![FailureDiagnostic {
            domain: FailureDomain::Resource,
            kind: FailureKind::Timeout,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: stderr.clone(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        }],
        ProcessTerminationKind::LaunchFailed | ProcessTerminationKind::WaitFailed => {
            vec![FailureDiagnostic {
                domain: FailureDomain::Environment,
                kind: if matches!(kind, ProcessTerminationKind::LaunchFailed) {
                    FailureKind::LauncherFailure
                } else {
                    FailureKind::ToolFailure
                },
                component: DiagnosticComponent::Sandbox,
                impact: DiagnosticImpact::Blocking,
                message: stderr.clone(),
                process: Some(termination.clone()),
                limits: Some(limits.clone()),
            }]
        }
        ProcessTerminationKind::Signaled => vec![FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::Signal,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: stderr.clone(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        }],
        _ => Vec::new(),
    };
    if network_policy == NetworkPolicy::Deny
        && stderr.contains("court-jester network access denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::NetworkDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "network access was denied by the sandbox".into(),
            process: Some(termination.clone()),
            limits: Some(limits.clone()),
        });
    }
    if network_policy == NetworkPolicy::Deny && stderr.contains("court-jester process spawn denied")
    {
        diagnostics.push(FailureDiagnostic {
            domain: FailureDomain::Environment,
            kind: FailureKind::ProcessSpawnDenied,
            component: DiagnosticComponent::Sandbox,
            impact: DiagnosticImpact::Blocking,
            message: "process spawning was denied by the sandbox".into(),
            process: Some(termination.clone()),
            limits: Some(limits),
        });
    }
    ExecutionResult {
        stdout,
        stderr,
        exit_code: status.as_ref().and_then(std::process::ExitStatus::code),
        duration_ms,
        timed_out: matches!(kind, ProcessTerminationKind::TimedOut),
        memory_error: matches!(kind, ProcessTerminationKind::MemoryLimit),
        termination: Some(termination),
        diagnostics,
    }
}

pub(super) async fn run_launch_command(
    plan: &LaunchPlan,
    timeout_seconds: f64,
    memory_mb: u64,
    runtime_profile: RuntimeProfile,
    network_policy: NetworkPolicy,
    is_typescript: bool,
) -> ExecutionResult {
    let configured_path = plan
        .env
        .iter()
        .find(|(name, _)| name == std::ffi::OsStr::new("PATH"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
    let mut command = Command::new(&plan.executable);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .env("PATH", configured_path)
        .env("HOME", plan.cwd.to_string_lossy().as_ref())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for (key, value) in &plan.env {
        command.env(key, value);
    }
    run_command_with_limits(
        command,
        timeout_seconds,
        memory_mb,
        runtime_profile,
        network_policy,
        is_typescript,
        "failed to launch harness",
    )
    .await
}
