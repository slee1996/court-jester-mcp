use court_jester_mcp::tools::verify::{
    parse_fuzz_failures, report_json_value, verify, VerifyOptions,
};
use court_jester_mcp::types::{ComplexityMetric, ExecuteGate, Language, ReportLevel, TestRunner};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn install_fake_tool_at(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script_path = dir.join(name);
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&script_path, body).unwrap();
    #[cfg(unix)]
    make_executable(&script_path);
    script_path
}

fn normalize_logged_path(path: &str) -> String {
    path.trim()
        .strip_prefix("/private")
        .unwrap_or(path.trim())
        .to_string()
}

fn assert_log_contains_path(log: &str, prefix: &str, expected: &Path) {
    let expected = normalize_logged_path(&expected.to_string_lossy());
    assert!(
        log.lines().any(|line| {
            line.strip_prefix(prefix)
                .map(normalize_logged_path)
                .as_deref()
                == Some(expected.as_str())
        }),
        "expected log to contain {prefix}{expected}, got:\n{log}"
    );
}

fn default_opts(test_code: Option<&str>) -> VerifyOptions<'_> {
    VerifyOptions {
        test_code,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    }
}

#[tokio::test]
async fn good_python_function() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(
        report.stages.iter().any(|s| s.name == "parse" && s.ok),
        "parse stage should pass"
    );
    // execute stage should also pass (42 + 42 doesn't error)
    if let Some(exec) = report.stages.iter().find(|s| s.name == "execute") {
        assert!(exec.ok, "execute stage failed: {:?}", exec.error);
    }
}

#[tokio::test]
async fn syntax_error_short_circuits() {
    let code = "def foo(:";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok);
    assert_eq!(report.stages.len(), 1, "should short-circuit after parse");
    assert_eq!(report.stages[0].name, "parse");
    assert!(!report.stages[0].ok);
}

#[tokio::test]
async fn with_passing_tests() {
    let code = "def double(x: int) -> int:\n    return x * 2";
    let tests = "assert double(5) == 10\nassert double(0) == 0";
    let report = verify(code, &Language::Python, default_opts(Some(tests))).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn tests_only_verify_skips_execute_stage() {
    let code = "def inverse(x: int) -> float:\n    return 1 / x";
    let tests = "assert inverse(2) == 0.5";
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn tests_only_verify_requires_authoritative_test() {
    let code = "def inverse(x: int) -> float:\n    return 1 / x";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: true,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    let test_stage = report
        .stages
        .iter()
        .find(|s| s.name == "test")
        .expect("tests_only mode should emit a failing test stage");
    assert!(!test_stage.ok);
    assert_eq!(
        test_stage.error.as_deref(),
        Some("tests_only mode requires an authoritative test")
    );
}

#[tokio::test]
async fn with_failing_tests() {
    let code = "def double(x: int) -> int:\n    return x * 3"; // bug: *3 instead of *2
    let tests = "assert double(5) == 10";
    let report = verify(code, &Language::Python, default_opts(Some(tests))).await;

    assert!(!report.overall_ok);
    assert!(report.stages.iter().any(|s| s.name == "test" && !s.ok));
}

#[tokio::test]
async fn lint_warnings_are_informational() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join("node_modules").join(".bin");
    install_fake_tool_at(
        &tool_dir,
        "biome",
        "#!/bin/sh\ncat <<'EOF'\n{\"diagnostics\":[{\"category\":\"lint/style/noNonNullAssertion\",\"description\":\"Avoid non-null assertions.\",\"severity\":\"warning\",\"location\":{\"start\":{\"line\":3,\"column\":12}}}]}\nEOF\nexit 1\n",
    );

    let code = r#"
function normalizeName(name: string): string {
    return name!.trim();
}
"#;
    let mut opts = default_opts(None);
    opts.project_dir = Some(project_dir.path().to_str().unwrap());
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    assert!(lint_stage.ok, "lint warnings should not fail verify");

    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        !diagnostics.is_empty(),
        "expected lint diagnostics to remain in the report"
    );
}

#[tokio::test]
async fn verify_passes_project_local_lint_context_to_ruff() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join(".venv").join("bin");
    let log_path = project_dir.path().join("ruff-verify.log");
    let config_path = project_dir.path().join("ruff.toml");
    let source_path = project_dir.path().join("src").join("account.py");
    std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();

    let code = "def add(a: int, b: int) -> int:\n    return a + b\n";
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&config_path, "[lint]\n").unwrap();

    install_fake_tool_at(
        &tool_dir,
        "ruff",
        &format!(
            r#"#!/bin/sh
printf 'cwd=%s\n' "$PWD" > "{log}"
for arg in "$@"; do
  printf 'arg=%s\n' "$arg" >> "{log}"
done
cat <<'EOF'
[{{"code":"F841","message":"local variable is assigned to but never used","location":{{"row":1,"column":1}}}}]
EOF
exit 1
"#,
            log = log_path.display(),
        ),
    );

    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(project_dir.path().to_str().unwrap()),
            lint_config_path: Some(config_path.to_str().unwrap()),
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    assert!(
        report.overall_ok,
        "lint diagnostics should stay informational"
    );

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        diagnostics
            .iter()
            .any(|diag| diag.get("rule").and_then(|value| value.as_str()) == Some("F841")),
        "real-file verify runs should keep file-aware unused-variable diagnostics"
    );

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert_log_contains_path(&log, "cwd=", project_dir.path());
    assert!(log.contains("arg=check"));
    assert!(log.contains("arg=--config"));
    assert_log_contains_path(&log, "arg=", &config_path);
    assert_log_contains_path(&log, "arg=", &source_path);
}

