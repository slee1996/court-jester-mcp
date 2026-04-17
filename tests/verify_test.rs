use court_jester_mcp::tools::verify::{parse_fuzz_failures, verify, VerifyOptions};
use court_jester_mcp::types::Language;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::Mutex;

fn path_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PathGuard {
    old_path: String,
}

impl PathGuard {
    fn install(prefix: &std::path::Path) -> Self {
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", prefix.display(), old_path));
        Self { old_path }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        std::env::set_var("PATH", &self.old_path);
    }
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn install_fake_tool(name: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    install_fake_tool_at(dir.path(), name, body);
    dir
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: None,
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
        tests_only: true,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: None,
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
        tests_only: true,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: None,
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
    let _guard = path_lock().lock().await;
    let tool_dir = install_fake_tool(
        "biome",
        "#!/bin/sh\ncat <<'EOF'\n{\"diagnostics\":[{\"category\":\"lint/style/noNonNullAssertion\",\"description\":\"Avoid non-null assertions.\",\"severity\":\"warning\",\"location\":{\"start\":{\"line\":3,\"column\":12}}}]}\nEOF\nexit 1\n",
    );
    let _path = PathGuard::install(tool_dir.path());

    let code = r#"
function normalizeName(name: string): string {
    return name!.trim();
}
"#;
    let report = verify(code, &Language::TypeScript, default_opts(None)).await;

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
            tests_only: false,
            complexity_threshold: None,
            project_dir: Some(project_dir.path().to_str().unwrap()),
            lint_config_path: Some(config_path.to_str().unwrap()),
            lint_virtual_file_path: None,
            diff: None,
            source_file: Some(source_path.to_str().unwrap()),
            output_dir: None,
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
    let _guard = path_lock().lock().await;
    let tool_dir = install_fake_tool(
        "ruff",
        "#!/bin/sh\ncat <<'EOF'\n[{\"code\":\"F841\",\"message\":\"assigned but unused\",\"location\":{\"row\":1,\"column\":1}}]\nEOF\nexit 1\n",
    );
    let _path = PathGuard::install(tool_dir.path());

    let code = "def add(a: int, b: int) -> int:\n    return a + b\n";
    let report = verify(code, &Language::Python, default_opts(None)).await;

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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
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
        tests_only: false,
        complexity_threshold: Some(3),
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: None,
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
            tests_only: false,
            complexity_threshold: Some(3),
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: Some(diff),
            source_file: None,
            output_dir: None,
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
            tests_only: false,
            complexity_threshold: Some(2),
            project_dir: None,
            lint_config_path: None,
            lint_virtual_file_path: None,
            diff: None,
            source_file: None,
            output_dir: None,
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: Some(diff),
        source_file: None,
        output_dir: None,
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
}

#[tokio::test]
async fn writes_report_to_output_dir() {
    let dir = tempfile::tempdir().unwrap();
    let code = "def add(a: int, b: int) -> int:\n    return a + b";
    let opts = VerifyOptions {
        test_code: None,
        test_source_file: None,
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
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
    assert!(parsed.get("meta").is_some());
    assert!(parsed.get("summary").is_some());
    assert!(parsed.get("stages").is_some());
    assert!(parsed.get("overall_ok").is_some());
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: None,
        output_dir: Some(dir.path().to_str().unwrap()),
    };
    let report = verify(code, &Language::Python, opts).await;

    assert!(
        !report.overall_ok,
        "rejected-only fuzz run should fail verify"
    );
    let path = report.report_path.unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let summary = parsed.get("summary").unwrap();
    assert_eq!(summary.get("functions_fuzzed").unwrap().as_u64(), Some(1));
    assert_eq!(summary.get("fuzz_pass").unwrap().as_u64(), Some(0));
    assert_eq!(summary.get("fuzz_crash").unwrap().as_u64(), Some(0));
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
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
        tests_only: true,
        complexity_threshold: None,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
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
        tests_only: true,
        complexity_threshold: None,
        project_dir: Some(dir.path().to_str().unwrap()),
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(normalizers_path.to_str().unwrap()),
        output_dir: None,
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
        tests_only: false,
        complexity_threshold: None,
        project_dir: None,
        lint_config_path: None,
        lint_virtual_file_path: None,
        diff: None,
        source_file: Some(src_path.to_str().unwrap()),
        output_dir: None,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}
