use court_jester_mcp::resolve_code;
use std::io::Write;

#[test]
fn resolve_code_inline() {
    let result = resolve_code("print('hello')", None);
    assert_eq!(result.unwrap(), "print('hello')");
}

#[test]
fn resolve_code_file() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "def greet(): pass").unwrap();
    let result = resolve_code("", Some(tmp.path().to_str().unwrap()));
    assert_eq!(result.unwrap(), "def greet(): pass");
}

#[test]
fn resolve_code_both_errors() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "x").unwrap();
    let result = resolve_code("some code", Some(tmp.path().to_str().unwrap()));
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("not both"),
        "should say not both"
    );
}

#[test]
fn resolve_code_neither_errors() {
    let result = resolve_code("", None);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Must provide"),
        "should say must provide"
    );
}

#[test]
fn resolve_code_bad_path() {
    let result = resolve_code("", Some("/nonexistent/path/to/file.py"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Cannot read"),
        "should say cannot read, got: {err}"
    );
    assert!(
        err.contains("/nonexistent/path/to/file.py"),
        "should include path, got: {err}"
    );
}