#[tokio::test]
async fn verify_filters_unused_variable_diagnostics_for_anonymous_inline_snippets() {
    let project_dir = tempfile::tempdir().unwrap();
    let tool_dir = project_dir.path().join(".venv").join("bin");
    install_fake_tool_at(
        &tool_dir,
        "ruff",
        "#!/bin/sh\ncat <<'EOF'\n[{\"code\":\"F841\",\"message\":\"assigned but unused\",\"location\":{\"row\":1,\"column\":1}}]\nEOF\nexit 1\n",
    );

    let code = "def add(a: int, b: int) -> int:\n    return a + b\n";
    let mut opts = default_opts(None);
    opts.project_dir = Some(project_dir.path().to_str().unwrap());
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.overall_ok,
        "snippet-only unused diagnostics should not fail verify"
    );

    let lint_stage = report
        .stages
        .iter()
        .find(|s| s.name == "lint")
        .expect("lint stage should be present");
    let diagnostics = lint_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .expect("lint diagnostics should be present");
    assert!(
        diagnostics.is_empty(),
        "anonymous inline snippets should continue filtering unused-variable false positives"
    );
}

#[tokio::test]
async fn blank_label_output_fails_verify() {
    let code = r#"
function secondaryLabel(labels: string[]): string {
    if (labels.length < 2) return "general";
    return labels[1].trim().toLowerCase();
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(!exec_stage.ok, "blank string output should fail verify");
}

#[tokio::test]
async fn blank_city_output_fails_verify() {
    let code = r#"
type User = {
    address?: {
        city?: string | null;
    } | null;
} | null;

function primaryCity(user: User): string {
    const city = user?.address?.city;
    return city ? city.trim() : "Unknown";
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(!exec_stage.ok, "blank city output should fail verify");
}

#[tokio::test]
async fn missing_preferred_timezone_fails_verify() {
    let code = r#"
def preferred_timezone(profile: dict | None) -> str:
    return profile["preferences"]["timezone"].strip()
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "missing nested preference data should fail verify"
    );
}

#[tokio::test]
async fn feature_flag_nested_none_fails_verify() {
    let code = r#"
def beta_checkout_enabled(config: dict | None) -> bool:
    value = (config or {}).get("flags", {}).get("beta_checkout")
    if value is None:
        return True
    return value
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "feature flag resolver should fail verify when nested flags=None crashes"
    );
}

#[tokio::test]
async fn typescript_feature_flag_explicit_false_fails_verify() {
    let code = r#"
type Config = {
  flags?: {
    betaCheckout?: boolean | null;
  } | null;
} | null;

function defaultFlags(): { betaCheckout: boolean } {
  return { betaCheckout: true };
}

export function betaCheckoutEnabled(config: Config): boolean {
  return config?.flags?.betaCheckout || defaultFlags().betaCheckout;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "feature flag resolver should fail verify when explicit false is overridden"
    );
}

#[tokio::test]
async fn typescript_feature_flag_explicit_false_can_pass_verify() {
    let code = r#"
type Config = {
  flags?: {
    betaCheckout?: boolean | null;
  } | null;
} | null;

function defaultFlags(): { betaCheckout: boolean } {
  return { betaCheckout: true };
}

export function betaCheckoutEnabled(config: Config): boolean {
  return config?.flags?.betaCheckout ?? defaultFlags().betaCheckout;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn typescript_semver_compare_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  return 0;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "semver compare should fail verify when prerelease ordering is ignored"
    );
}

#[tokio::test]
async fn typescript_semver_compare_prerelease_can_pass_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareIdentifiers(left: string, right: string): number {
  const leftNumeric = /^\d+$/.test(left);
  const rightNumeric = /^\d+$/.test(right);
  if (leftNumeric && rightNumeric) {
    const a = Number.parseInt(left, 10);
    const b = Number.parseInt(right, 10);
    return a === b ? 0 : a < b ? -1 : 1;
  }
  if (leftNumeric) return -1;
  if (rightNumeric) return 1;
  return left === right ? 0 : left < right ? -1 : 1;
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return 0;
  }
  if (a.major !== b.major) return a.major < b.major ? -1 : 1;
  if (a.minor !== b.minor) return a.minor < b.minor ? -1 : 1;
  if (a.patch !== b.patch) return a.patch < b.patch ? -1 : 1;
  if (a.prerelease == null && b.prerelease == null) return 0;
  if (a.prerelease == null) return 1;
  if (b.prerelease == null) return -1;
  for (let i = 0; i < Math.min(a.prerelease.length, b.prerelease.length); i++) {
    const cmp = compareIdentifiers(a.prerelease[i], b.prerelease[i]);
    if (cmp !== 0) return cmp;
  }
  if (a.prerelease.length === b.prerelease.length) return 0;
  return a.prerelease.length < b.prerelease.length ? -1 : 1;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn typescript_semver_caret_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  return candidate.major === base.major;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "semver caret matcher should fail verify when prereleases and zero-major bounds are ignored"
    );
}

#[tokio::test]
async fn typescript_semver_caret_prerelease_can_pass_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base || candidate.prerelease != null) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  if (base.major > 0) {
    return candidate.major === base.major;
  }
  if (base.minor > 0) {
    return candidate.major === 0 && candidate.minor === base.minor;
  }
  return candidate.major === 0 && candidate.minor === 0 && candidate.patch === base.patch;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn typescript_semver_caret_same_core_prerelease_fails_verify() {
    let code = r#"
type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[] | null;
};

