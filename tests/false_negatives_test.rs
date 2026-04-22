//! False negative tests: verify the fuzzer catches known bugs.
//! Each test writes a buggy function, runs verify, and asserts the fuzz stage fails.

use court_jester_mcp::tools::verify::{verify, VerifyOptions};
use court_jester_mcp::types::{ComplexityMetric, ExecuteGate, Language, ReportLevel};

fn opts() -> VerifyOptions<'static> {
    VerifyOptions {
        test_code: None,
        test_source_file: None,
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

/// Helper: run verify, return whether the execute stage reported crashes.
async fn fuzz_catches_bug(code: &str, language: &Language) -> bool {
    let report = verify(code, language, opts()).await;
    let exec_stage = report.stages.iter().find(|s| s.name == "execute");
    match exec_stage {
        Some(stage) => !stage.ok,
        None => false, // no execute stage = no functions found
    }
}

// ── Python false negatives ──────────────────────────────────────────────────

#[tokio::test]
async fn catches_empty_string_crash() {
    let code = r#"
def first_char(s: str) -> str:
    return s[0]
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch IndexError on empty string"
    );
}

#[tokio::test]
async fn catches_division_by_zero() {
    let code = r#"
def inverse(x: int) -> float:
    return 1 / x
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch ZeroDivisionError"
    );
}

#[tokio::test]
async fn catches_none_attribute_access() {
    let code = r#"
def get_length(s: str) -> int:
    if s == "":
        s = None
    return len(s)
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch TypeError on None.len()"
    );
}

#[tokio::test]
async fn catches_index_out_of_bounds() {
    let code = r#"
def last_char(s: str) -> str:
    return s[len(s)]
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch IndexError (off-by-one)"
    );
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
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch KeyError on random string"
    );
}

#[tokio::test]
async fn catches_recursion_error() {
    let code = r#"
def factorial(n: int) -> int:
    if n == 0:
        return 1
    return n * factorial(n - 1)
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch RecursionError on negative input"
    );
}

#[tokio::test]
async fn catches_unicode_encode_error() {
    let code = r#"
def to_ascii(s: str) -> bytes:
    return s.encode("ascii")
"#;
    assert!(
        fuzz_catches_bug(code, &Language::Python).await,
        "should catch UnicodeEncodeError on non-ASCII input"
    );
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

// ── TypeScript false negatives ──────────────────────────────────────────────

#[tokio::test]
async fn ts_catches_undefined_property_access() {
    let code = r#"
function getLength(s: string | null): number {
    return s!.length;
}
"#;
    assert!(
        fuzz_catches_bug(code, &Language::TypeScript).await,
        "should catch Cannot read properties of null"
    );
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
