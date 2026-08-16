use court_jester::tools::sandbox::parse_harness_events;
use court_jester::types::*;

#[test]
fn typed_diagnostics_round_trip_and_old_v3_defaults() {
    let diagnostic = FailureDiagnostic {
        domain: FailureDomain::Resource,
        kind: FailureKind::MemoryLimit,
        component: DiagnosticComponent::Sandbox,
        impact: DiagnosticImpact::Blocking,
        message: "memory cap reached".into(),
        process: Some(ProcessTermination {
            kind: ProcessTerminationKind::MemoryLimit,
            exit_code: None,
            signal: Some(9),
            signal_name: Some("SIGKILL".into()),
        }),
        limits: Some(ExecutionLimits {
            timeout_seconds: 10.0,
            memory_mb: 64,
            runtime_profile: RuntimeProfile::LocalTrusted,
            network_policy: NetworkPolicy::Deny,
        }),
    };
    let json = serde_json::to_string(&diagnostic).unwrap();
    let decoded: FailureDiagnostic = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, diagnostic);

    let old = serde_json::json!({
        "schema_version": 3,
        "meta": {"language": "python", "timestamp": "2026-01-01T00:00:00Z", "duration_ms": 1},
        "stages": [],
        "verdict": "pass",
        "strength": "parse_only",
        "summary": {
            "functions_analyzed": 0,
            "functions_fuzzed": 0,
            "functions_skipped": 0,
            "functions_blocked_module_load": 0,
            "fuzz_pass": 0,
            "fuzz_no_inputs_reached": 0,
            "findings": {"total": 0, "gating": 0, "advisory": 0, "suppressed": 0},
            "suppressed_complexity_violations": 0,
            "suppressed_portability_warnings": 0,
            "lint_issues": 0,
            "lint_runner_failures": 0,
            "complexity_violations": 0,
            "coverage": {"required": 0, "behaviorally_checked": 0, "reached_only": 0, "no_inputs_reached": 0, "skipped": 0, "blocked": 0}
        }
    });
    let report: PersistedReport = serde_json::from_value(old).unwrap();
    assert_eq!(report.schema_version, REPORT_SCHEMA_VERSION);
    assert!(report.diagnostics.is_empty());
    assert!(report.diagnostics_summary.is_none());
    assert_eq!(report.summary.diagnostics.total, 0);
}

#[test]
fn bun_junit_adapter_preserves_public_variant_and_serde_value() {
    let adapter = TestAdapter::BunJunit;
    assert_eq!(
        serde_json::to_value(adapter).unwrap(),
        serde_json::json!("bun_junit")
    );
    assert_eq!(
        serde_json::from_value::<TestAdapter>(serde_json::json!("bun_junit")).unwrap(),
        adapter
    );
}
#[test]
fn project_adapter_contract_preserves_capability_and_strategy_values() {
    let adapter = ProjectAdapterContract {
        kind: ProjectAdapterKind::Nuxt,
        root: "/workspace/apps/client".into(),
        package_root: "/workspace/apps/client".into(),
        workspace_root: "/workspace".into(),
        dependency_roots: vec!["/workspace".into()],
        effective_config: Some("/workspace/apps/client/.nuxt/tsconfig.json".into()),
        selected_runner: Some(ProjectRuntimeAdapterKind::Nuxt),
        rationale: vec!["Nuxt configuration selected the adapter".into()],
        capabilities: ProjectAdapterCapabilities {
            authoritative_source_overlay: true,
            package_runtime: true,
            project_test_runner: true,
            framework_auto_import_runtime: true,
        },
    };
    let value = serde_json::to_value(&adapter).unwrap();
    assert_eq!(value["kind"], "nuxt");
    assert_eq!(value["capabilities"]["framework_auto_import_runtime"], true);
    assert_eq!(value["selected_runner"], "nuxt");
    assert_eq!(value["workspace_root"], "/workspace");
    assert_eq!(
        serde_json::to_value(SurfaceExecutionStrategy::FrameworkRuntime).unwrap(),
        "framework_runtime"
    );
    assert_eq!(
        serde_json::from_value::<ProjectAdapterContract>(value).unwrap(),
        adapter
    );
}

#[test]
fn harness_event_parser_enforces_version_order_bounds_and_deduplication() {
    let bootstrap =
        serde_json::json!({"protocol_version": 1, "sequence": 0, "event": "bootstrap_started"});
    let resolved = serde_json::json!({"protocol_version": 1, "sequence": 1, "event": "target_resolved", "data": {"module": "target"}});
    let ready = serde_json::json!({"protocol_version": 1, "sequence": 2, "event": "target_ready"});
    let completed = serde_json::json!({"protocol_version": 1, "sequence": 3, "event": "harness_completed", "data": {"completed_units": 0}});
    let input = [bootstrap.clone(), resolved, ready, completed]
        .into_iter()
        .map(|record| {
            format!(
                "{}{}",
                court_jester::tools::sandbox::HARNESS_EVENT_SENTINEL,
                record
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = parse_harness_events(&input).unwrap();
    assert!(parsed.harness_completed);
    assert!(parsed.target_ready);

    let duplicate = format!(
        "{}\n{}{}",
        input,
        court_jester::tools::sandbox::HARNESS_EVENT_SENTINEL,
        bootstrap
    );
    assert!(parse_harness_events(&duplicate).is_err());
    let bad_version = input.replace("\"protocol_version\":1", "\"protocol_version\":2");
    assert!(parse_harness_events(&bad_version).is_err());
}