function parseVersion(input: string): ParsedVersion | null {
  const normalized = input.trim().replace(/^v/i, "").split("+", 1)[0];
  const [core, prereleaseText] = normalized.split("-", 2);
  const parts = core.split(".");
  if (parts.length !== 3) {
    return null;
  }
  const [major, minor, patch] = parts.map((part) => Number.parseInt(part, 10));
  if ([major, minor, patch].some((part) => Number.isNaN(part) || part < 0)) {
    return null;
  }
  return {
    major,
    minor,
    patch,
    prerelease: prereleaseText ? prereleaseText.split(".") : null,
  };
}

function compareCore(left: ParsedVersion, right: ParsedVersion): number {
  if (left.major !== right.major) return left.major < right.major ? -1 : 1;
  if (left.minor !== right.minor) return left.minor < right.minor ? -1 : 1;
  if (left.patch !== right.patch) return left.patch < right.patch ? -1 : 1;
  return 0;
}

export function matchesCaret(version: string, range: string): boolean {
  if (!range.startsWith("^")) {
    return false;
  }
  const candidate = parseVersion(version);
  const base = parseVersion(range.slice(1));
  if (!candidate || !base) {
    return false;
  }
  if (compareCore(candidate, base) < 0) {
    return false;
  }
  if (candidate.prerelease) {
    if (
      candidate.major !== base.major ||
      candidate.minor !== base.minor ||
      candidate.patch !== base.patch
    ) {
      return false;
    }
  }
  if (base.major !== 0) {
    return candidate.major === base.major;
  }
  if (base.minor !== 0) {
    return candidate.major === base.major && candidate.minor === base.minor;
  }
  return candidate.major === 0 && candidate.minor === 0 && candidate.patch === base.patch;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "semver caret matcher should fail verify when a stable range still admits same-core prereleases"
    );
}

#[tokio::test]
async fn typescript_defaults_null_override_and_inherited_keys_fail_verify() {
    let code = r#"
const objectProto = Object.prototype;

function shouldAssignDefault(object: Record<string, unknown>, key: string): boolean {
  const value = object[key];
  return value == null || (value === objectProto[key] && !Object.hasOwn(object, key));
}

export function defaults<T extends object>(object: T, ...sources: Array<object | null | undefined>): T {
  const target = Object(object) as Record<string, unknown>;
  for (const source of sources) {
    if (source == null) {
      continue;
    }
    for (const key of Object.keys(source)) {
      if (shouldAssignDefault(target, key)) {
        target[key] = (source as Record<string, unknown>)[key];
      }
    }
  }
  return target as T;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "defaults should fail verify when null targets are overwritten or inherited keys are skipped"
    );
}

#[tokio::test]
async fn typescript_defaults_null_override_and_inherited_keys_can_pass_verify() {
    let code = r#"
const objectProto = Object.prototype;

function shouldAssignDefault(object: Record<string, unknown>, key: string): boolean {
  const value = object[key];
  return value === undefined || (value === objectProto[key] && !Object.hasOwn(object, key));
}

export function defaults<T extends object>(object: T, ...sources: Array<object | null | undefined>): T {
  const target = Object(object) as Record<string, unknown>;
  for (const source of sources) {
    if (source == null) {
      continue;
    }
    for (const key in Object(source)) {
      if (shouldAssignDefault(target, key)) {
        target[key] = (source as Record<string, unknown>)[key];
      }
    }
  }
  return target as T;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn query_string_nullish_leak_fails_verify() {
    let code = r#"
from urllib.parse import quote_plus

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        value = params[key]
        if value is None:
            continue
        if isinstance(value, list):
            for item in value:
                parts.append(f"{quote_plus(str(key))}={quote_plus(str(item).strip())}")
        else:
            parts.append(f"{quote_plus(str(key))}={quote_plus(str(value).strip())}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "query-like serialization that leaks None/null should fail verify"
    );
}

#[tokio::test]
async fn query_string_blank_and_unicode_semantics_fail_verify() {
    let code = r#"
from urllib.parse import quote_plus

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        value = params[key]
        if value is None:
            continue
        if isinstance(value, list):
            for item in value:
                if item is None:
                    continue
                parts.append(f"{quote_plus(str(key))}={quote_plus(str(item).strip())}")
        else:
            parts.append(f"{quote_plus(str(key))}={quote_plus(str(value).strip())}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "query-like serialization that keeps blanks or accents should fail verify"
    );
}

#[tokio::test]
async fn query_string_canonicalization_can_pass_verify() {
    let code = r#"
from urllib.parse import quote_plus
import unicodedata

def _canonical_scalar(value: object) -> str | None:
    if value is None or isinstance(value, (dict, list, tuple, set)):
        return None
    text = unicodedata.normalize("NFKD", str(value).strip()).encode("ascii", "ignore").decode("ascii")
    return text or None

def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        raw = params[key]
        values = raw if isinstance(raw, list) else [raw]
        for item in values:
            text = _canonical_scalar(item)
            if text is None:
                continue
            parts.append(f"{quote_plus(str(key))}={quote_plus(text)}")
    return "&".join(parts)
"#;
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn typescript_query_string_blank_and_unicode_semantics_fail_verify() {
    let code = r#"
export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const value = params[key];
    if (value == null) {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        if (item == null) {
          continue;
        }
        entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(item).trim())}`);
      }
    } else {
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value).trim())}`);
    }
  }
  return entries.join("&");
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(!report.overall_ok, "report: {:#?}", report.stages);

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        !exec_stage.ok,
        "query-like serialization that keeps blanks or accents should fail verify"
    );
}

