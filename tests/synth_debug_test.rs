use court_jester_mcp::tools::synthesize::synthesize_calls;
use court_jester_mcp::types::*;

#[test]
fn debug_print_synthesized_code() {
    let analysis = AnalysisResult {
        functions: vec![FunctionInfo {
            name: "count_chars".to_string(),
            params: vec![ParamInfo {
                name: "s".to_string(),
                type_annotation: Some("string".to_string()),
                default_value: None,
                keyword_only: false,
            }],
            return_type: Some("number".to_string()),
            line: 1,
            end_line: 1,
            complexity: 1,
            is_method: false,
            is_nested: false,
            is_exported: true,
        }],
        classes: vec![],
        aliases: vec![],
        imports: vec![],
        complexity: 1,
        parse_error: false,
    };
    let code = synthesize_calls(&analysis, &Language::TypeScript);
    println!("=== SYNTHESIZED CODE ===");
    println!("{}", code);
    println!("=== END ===");

    // Check that paramTypes is present
    assert!(
        code.contains("[\"string\"]"),
        "Should have paramTypes [\"string\"], got:\n{code}"
    );
    assert!(
        code.contains("\"nonneg\""),
        "Should have nonneg property, got:\n{code}"
    );
}
