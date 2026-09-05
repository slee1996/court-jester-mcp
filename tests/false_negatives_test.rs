//! Detection tests: preserve exceptions as observations and gate admitted failures.

use court_jester::tools::verify::{verify, VerifyOptions};
use court_jester::types::{
    ComplexityMetric, CoverageGate, ExecuteGate, InferredOracleGate, Language, NetworkPolicy,
    ReportLevel, RuntimeProfile, StageStatus, TestRunner, VerificationReport,
    DEFAULT_PYTHON_DOCKER_IMAGE, DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
};

fn opts() -> VerifyOptions<'static> {
    VerifyOptions {
        test_code: None,
        test_source_file: None,
        base_code: None,
        base_source_file: None,
        base_project_dir: None,
        test_runner: TestRunner::Auto,
        tests_only: false,
        test_quality_max_mutants: None,
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
        coverage_gate: CoverageGate::ChangedExports,
        inferred_oracle_gate: InferredOracleGate::Fail,
        runtime_profile: RuntimeProfile::LocalTrusted,
        python_docker_image: DEFAULT_PYTHON_DOCKER_IMAGE,
        typescript_docker_image: DEFAULT_TYPESCRIPT_DOCKER_IMAGE,
        memory_mb: 512,
        network: NetworkPolicy::Deny,
        harness_args: vec![],
    }
}

async fn fixture_report(code: &str, language: &Language) -> VerificationReport {
    let project = tempfile::tempdir().unwrap();
    let extension = match language {
        Language::Python => "py",
        Language::TypeScript => "ts",
    };
    let source = project.path().join(format!("target.{extension}"));
    std::fs::write(&source, code).unwrap();
    let mut options = opts();
    options.project_dir = Some(project.path().to_str().unwrap());
    options.source_file = Some(source.to_str().unwrap());
    verify(code, language, options).await
}

async fn fuzz_catches_bug(code: &str, language: &Language) -> bool {
    let report = fixture_report(code, language).await;
    let exec_stage = report.stages.iter().find(|s| s.name == "execute");
    match exec_stage {
        Some(stage) => stage.status == StageStatus::Failed,
        None => false,
    }
}

async fn exception_requires_admission(
    code: &str,
    admitted: &str,
    language: &Language,
    error_type: &str,
) {
    for (source, expected_status, classification) in [
        (code, StageStatus::Inconclusive, "unknown"),
        (admitted, StageStatus::Failed, "valid"),
    ] {
        let report = fixture_report(source, language).await;
        let stage = report
            .stages
            .iter()
            .find(|stage| stage.name == "execute")
            .unwrap();
        assert_eq!(stage.status, expected_status, "{error_type}: {report:#?}");
        let findings = stage.detail.as_ref().unwrap()["findings"]
            .as_array()
            .unwrap();
        assert!(
            findings
                .iter()
                .any(|finding| finding["error_type"] == error_type
                    && finding["input_classification"] == classification),
            "{error_type}: {findings:?}"
        );
        if classification == "unknown" {
            assert_eq!(report.summary.findings.gating, 0);
        }
    }
}

// ── Python false negatives ──────────────────────────────────────────────────

#[tokio::test]
async fn catches_empty_string_crash() {
    let code = r#"
def first_char(s: str) -> str:
    return s[0]
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("s: str", "s: Literal['', 'a']")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "IndexError").await;
}

#[tokio::test]
async fn catches_division_by_zero() {
    let code = r#"
def inverse(x: int) -> float:
    return 1 / x
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("x: int", "x: Literal[0, 1]")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "ZeroDivisionError").await;
}

#[tokio::test]
async fn catches_none_attribute_access() {
    let code = r#"
def get_length(s: str) -> int:
    if s == "":
        s = None
    return len(s)
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("s: str", "s: Literal['', 'a']")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "TypeError").await;
}

#[tokio::test]
async fn catches_index_out_of_bounds() {
    let code = r#"
def last_char(s: str) -> str:
    return s[len(s)]
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("s: str", "s: Literal['', 'a']")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "IndexError").await;
}

#[tokio::test]
async fn catches_type_error_on_none_arithmetic() {
    let code = r#"
def double_or_none(x: int) -> int:
    if x == 0:
        return None
    return x * 2

def use_result(x: int) -> int:
    return double_or_none(x) + 1
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch TypeError: None + 1"
    );
}

#[tokio::test]
async fn catches_key_error() {
    let code = r#"
def get_value(key: str) -> str:
    d = {"hello": "world", "foo": "bar"}
    return d[key]
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("key: str", "key: Literal['hello', 'missing']")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "KeyError").await;
}

#[tokio::test]
async fn catches_recursion_error() {
    let code = r#"
def factorial(n: int) -> int:
    if n == 0:
        return 1
    return n * factorial(n - 1)
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("n: int", "n: Literal[-1, 0]")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "RecursionError").await;
}

#[tokio::test]
async fn catches_unicode_encode_error() {
    let code = r#"
def to_ascii(s: str) -> bytes:
    return s.encode("ascii")
"#;
    let admitted = format!(
        "from typing import Literal\n{}",
        code.replace("s: str", "s: Literal['a', 'é']")
    );
    exception_requires_admission(code, &admitted, &Language::Python, "UnicodeEncodeError").await;
}

// ── Python: verify robust functions DON'T false-positive ────────────────────

#[tokio::test]
async fn no_false_positive_on_safe_add() {
    let code = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    assert!(
        !fuzz_catches_bug(code, &Language::Python).await,
        "safe add should NOT be flagged"
    );
}

#[tokio::test]
async fn no_false_positive_on_safe_string_fn() {
    let code = r#"
def greet(name: str) -> str:
    return f"hello {name}"
"#;
    assert!(
        !fuzz_catches_bug(code, &Language::Python).await,
        "safe greeting should NOT be flagged"
    );
}

#[tokio::test]
async fn no_false_positive_for_unavailable_cursor_collaborator() {
    let code = r#"
def require_collection_persistence(cursor, school_id):
    cursor.execute("SELECT 1", (school_id,))
"#;
    assert!(
        !fuzz_catches_bug(code, &Language::Python).await,
        "generated scalar cursor substitutes are invalid collaborator inputs"
    );
}

// ── TypeScript false negatives ──────────────────────────────────────────────

#[tokio::test]
async fn ts_catches_undefined_property_access() {
    let code = r#"
function getLength(s: string | null): number {
    return s!.length;
}
"#;
    let admitted = code.replace("s: string | null", "s: 'a' | null");
    exception_requires_admission(code, &admitted, &Language::TypeScript, "TypeError").await;
}

#[tokio::test]
async fn ts_catches_array_index_oob() {
    let code = r#"
function lastElement(arr: number[]): number {
    return arr[arr.length];
}
"#;
    assert!(
        fuzz_catches_bug(code, &Language::TypeScript).await,
        "should catch undefined from out-of-bounds access"
    );
}

#[tokio::test]
async fn ts_no_false_positive_on_safe_add() {
    let code = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;
    assert!(
        !fuzz_catches_bug(code, &Language::TypeScript).await,
        "safe add should NOT be flagged"
    );
}