#[tokio::test]
async fn typescript_query_string_canonicalization_can_pass_verify() {
    let code = r#"
function canonicalScalar(value: unknown): string | null {
  if (value == null || Array.isArray(value) || (typeof value === "object" && value !== null)) {
    return null;
  }
  const text = String(value).trim().normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
  return text.length > 0 ? text : null;
}

export function canonicalQuery(params: Record<string, unknown>): string {
  const entries: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const raw = params[key];
    const values = Array.isArray(raw) ? raw : [raw];
    for (const item of values) {
      const text = canonicalScalar(item);
      if (text == null) {
        continue;
      }
      entries.push(`${encodeURIComponent(key)}=${encodeURIComponent(text)}`);
    }
  }
  return entries.join("&");
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
}

#[tokio::test]
async fn python_test_stage_can_import_source_module_from_sibling_path() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("billing.py");
    let code = "def billing_country(order: dict | None) -> str:\n    return \"US\"";
    std::fs::write(&source_path, code).unwrap();

    let tests = "from billing import billing_country\nassert billing_country(None) == \"US\"";
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn verify_with_threshold_adds_stage() {
    let code = "def complex_fn(x: int) -> int:\n    if x > 0:\n        for i in range(x):\n            if i > 5:\n                return i\n    return x";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: Some(3),
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;
    assert!(
        report.stages.iter().any(|s| s.name == "complexity"),
        "should have complexity stage"
    );
}

#[tokio::test]
async fn verify_complexity_threshold_scopes_to_changed_functions_in_diff_mode() {
    let code = "\
def legacy_complex(x: int) -> int:
    if x > 0:
        for i in range(x):
            if i > 5:
                return i
    return x

def changed(x: int) -> int:
    return x + 1
";
    let diff = "@@ -8,2 +8,2 @@\n+def changed(x: int) -> int:\n+    return x + 1\n";
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: Some(3),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: Some(diff),
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|s| s.name == "complexity")
        .expect("complexity stage should be present");
    assert!(
        complexity_stage.ok,
        "only the changed simple function should be checked in diff mode"
    );
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["checked_functions"].as_u64(), Some(1));
    assert_eq!(detail["diff_scoped"].as_bool(), Some(true));
}

#[tokio::test]
async fn verify_complexity_stage_reports_cognitive_and_breakdown_details() {
    let code = "\
def classify(x: int) -> str:
    match x:
        case 0:
            return \"zero\"
        case 1:
            return \"one\"
        case _:
            return \"other\"
";
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: Some(2),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|s| s.name == "complexity")
        .expect("complexity stage should be present");
    assert!(!complexity_stage.ok, "match/case should exceed threshold 2");

    let violations = complexity_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("violations"))
        .and_then(|value| value.as_array())
        .expect("violations should be present");
    assert_eq!(violations.len(), 1);
    assert!(
        violations[0]["cognitive_complexity"].as_u64().unwrap_or(0) > 0,
        "violation should include cognitive complexity"
    );
    assert_eq!(
        violations[0]["complexity_breakdown"]["case"].as_u64(),
        Some(3)
    );
}

#[tokio::test]
async fn verify_can_gate_on_cognitive_complexity() {
    let code = r#"
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: Some(5),
            complexity_metric: ComplexityMetric::Cognitive,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: None,
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    let complexity_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
        .expect("complexity stage should be present");
    assert!(
        !complexity_stage.ok,
        "cognitive complexity should exceed threshold 5"
    );
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["metric"].as_str(), Some("cognitive"));
    let violations = detail["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["function"].as_str(), Some("check_access"));
}

#[tokio::test]
async fn verify_without_threshold_no_stage() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert!(
        !report.stages.iter().any(|s| s.name == "complexity"),
        "should NOT have complexity stage"
    );
}

#[test]
fn parse_fuzz_failures_from_stdout() {
    let stdout = r#"FUZZ greet: 30 passed, 0 rejected (of 30)
__COURT_JESTER_FUZZ_JSON__
[{"function":"boom","input":"[42]","error_type":"TypeError","message":"bad","severity":"crash"}]
"#;
    let failures = parse_fuzz_failures(stdout);
    assert!(failures.is_some());
    let failures = failures.unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].function, "boom");
    assert_eq!(failures[0].severity, "crash");
}

#[test]
fn parse_fuzz_failures_no_sentinel() {
    let stdout = "FUZZ greet: 30 passed, 0 rejected (of 30)\nAll fuzz tests passed\n";
    assert!(parse_fuzz_failures(stdout).is_none());
}

#[tokio::test]
async fn verify_diff_mode_only_fuzzes_changed() {
    // Two functions, diff only touches the second one
    let code = "\
def untouched(x: int) -> int:
    return x

def changed(x: int) -> int:
    return x + 1
";
    // Diff touching lines 4-5 (the changed function)
    let diff = "@@ -4,2 +4,2 @@\n+def changed(x: int) -> int:\n+    return x + 1\n";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: Some(diff),
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;
    // Should pass since changed() is a simple function
    if let Some(exec) = report.stages.iter().find(|s| s.name == "execute") {
        // The fuzz should only test the changed function
        let detail = exec.detail.as_ref().unwrap();
        let stdout = detail["stdout"].as_str().unwrap_or("");
        // untouched should NOT appear in fuzz output
        assert!(
            !stdout.contains("FUZZ untouched"),
            "untouched should not be fuzzed in diff mode, got: {stdout}"
        );
    }
    let coverage = report
        .stages
        .iter()
        .find(|s| s.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .expect("coverage stage should be present");
    assert_eq!(coverage["diff_scoped"].as_bool(), Some(true));
}

#[tokio::test]
async fn writes_report_to_output_dir() {
    let dir = tempfile::tempdir().unwrap();
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.report_path.is_some(), "should have report_path");
    let path = report.report_path.unwrap();
    assert!(
        std::path::Path::new(&path).exists(),
        "report file should exist"
    );

    // Verify it's valid JSON with expected structure
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        parsed
            .get("schema_version")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert!(parsed.get("meta").is_some());
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("stages").is_some());
    assert!(parsed.get("overall_ok").is_some());
}

