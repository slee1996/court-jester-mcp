//! Owned container execution, cancellation, and bounded teardown.
use super::*;

static ACTIVE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct ActiveWorker;
impl Drop for ActiveWorker {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Wait for owned workers to finish; failures are reported by each worker.
/// Quiescence is not a claim that an unavailable daemon confirmed removal.
pub async fn wait_for_docker_cleanup(timeout: std::time::Duration) -> bool {
    tokio::time::timeout(timeout, async {
        while ACTIVE_WORKERS.load(std::sync::atomic::Ordering::SeqCst) != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

pub(super) struct DockerLifecycle {
    pub(super) program: std::path::PathBuf,
    pub(super) create: Vec<String>,
    pub(super) container: String,
    pub(super) started: Instant,
    pub(super) limits: ExecutionLimits,
    pub(super) adapter: Option<TestAdapter>,
    pub(super) _workspace: Option<tempfile::TempDir>,
    pub(super) _generated_workspace: Option<std::sync::Arc<tempfile::TempDir>>,
    pub(super) _runtime_guard: Option<std::sync::Arc<NetworkGuard>>,
    pub(super) _resolver: Option<NodePackageResolver>,
}

struct CancelDockerOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelDockerOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) async fn supervise_docker_lifecycle(lifecycle: DockerLifecycle) -> ExecutionResult {
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _owner = CancelDockerOnDrop(cancelled.clone());
    // The worker owns the lease through cleanup even if its caller disappears.
    ACTIVE_WORKERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let active = ActiveWorker;
    tokio::spawn(async move {
        let _active = active;
        lifecycle.run(cancelled).await
    })
    .await
    .unwrap_or_else(|error| launch_failure(format!("Docker lifecycle worker failed: {error}")))
}

impl DockerLifecycle {
    async fn command(&self, args: &[&str], timeout: f64) -> ExecutionResult {
        docker_program_output(&self.program, args, timeout, self.limits.memory_mb).await
    }

    async fn interruptible(
        &self,
        args: &[&str],
        timeout: f64,
        cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ExecutionResult {
        let cancellation = async {
            while !cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            biased;
            _ = cancellation => launch_failure("Docker workflow cancelled"),
            result = self.command(args, timeout) => result,
        }
    }

    async fn run(
        self,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ExecutionResult {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return launch_failure("Docker workflow cancelled before creation");
        }
        let remaining =
            || (self.limits.timeout_seconds - self.started.elapsed().as_secs_f64()).max(0.0);
        let create_args: Vec<&str> = self.create.iter().map(String::as_str).collect();
        // Teardown has a separate bounded allowance so an exhausted execution
        // deadline cannot prevent an attempt to remove this uniquely named container.
        let cleanup_timeout = self.limits.timeout_seconds.min(5.0);
        let cleanup = || async {
            let name = &self.container;
            let mut result = self
                .command(&["rm", "-f", &self.container], cleanup_timeout)
                .await;
            if !docker_command_succeeded(&result) {
                result.stderr = format!("container {name}: {}", result.stderr);
                eprintln!(
                    "Court Jester container cleanup could not be confirmed: {}",
                    result.stderr
                );
            }
            result
        };
        let created = self.command(&create_args, remaining().min(10.0)).await;
        if !docker_command_succeeded(&created) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure("create", created, Some(&cleaned));
        }
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure(
                "cancelled",
                launch_failure("Docker workflow cancelled"),
                Some(&cleaned),
            );
        }
        let launched = self
            .command(&["start", &self.container], remaining().min(10.0))
            .await;
        if !docker_command_succeeded(&launched) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure("start", launched, Some(&cleaned));
        }
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure(
                "cancelled",
                launch_failure("Docker workflow cancelled"),
                Some(&cleaned),
            );
        }
        let wait_result = self
            .interruptible(&["wait", &self.container], remaining(), &cancelled)
            .await;
        if !docker_command_succeeded(&wait_result) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure("wait", wait_result, Some(&cleaned));
        }
        let inspected = self
            .interruptible(
                &["inspect", "--format", "{{json .State}}", &self.container],
                remaining(),
                &cancelled,
            )
            .await;
        if !docker_command_succeeded(&inspected) {
            let cleaned = cleanup().await;
            return docker_lifecycle_failure("inspect", inspected, Some(&cleaned));
        }
        let logs = self
            .interruptible(&["logs", &self.container], remaining(), &cancelled)
            .await;
        let cleaned = cleanup().await;
        if !docker_command_succeeded(&logs) {
            return docker_lifecycle_failure("logs", logs, Some(&cleaned));
        }
        if !docker_command_succeeded(&cleaned) {
            return docker_lifecycle_failure("cleanup", cleaned, None);
        }
        let state = serde_json::from_str::<serde_json::Value>(&inspected.stdout).ok();
        if state
            .as_ref()
            .and_then(|value| value.get("Running"))
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        {
            return launch_failure("Docker inspect did not confirm that the container stopped");
        }
        let memory_limited = state
            .as_ref()
            .and_then(|value| value.get("OOMKilled"))
            .and_then(serde_json::Value::as_bool);
        let exit_code = state
            .as_ref()
            .and_then(|value| value.get("ExitCode"))
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok());
        let (Some(memory_limited), Some(exit_code)) = (memory_limited, exit_code) else {
            return launch_failure(
                "Docker inspect returned incomplete container termination evidence",
            );
        };
        let kind = if memory_limited {
            ProcessTerminationKind::MemoryLimit
        } else {
            ProcessTerminationKind::Exited
        };
        let mut process = ExecutionResult {
            stdout: logs.stdout,
            stderr: if memory_limited {
                format!(
                    "Killed: memory limit exceeded ({} MB)",
                    self.limits.memory_mb
                )
            } else {
                logs.stderr
            },
            exit_code: Some(exit_code),
            duration_ms: self.started.elapsed().as_millis() as u64,
            timed_out: false,
            memory_error: memory_limited,
            termination: Some(termination(kind, Some(exit_code), None)),
            diagnostics: Vec::new(),
        };
        let execution_limits = crate::types::ExecutionLimits {
            timeout_seconds: self.limits.timeout_seconds,
            memory_mb: self.limits.memory_mb,
            runtime_profile: self.limits.runtime_profile,
            network_policy: NetworkPolicy::Deny,
        };
        process.diagnostics = harness_diagnostics(self.adapter, &process, &execution_limits);
        process
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "requires Docker and the installed Python image"]
    async fn cancelled_real_container_worker_removes_only_its_container() {
        let program = std::path::PathBuf::from("docker");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let names = [
            format!("court-jester-cancel-{nonce}-a"),
            format!("court-jester-cancel-{nonce}-b"),
        ];
        let mut tasks = Vec::new();
        for name in &names {
            let lifecycle = super::DockerLifecycle {
                program: program.clone(),
                create: vec![
                    "create".into(),
                    "--name".into(),
                    name.clone(),
                    "--pull=never".into(),
                    "--network=none".into(),
                    "--read-only".into(),
                    "--memory=128m".into(),
                    "python:3.12-slim".into(),
                    "python3".into(),
                    "-c".into(),
                    "import time; time.sleep(20)".into(),
                ],
                container: name.clone(),
                started: std::time::Instant::now(),
                limits: crate::types::ExecutionLimits {
                    timeout_seconds: 10.0,
                    memory_mb: 128,
                    runtime_profile: crate::types::RuntimeProfile::Isolated,
                    network_policy: crate::types::NetworkPolicy::Deny,
                },
                adapter: None,
                _workspace: None,
                _generated_workspace: None,
                _runtime_guard: None,
                _resolver: None,
            };
            tasks.push(tokio::spawn(super::supervise_docker_lifecycle(lifecycle)));
        }
        let ready = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let mut both = true;
                for name in &names {
                    let state = super::docker_output(
                        &["inspect", "--format", "{{.State.Running}}", name],
                        1.0,
                        128,
                    )
                    .await;
                    both &=
                        super::docker_command_succeeded(&state) && state.stdout.trim() == "true";
                }
                if both {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        tasks[0].abort();
        let first_removed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let listed = super::docker_output(
                    &[
                        "ps",
                        "-a",
                        "--filter",
                        &format!("name={}", names[0]),
                        "--format",
                        "{{.Names}}",
                    ],
                    1.0,
                    128,
                )
                .await;
                if super::docker_command_succeeded(&listed)
                    && !listed.stdout.lines().any(|name| name == names[0])
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        let other = super::docker_output(
            &["inspect", "--format", "{{.State.Running}}", &names[1]],
            1.0,
            128,
        )
        .await;
        tasks[1].abort();
        let second_removed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let listed = super::docker_output(
                    &[
                        "ps",
                        "-a",
                        "--filter",
                        &format!("name={}", names[1]),
                        "--format",
                        "{{.Names}}",
                    ],
                    1.0,
                    128,
                )
                .await;
                if super::docker_command_succeeded(&listed)
                    && !listed.stdout.lines().any(|name| name == names[1])
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        for task in tasks {
            let _ = task.await;
        }
        // Exact-name fallback cleanup runs even if an assertion will fail.
        if first_removed.is_err() || second_removed.is_err() {
            for name in &names {
                let _ = super::docker_output(&["rm", "-f", name], 3.0, 128).await;
            }
        }
        assert!(ready.is_ok(), "containers did not become ready");
        assert!(
            second_removed.is_ok(),
            "second cancelled container remained allocated"
        );
        assert!(
            first_removed.is_ok(),
            "cancelled container remained allocated"
        );
        assert!(
            super::docker_command_succeeded(&other) && other.stdout.trim() == "true",
            "cancelling one run stopped its neighbor: {other:?}"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_container_lifecycle_retains_lease_and_removes_exact_container() {
        use std::os::unix::fs::PermissionsExt;
        async fn scenario(operation: &str, identity: &str, supervised: bool) {
            let control = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let lease = workspace.path().to_path_buf();
            let generated = std::sync::Arc::new(tempfile::tempdir().unwrap());
            let guard = std::sync::Arc::new(
                super::create_network_guard(crate::types::RuntimeProfile::LocalTrusted, None)
                    .unwrap(),
            );
            let resolver =
                super::create_node_package_resolver(crate::types::RuntimeProfile::LocalTrusted)
                    .unwrap();
            let retained = [
                lease.clone(),
                generated.path().to_path_buf(),
                guard._directory.path().to_path_buf(),
                resolver._directory.path().to_path_buf(),
            ];
            let lease_checks = retained
                .iter()
                .map(|path| format!("[ -d '{}' ] || exit 9;", path.display()))
                .collect::<Vec<_>>()
                .join(" ");
            let ready = control.path().join("ready");
            let created = control.path().join("created");
            let removed = control.path().join("removed");
            let calls = control.path().join("calls");
            let program = control.path().join("docker");
            let delay = if operation == "wait" { "3" } else { "0.3" };
            std::fs::write(&program, format!("#!/bin/sh\necho \"$1\" >> '{}'\nif [ \"$1\" = '{operation}' ]; then echo ready > '{}'; /bin/sleep {delay}; fi\ncase \"$1\" in\ncreate) echo created > '{}' ;;\ninspect) echo '{{\"ExitCode\":0,\"OOMKilled\":false,\"Running\":false}}' ;;\nrm) {lease_checks} /bin/rm -f '{}'; echo \"$3\" >> '{}' ;;\nesac\nexit 0\n", calls.display(), ready.display(), created.display(), created.display(), removed.display())).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let lifecycle = super::DockerLifecycle {
                program,
                create: vec!["create".into(), "--name".into(), identity.into()],
                container: identity.into(),
                started: std::time::Instant::now(),
                limits: crate::types::ExecutionLimits {
                    timeout_seconds: 5.0,
                    memory_mb: 128,
                    runtime_profile: crate::types::RuntimeProfile::Isolated,
                    network_policy: crate::types::NetworkPolicy::Deny,
                },
                adapter: None,
                _workspace: Some(workspace),
                _generated_workspace: Some(generated),
                _runtime_guard: Some(guard),
                _resolver: Some(resolver),
            };
            let caller = if supervised {
                tokio::spawn(super::supervise_docker_lifecycle(lifecycle))
            } else {
                tokio::spawn(lifecycle.run(std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(false),
                )))
            };
            tokio::time::timeout(std::time::Duration::from_secs(4), async {
                while !ready.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fixture did not reach cancellation phase");
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            if !supervised {
                assert!(
                    created.exists(),
                    "negative control did not create a container"
                );
                assert!(
                    !removed.exists(),
                    "unsupervised control unexpectedly cleaned up"
                );
                assert!(
                    !lease.exists(),
                    "unsupervised control unexpectedly retained its lease"
                );
                return;
            }
            tokio::time::timeout(std::time::Duration::from_secs(4), async {
                while !removed.exists() || lease.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("cancelled container was not cleaned with its lease retained");
            assert_eq!(std::fs::read_to_string(&removed).unwrap().trim(), identity);
            assert!(!created.exists());
            assert!(retained.iter().all(|path| !path.exists()));
            let calls = std::fs::read_to_string(calls).unwrap();
            assert_eq!(calls.lines().filter(|line| *line == "rm").count(), 1);
            if operation == "create" {
                assert!(!calls.lines().any(|line| line == "start"));
            }
            if operation == "start" {
                assert!(!calls.lines().any(|line| line == "wait"));
            }
        }
        for operation in ["create", "start", "wait", "rm"] {
            tokio::join!(
                scenario(operation, "owned-container-a", true),
                scenario(operation, "owned-container-b", true)
            );
        }
        scenario("wait", "unowned-negative-control", false).await;
    }
}
