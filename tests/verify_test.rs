use court_jester_mcp::tools::verify::{parse_fuzz_failures, verify, VerifyOptions};
use court_jester_mcp::types::Language;

fn default_opts(test_code: Option<&str>) -> VerifyOptions<'_> {
    VerifyOptions {
        test_code,
        test_source_file: None,
        complexity_threshold: None,
        project_dir: None,
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
async fn with_failing_tests() {
    let code = "def double(x: int) -> int:\n    return x * 3"; // bug: *3 instead of *2
    let tests = "assert double(5) == 10";
    let report = verify(code, &Language::Python, default_opts(Some(tests))).await;

    assert!(!report.overall_ok);
    assert!(report.stages.iter().any(|s| s.name == "test" && !s.ok));
}

#[tokio::test]
async fn lint_warnings_are_informational() {
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
async fn python_test_stage_can_import_source_module_from_sibling_path() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("billing.py");
    let code = "def billing_country(order: dict | None) -> str:\n    return \"US\"";
    std::fs::write(&source_path, code).unwrap();

    let tests = "from billing import billing_country\nassert billing_country(None) == \"US\"";
    let opts = VerifyOptions {
        test_code: Some(tests),
        test_source_file: None,
        complexity_threshold: None,
        project_dir: None,
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
        complexity_threshold: Some(3),
        project_dir: None,
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
        complexity_threshold: None,
        project_dir: None,
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
        complexity_threshold: None,
        project_dir: None,
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
        complexity_threshold: None,
        project_dir: None,
        diff: None,
        source_file: Some(source_path.to_str().unwrap()),
        output_dir: None,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
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
        complexity_threshold: None,
        project_dir: None,
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
    std::fs::create_dir_all(&tests_path.parent().unwrap()).unwrap();

    let code = r#"
export function displayInitials(name: string | null): string {
  return name
    ?.trim()
    .split(/\s+/)
    .map((part) => part[0]!.toUpperCase())
    .join("")
    .slice(0, 2);
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
        complexity_threshold: None,
        project_dir: None,
        diff: None,
        source_file: Some(src_path.to_str().unwrap()),
        output_dir: None,
    };
    let report = verify(code, &Language::TypeScript, opts).await;

    assert!(report.overall_ok, "report: {:#?}", report.stages);
    assert!(report.stages.iter().any(|s| s.name == "test" && s.ok));
}