#[tokio::test]
async fn minimal_report_level_omits_full_parse_detail() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let mut opts = default_opts(None);
    opts.report_level = ReportLevel::Minimal;
    let report = verify(code, &Language::Python, opts).await;
    let json = report_json_value(&report, ReportLevel::Minimal);

    assert_eq!(json["schema_version"].as_u64(), Some(2));
    assert!(json.get("summary").is_some());

    let parse_stage = json["stages"]
        .as_array()
        .and_then(|stages| {
            stages
                .iter()
                .find(|stage| stage.get("name").and_then(|value| value.as_str()) == Some("parse"))
        })
        .expect("parse stage should be present");
    assert!(
        parse_stage.get("detail").is_none(),
        "minimal report should omit full parse detail: {parse_stage:?}"
    );
}

#[tokio::test]
async fn no_report_without_output_dir() {
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let report = verify(code, &Language::Python, default_opts(None)).await;
    assert!(
        report.report_path.is_none(),
        "should NOT have report_path when output_dir not set"
    );
}

#[tokio::test]
async fn rejected_only_fuzz_run_is_not_counted_as_pass_in_report_summary() {
    let dir = tempfile::tempdir().unwrap();
    let code = "class ValidationError(Exception):\n    pass\n\ndef always_reject(x: int) -> int:\n    raise ValidationError('nope')";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        report.overall_ok,
        "rejected-only fuzz run should be diagnostic only"
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.ok,
        "execute stage should stay green for no-inputs-only runs"
    );
    let execute_detail = execute_stage.detail.as_ref().unwrap();
    assert_eq!(execute_detail["no_inputs_reached"].as_u64(), Some(1));
    assert_eq!(execute_detail["execute_gate_failed"].as_bool(), Some(false));

    let path = report.report_path.unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let summary = parsed.get("summary").unwrap();
    assert_eq!(summary.get("functions_fuzzed").unwrap().as_u64(), Some(1));
    assert_eq!(summary.get("fuzz_pass").unwrap().as_u64(), Some(0));
    assert_eq!(summary.get("fuzz_crash").unwrap().as_u64(), Some(0));
    assert_eq!(
        summary.get("fuzz_no_inputs_reached").unwrap().as_u64(),
        Some(1)
    );
}

#[tokio::test]
async fn execute_gate_crash_allows_property_violations() {
    let code = r#"
export function compareScore(a: number, b: number): number {
  return 1;
}
"#;

    let report_default = verify(code, &Language::TypeScript, default_opts(None)).await;
    assert!(
        !report_default.overall_ok,
        "default execute gate should fail property violations: {:#?}",
        report_default.stages
    );

    let mut crash_only_opts = default_opts(None);
    crash_only_opts.execute_gate = ExecuteGate::Crash;
    let report = verify(code, &Language::TypeScript, crash_only_opts).await;
    assert!(
        report.overall_ok,
        "crash-only execute gate should allow property violations: {:#?}",
        report.stages
    );

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.ok,
        "execute stage should remain green under crash gate"
    );
    let detail = execute_stage.detail.as_ref().unwrap();
    assert_eq!(detail["execute_gate"].as_str(), Some("crash"));
    assert_eq!(
        detail["finding_counts"]["property_violation"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        true
    );
    assert_eq!(detail["execute_gate_failed"].as_bool(), Some(false));
}

#[tokio::test]
async fn execute_findings_can_be_suppressed_without_failing_verify() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("first_char.py");
    let code = "def first_char(s: str) -> str:\n    return s[0]\n";
    std::fs::write(&source_path, code).unwrap();

    let suppressions = r#"
{
  "rules": [
    {
      "path": "first_char.py",
      "stage": "execute",
      "function": "first_char",
      "severity": "crash",
      "error_type": "IndexError"
    }
  ]
}
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: Some(suppressions),
            suppression_source: Some(".court-jester-ignore.json"),
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    assert!(
        report.overall_ok,
        "suppressed execute finding should not fail verify"
    );
    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.ok,
        "execute stage should stay green when all findings are suppressed"
    );
    let detail = execute_stage.detail.as_ref().unwrap();
    assert_eq!(
        detail["suppression_source"].as_str(),
        Some(".court-jester-ignore.json")
    );
    assert_eq!(detail["finding_counts"]["crash"].as_u64(), Some(0));
    assert!(
        detail["suppressed_finding_counts"]["crash"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "expected suppressed crash findings"
    );
    let suppressed = detail["suppressed_fuzz_failures"].as_array().unwrap();
    assert!(!suppressed.is_empty(), "expected suppressed fuzz failures");
    assert_eq!(suppressed[0]["function"].as_str(), Some("first_char"));
    assert!(report.summary.suppressed_fuzz_findings > 0);
}

