use court_jester::tools::synthesize::synthesize_calls;
use court_jester::types::*;
use std::collections::BTreeMap;

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
                optional: false,
                variadic: None,
            }],
            return_type: Some("number".to_string()),
            type_parameters: vec![],
            type_parameter_constraints: BTreeMap::new(),
            line: 1,
            end_line: 1,
            complexity: 1,
            cognitive_complexity: 0,
            max_nesting_depth: 0,
            complexity_breakdown: BTreeMap::new(),
            is_method: false,
            is_nested: false,
            is_exported: true,
            declared_properties: vec![],
            predicate_seeds: vec![],
            effects: vec![],
            invocation_target: None,
            returned_callables: vec![],
        }],
        classes: vec![],
        aliases: vec![],
        imports: vec![],
        complexity: 1,
        cognitive_complexity: 0,
        max_nesting_depth: 0,
        source_mode: SourceMode::TypeScript,
        parse_diagnostics: vec![],
        complexity_breakdown: BTreeMap::new(),
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