#[tokio::test]
async fn complexity_violations_can_be_suppressed_by_function_name() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("authz.py");
    let code = r#"
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
    std::fs::write(&source_path, code).unwrap();
    let suppressions = r#"
{
  "rules": [
    {
      "path": "authz.py",
      "stage": "complexity",
      "function": "check_access"
    }
  ]
}
"#;
    let report = verify(
        code,
        &Language::Python,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: Some(2),
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: Some(suppressions),
            suppression_source: Some(".court-jester-ignore.json"),
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    assert!(
        report.overall_ok,
        "suppressed complexity violation should not fail verify"
    );
    let complexity_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "complexity")
        .expect("complexity stage should be present");
    assert!(complexity_stage.ok);
    let detail = complexity_stage.detail.as_ref().unwrap();
    assert_eq!(detail["violations"].as_array().unwrap().len(), 0);
    assert_eq!(detail["suppressed_violations"].as_array().unwrap().len(), 1);
    assert_eq!(report.summary.suppressed_complexity_violations, 1);
}

#[tokio::test]
async fn value_error_is_treated_as_a_crash() {
    let code = "def normalize_timezone(value: str) -> str:\n    raise ValueError('invalid timezone offset')";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    assert!(!report.overall_ok, "value errors should fail verify");

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(!exec_stage.ok, "value error should be treated as a crash");

    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("fuzz_failures"))
        .and_then(|value| value.as_array())
        .expect("fuzz failures should be present");
    assert!(
        failures.iter().any(
            |failure| failure.get("error_type").and_then(|value| value.as_str())
                == Some("ValueError")
        ),
        "expected ValueError fuzz failure, got: {failures:?}"
    );
}

#[tokio::test]
async fn typescript_malformed_uri_is_treated_as_reject_not_crash() {
    let code = r#"
export function decodeSegment(value: string): string {
    return decodeURIComponent(value);
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(
        report.overall_ok,
        "malformed URI inputs should be rejected, not fail verify: {:#?}",
        report.stages
    );

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    assert!(
        exec_stage.ok,
        "execute stage should pass: {:?}",
        exec_stage.error
    );

    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("fuzz_failures"));
    assert!(
        failures.is_none(),
        "malformed URI rejections should not be recorded as fuzz failures: {failures:?}"
    );
}

#[tokio::test]
async fn verify_reports_per_function_fuzz_coverage_honestly() {
    let code = r#"
export function verifyRequest(request: Request): boolean {
  return request.headers.has("authorization");
}

function parseSignatureHeader(header: string): Record<string, string> {
  return Object.fromEntries(
    header
      .split(",")
      .filter(Boolean)
      .map((part, index) => [`v${index}`, part.trim()]),
  );
}

function encodePair(left: string, right: string): string {
  return `${left}:${right}`;
}

function unresolved(value: MissingThing): string {
  return String(value);
}

function _privateToken(): string {
  return "token";
}

class Reader {
  read(headers: Headers): string {
    return headers.get("authorization") ?? "";
  }
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");

    let status_for = |name: &str| {
        coverage
            .iter()
            .find(|entry| entry.get("function").and_then(|value| value.as_str()) == Some(name))
            .and_then(|entry| entry.get("status"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
    };

    assert_eq!(status_for("verifyRequest"), "fuzzed");
    assert_eq!(status_for("parseSignatureHeader"), "fuzzed");
    assert_eq!(status_for("encodePair"), "skipped_internal_helper");
    assert_eq!(status_for("unresolved"), "skipped_unsupported_type");
    assert_eq!(status_for("_privateToken"), "skipped_private_name");
    assert_eq!(status_for("read"), "skipped_method");
}

#[tokio::test]
async fn zero_arg_object_getter_is_classified_as_no_fuzzable_surface() {
    let code = r#"
export function ensureScraper(): { enabled: boolean } {
  return process.env.SCRAPER_TOKEN ? { enabled: true } : { enabled: false };
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(
        !report.stages.iter().any(|stage| stage.name == "execute"),
        "no-fuzzable-surface function should not synthesize execute work"
    );

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let ensure_scraper = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("ensureScraper")
        })
        .expect("ensureScraper coverage should be present");
    assert_eq!(
        ensure_scraper
            .get("status")
            .and_then(|value| value.as_str()),
        Some("skipped_no_fuzzable_surface")
    );
}

#[tokio::test]
async fn zero_arg_primitive_helper_can_still_be_fuzzed() {
    let code = r#"
export function buildVersion(): number {
  return 42;
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let coverage = report
        .stages
        .iter()
        .find(|stage| stage.name == "coverage")
        .and_then(|stage| stage.detail.as_ref())
        .and_then(|detail| detail.get("functions"))
        .and_then(|value| value.as_array())
        .expect("coverage stage should contain per-function entries");
    let build_version = coverage
        .iter()
        .find(|entry| {
            entry.get("function").and_then(|value| value.as_str()) == Some("buildVersion")
        })
        .expect("buildVersion coverage should be present");
    assert_eq!(
        build_version.get("status").and_then(|value| value.as_str()),
        Some("fuzzed")
    );
}

#[tokio::test]
async fn crash_can_be_classified_as_type_signature_wider_than_usage() {
    let code = r#"
export function jsonResponse(status: number): string {
  return new Response("ok", { status }).statusText;
}

jsonResponse(200);
jsonResponse(201);
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    let failures = execute_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("fuzz_failures"))
        .and_then(|value| value.as_array())
        .expect("fuzz failures should be present");
    assert!(
        failures.iter().any(|failure| {
            failure
                .get("classification")
                .and_then(|value| value.as_str())
                == Some("type_signature_wider_than_usage")
        }),
        "expected a wide-type classification in: {failures:#?}"
    );
    let classified = failures
        .iter()
        .find(|failure| {
            failure
                .get("classification")
                .and_then(|value| value.as_str())
                == Some("type_signature_wider_than_usage")
        })
        .unwrap();
    assert!(
        classified
            .get("suggestion")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .contains("200, 201"),
        "expected observed literal suggestion, got: {classified:#?}"
    );
}

#[tokio::test]
async fn verify_separates_portability_warning_from_execute_success() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bun.lock"), "").unwrap();
    std::fs::write(dir.path().join("helper.ts"), "export const value = 7;\n").unwrap();

    let source_path = dir.path().join("main.ts");
    let code = r#"
import { value } from "./helper";

export function add(input: number): number {
  return input + value;
}
"#;
    std::fs::write(&source_path, code).unwrap();

    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);

    let portability_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "portability")
        .expect("portability stage should be present");
    assert!(
        !portability_stage.ok,
        "portability stage should record the Node warning"
    );
    let portability_detail = portability_stage
        .detail
        .as_ref()
        .expect("portability stage should include details");
    assert_eq!(
        portability_detail["reason"].as_str(),
        Some("err_module_not_found")
    );
    assert!(
        portability_detail["failing_imports"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("helper")),
        "expected failing import list to include helper"
    );
    assert!(
        portability_detail["fix_hint"]
            .as_str()
            .unwrap_or("")
            .contains("explicit Node ESM file extensions"),
        "expected a Node ESM fix hint"
    );
    let node_stderr = portability_detail["node_result"]["stderr"]
        .as_str()
        .unwrap_or("");
    assert!(
        node_stderr.contains("ERR_MODULE_NOT_FOUND"),
        "expected Node module resolution warning, got: {node_stderr}"
    );

    let execute_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .expect("execute stage should be present");
    assert!(
        execute_stage.ok,
        "execute stage should succeed: {:?}",
        execute_stage.error
    );
    let runtime = execute_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("runtime"))
        .and_then(|value| value.as_str());
    assert_eq!(runtime, Some("bun"));
}

#[tokio::test]
async fn auto_seed_uses_nearby_test_literals_for_execute_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("host_label.ts");
    let test_path = tests_dir.join("host_label.test.ts");
    let code = r#"
export function hostLabel(url: string): string {
  if (!url.startsWith("https://")) {
    throw new Error("invalid base url");
  }
  return new URL(url).host;
}
"#;
    let test_code = r#"
import { hostLabel } from "../src/host_label.ts";

hostLabel("https://example.com");
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, test_code).unwrap();

    let report_seeded = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;
    let seeded_execute = report_seeded
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert_eq!(seeded_execute["no_inputs_reached"].as_u64(), Some(0));
    assert!(
        seeded_execute["seed_input_count"].as_u64().unwrap_or(0) > 0,
        "expected seeded inputs in execute detail"
    );
    assert!(
        seeded_execute["seed_sources"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|value| value.as_str() == Some(test_path.to_string_lossy().as_ref())),
        "expected nearby test path in seed sources"
    );

    let report_unseeded = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: None,
            test_source_file: None,
            test_runner: TestRunner::Auto,
            tests_only: false,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: false,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;
    let unseeded_execute = report_unseeded
        .stages
        .iter()
        .find(|stage| stage.name == "execute")
        .and_then(|stage| stage.detail.as_ref())
        .expect("execute stage should be present");
    assert_eq!(unseeded_execute["no_inputs_reached"].as_u64(), Some(1));
    assert_eq!(unseeded_execute["seed_input_count"].as_u64(), Some(0));
}

#[tokio::test]
async fn fuzz_failures_truncate_large_inputs_and_messages() {
    let code =
        "def explode(name: str) -> str:\n    if len(name) < 1000:\n        return name\n    raise TypeError('x' * 500)";
    let report = verify(code, &Language::Python, default_opts(None)).await;

    let exec_stage = report
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stage should be present");
    let failures = exec_stage
        .detail
        .as_ref()
        .and_then(|detail| detail.get("fuzz_failures"))
        .and_then(|value| value.as_array())
        .expect("fuzz failures should be present");
    let first = failures
        .first()
        .expect("expected at least one fuzz failure");

    let input = first
        .get("input")
        .and_then(|value| value.as_str())
        .expect("failure input should be present");
    let message = first
        .get("message")
        .and_then(|value| value.as_str())
        .expect("failure message should be present");

    assert!(
        input.len() <= 270 && input.contains("[truncated "),
        "expected truncated input, got: {input}"
    );
    assert!(
        message.len() <= 270 && message.contains("[truncated "),
        "expected truncated message, got: {message}"
    );
}

#[tokio::test]
async fn typescript_test_stage_can_import_source_module_from_test_file() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("handle.ts");
    let test_path = tests_dir.join("court_jester_public_verify.ts");
    let code = r#"
export function displayHandle(user?: { profile?: { handle?: string | null } | null, username?: string | null } | null): string {
  const handle = user?.profile?.handle?.trim();
  if (handle) return handle.toLowerCase();
  const username = user?.username?.trim();
  if (username) return username.toLowerCase();
  return "guest";
}
"#;
    let tests = r#"
import assert from "node:assert/strict";
import { displayHandle } from "../src/handle.ts";

assert.equal(displayHandle({ profile: { handle: " Admin " }, username: "root" }), "admin");
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn typescript_test_stage_auto_prefers_bun_for_bun_test_imports() {
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = dir.path().join("node_modules").join(".bin");
    let bun_log = dir.path().join("bun.log");
    let node_log = dir.path().join("node.log");
    install_fake_tool_at(
        &tool_dir,
        "bun",
        &format!(
            "#!/bin/sh\nprintf 'runner=bun\\n' > \"{}\"\nfor arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\" >> \"{}\"; done\nexit 0\n",
            bun_log.display(),
            bun_log.display(),
        ),
    );
    install_fake_tool_at(
        &tool_dir,
        "node",
        &format!(
            "#!/bin/sh\nprintf 'runner=node\\n' > \"{}\"\nexit 1\n",
            node_log.display(),
        ),
    );
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("math.ts");
    let test_path = tests_dir.join("unit.test.ts");
    let code = "export function add(a: number, b: number): number { return a + b; }\n";
    let tests = r#"
import { test, expect } from "bun:test";
import { add } from "../src/math.ts";

test("add", () => {
  expect(add(2, 3)).toBe(5);
});
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let report = verify(
        code,
        &Language::TypeScript,
        VerifyOptions {
            test_code: Some(tests),
            test_source_file: Some(test_path.to_str().unwrap()),
            test_runner: TestRunner::Auto,
            tests_only: true,
            complexity_threshold: None,
            complexity_metric: ComplexityMetric::Cyclomatic,
            project_dir: Some(dir.path().to_str().unwrap()),
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            suppressions: None,
            suppression_source: None,
            auto_seed: true,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
            report_level: ReportLevel::Full,
            execute_gate: ExecuteGate::All,
        },
    )
    .await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    let test_stage = report
        .stages
        .iter()
        .find(|stage| stage.name == "test")
        .expect("test stage should be present");
    assert!(
        test_stage.ok,
        "test stage should pass: {:?}",
        test_stage.error
    );
    let detail = test_stage.detail.as_ref().unwrap();
    assert_eq!(detail["test_runner_requested"].as_str(), Some("auto"));
    assert_eq!(detail["test_runner_selected"].as_str(), Some("bun"));

    let bun_log_text = std::fs::read_to_string(&bun_log).unwrap();
    assert!(
        bun_log_text.contains("runner=bun"),
        "expected bun runner log, got: {bun_log_text}"
    );
    assert!(
        bun_log_text.contains("arg=test"),
        "expected bun test subcommand, got: {bun_log_text}"
    );
    assert!(
        !node_log.exists(),
        "node should not have been invoked for bun:test authoritative tests"
    );
}

#[tokio::test]
async fn python_test_stage_executes_original_test_file_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let source_path = src_dir.join("app.py");
    let test_path = tests_dir.join("test_app.py");
    let code = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    let tests = r#"
from pathlib import Path

from src.app import add

assert add(2, 3) == 5
assert Path(__file__).name == "test_app.py"
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        test_runner: TestRunner::Auto,
        tests_only: true,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn python_relative_import_test_stage_executes_original_module_when_code_matches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let pkg_dir = dir.path().join("mypkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("__init__.py"), "").unwrap();

    let source_path = pkg_dir.join("app.py");
    let test_path = pkg_dir.join("test_app.py");
    let code = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    let tests = r#"
from pathlib import Path

from .app import add

assert add(2, 3) == 5
assert Path(__file__).name == "test_app.py"
"#;
    std::fs::write(&source_path, code).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        test_runner: TestRunner::Auto,
        tests_only: true,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(!report.stages.iter().any(|s| s.name == "execute"));
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn typescript_normalize_helper_can_return_blank_when_api_handles_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("src");
    let tests_dir = dir.path().join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    let normalizers_path = src_dir.join("normalizers.ts");
    let plans_path = src_dir.join("plans.ts");
    let test_path = tests_dir.join("court_jester_public_verify.ts");
    let normalizers = r#"
export function normalizePlanCode(value: string | null | undefined): string {
  if (typeof value !== "string") {
    return "";
  }
  return value.trim().toUpperCase();
}
"#;
    let plans = r#"
import { normalizePlanCode } from "./normalizers.ts";

export type Account = {
  plans?: Array<string | null> | null;
} | null;

export function primaryPlanCode(account: Account): string {
  const plans = account?.plans;
  if (plans) {
    for (const p of plans) {
      const code = normalizePlanCode(p);
      if (code) return code;
    }
  }
  return "FREE";
}
"#;
    let tests = r#"
import assert from "node:assert/strict";
import { primaryPlanCode } from "../src/plans.ts";

assert.equal(primaryPlanCode({ plans: ["   ", "pro"] }), "PRO");
assert.equal(primaryPlanCode({ plans: [null, ""] }), "FREE");
"#;
    std::fs::write(&normalizers_path, normalizers).unwrap();
    std::fs::write(&plans_path, plans).unwrap();
    std::fs::write(&test_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(test_path.to_str().unwrap()),
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(normalizers_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(normalizers, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "execute" && s.ok));
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}

#[tokio::test]
async fn typescript_test_file_without_imports_uses_source_file_scope() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("initials.ts");
    let tests_path = dir
        .path()
        .join("tests")
        .join("court_jester_public_verify.ts");
    std::fs::create_dir_all(tests_path.parent().unwrap()).unwrap();

    let code = r#"
export function displayInitials(name: string | null): string {
  const parts = name?.trim().split(/\s+/).filter(Boolean) ?? [];
  const initials = parts.map((part) => part[0]?.toUpperCase() ?? "").join("");
  return initials || "NA";
}
"#;
    let tests = r#"
if (displayInitials("Spencer Lee") !== "SL") {
  throw new Error("expected SL");
}
"#;
    std::fs::write(&src_path, code).unwrap();
    std::fs::write(&tests_path, tests).unwrap();

    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: Some(tests_path.to_str().unwrap()),
        test_runner: TestRunner::Auto,
        tests_only: false,
        complexity_threshold: None,
        complexity_metric: ComplexityMetric::Cyclomatic,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        suppressions: None,
        suppression_source: None,
        auto_seed: true,
        source_file: Some(src_path.to_str().unwrap()),
        output_dir: None,
        report_level: ReportLevel::Full,
        execute_gate: ExecuteGate::All,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}
