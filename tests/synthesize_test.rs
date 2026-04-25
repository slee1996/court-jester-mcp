use court_jester_mcp::tools::synthesize::{
    synthesize_calls, synthesize_plan, synthesize_plan_for_with_seeds,
};
use court_jester_mcp::types::*;
use std::collections::{BTreeMap, HashMap};

fn make_analysis(functions: Vec<FunctionInfo>, classes: Vec<ClassInfo>) -> AnalysisResult {
    AnalysisResult {
        functions,
        classes,
        aliases: vec![],
        imports: vec![],
        complexity: 1,
        cognitive_complexity: 0,
        max_nesting_depth: 0,
        complexity_breakdown: BTreeMap::new(),
        parse_error: false,
    }
}

fn func(name: &str, params: Vec<(&str, Option<&str>)>, ret: Option<&str>) -> FunctionInfo {
    FunctionInfo {
        name: name.to_string(),
        params: params
            .into_iter()
            .map(|(n, t)| ParamInfo {
                name: n.to_string(),
                type_annotation: t.map(|s| s.to_string()),
                default_value: None,
                keyword_only: false,
            })
            .collect(),
        return_type: ret.map(|s| s.to_string()),
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
        invocation_target: None,
        returned_callables: vec![],
    }
}

fn kwonly_func(
    name: &str,
    positional: Vec<(&str, Option<&str>)>,
    keyword: Vec<(&str, Option<&str>)>,
    ret: Option<&str>,
) -> FunctionInfo {
    let mut params: Vec<ParamInfo> = positional
        .into_iter()
        .map(|(n, t)| ParamInfo {
            name: n.to_string(),
            type_annotation: t.map(|s| s.to_string()),
            default_value: None,
            keyword_only: false,
        })
        .collect();
    params.extend(keyword.into_iter().map(|(n, t)| ParamInfo {
        name: n.to_string(),
        type_annotation: t.map(|s| s.to_string()),
        default_value: None,
        keyword_only: true,
    }));
    FunctionInfo {
        name: name.to_string(),
        params,
        return_type: ret.map(|s| s.to_string()),
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
        invocation_target: None,
        returned_callables: vec![],
    }
}

// ── Python fuzz harness generation ──────────────────────────────────────────

#[test]
fn python_generates_fuzz_harness() {
    let a = make_analysis(
        vec![func("greet", vec![("name", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("_fuzz_str()"),
        "should use fuzz generators, got: {code}"
    );
    assert!(
        code.contains("FUZZ greet"),
        "should have fuzz label, got: {code}"
    );
    assert!(
        code.contains("_fuzz_failures"),
        "should track failures, got: {code}"
    );
    assert!(
        code.contains("_is_crash"),
        "should have crash detection, got: {code}"
    );
    assert!(
        code.contains("_reject"),
        "should track rejections, got: {code}"
    );
}

#[test]
fn python_seed_rows_shape_fuzz_domain() {
    let functions = vec![func(
        "find_in_sorted",
        vec![("arr", None), ("x", None)],
        None,
    )];
    let mut seeds = HashMap::new();
    seeds.insert(
        "find_in_sorted".to_string(),
        vec![vec!["[1, 2, 3]".to_string(), "2".to_string()]],
    );
    let plan = synthesize_plan_for_with_seeds(&functions, &[], &[], &Language::Python, &seeds);
    assert!(
        plan.code.contains("_seed_rows = [[[1, 2, 3], 2]]"),
        "expected embedded Python seed rows, got: {}",
        plan.code
    );
    assert!(
        plan.code.contains("_fuzz_seed_row(_seed_rows)"),
        "expected seed-shaped fuzzing instead of only broad generators, got: {}",
        plan.code
    );
}

#[test]
fn python_literal_params_generate_declared_domain_values() {
    let functions = vec![func(
        "status_label",
        vec![("status", Some("Literal[\"draft\", \"published\"]"))],
        Some("str"),
    )];
    let plan =
        synthesize_plan_for_with_seeds(&functions, &[], &[], &Language::Python, &HashMap::new());
    assert!(
        plan.code
            .contains("[\"draft\", \"published\"][_fuzz_int_range(0, 1)]"),
        "Literal params should generate declared values, got: {}",
        plan.code
    );
}

#[test]
fn python_nested_literal_list_elements_generate_declared_domain_values() {
    let functions = vec![func(
        "count_billable_actions",
        vec![("actions", Some("list[Literal[\"create\", \"delete\"]]"))],
        Some("int"),
    )];
    let plan =
        synthesize_plan_for_with_seeds(&functions, &[], &[], &Language::Python, &HashMap::new());
    assert!(
        plan.code
            .contains("[\"create\", \"delete\"][_fuzz_int_range(0, 1)] for _ in range"),
        "nested Literal element types should shape collection values, got: {}",
        plan.code
    );
}

#[test]
fn python_untyped_params_without_seed_domain_are_skipped() {
    let functions = vec![func("walk_graph", vec![("node", None)], None)];
    let plan =
        synthesize_plan_for_with_seeds(&functions, &[], &[], &Language::Python, &HashMap::new());
    assert!(
        plan.code.is_empty(),
        "untyped params without domain evidence should not get arbitrary fuzz code: {}",
        plan.code
    );
    assert_eq!(
        plan.coverage[0].status,
        FuzzFunctionStatus::SkippedUnsupportedType
    );
}

#[test]
fn python_idempotency_requires_declared_property() {
    let mut clean_text = func("clean_text", vec![("s", Some("str"))], Some("str"));
    clean_text.declared_properties = vec!["idempotent".into()];
    let a = make_analysis(vec![clean_text], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("idempotent"),
        "declared idempotent str→str should check idempotency, got: {code}"
    );
}

#[test]
fn python_no_idempotency_from_name_only() {
    let a = make_analysis(
        vec![func("clean_text", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("idempotent"),
        "name alone should NOT check idempotency, got: {code}"
    );
}

#[test]
fn python_average_order_value_is_not_treated_as_comparator() {
    let a = make_analysis(
        vec![func(
            "average_order_value",
            vec![("total_cents", Some("int")), ("order_count", Some("int"))],
            Some("float"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("Comparator self-compare should be zero"),
        "average_order_value should not get comparator checks, got: {code}"
    );
}

#[test]
fn python_query_string_serializer_gets_semantic_examples() {
    let a = make_analysis(
        vec![func(
            "canonical_query",
            vec![("params", Some("dict[str, object]"))],
            Some("str"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("_parse_qsl"),
        "query-string harness should parse query output, got: {code}"
    );
    assert!(
        code.contains("query semantics:{_query_label}"),
        "query-string harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("_ascii_fold(\"naïve café\")"),
        "query-string harness should check accent folding, got: {code}"
    );
    assert!(
        code.contains("{\"filters\": [{\"label\": \"pro\"}, None, \" beta \"]}"),
        "query-string harness should cover nested non-scalars, got: {code}"
    );
}

#[test]
fn python_pep440_version_ordering_property_gets_semantic_examples() {
    let mut compare_versions = func(
        "compare_versions",
        vec![("left", Some("str")), ("right", Some("str"))],
        Some("int"),
    );
    compare_versions.declared_properties = vec!["pep440_version_ordering".into()];
    let a = make_analysis(vec![compare_versions], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("pep440 version ordering:{_pep440_label}"),
        "PEP 440 ordering harness should label failures, got: {code}"
    );
    assert!(
        code.contains("\"1.0rc1\", \"1.0\", -1"),
        "PEP 440 ordering harness should check rc before final, got: {code}"
    );
}

#[test]
fn python_pep440_specifier_property_gets_semantic_examples() {
    let mut allows = func(
        "allows",
        vec![("version", Some("str")), ("specifier", Some("str"))],
        Some("bool"),
    );
    allows.declared_properties = vec!["pep440_specifier_membership".into()];
    let a = make_analysis(vec![allows], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("pep440 specifier membership:{_specifier_label}"),
        "PEP 440 specifier harness should label failures, got: {code}"
    );
    assert!(
        code.contains("\"1.5.0\", \"~=1.4.5\", False"),
        "PEP 440 specifier harness should check compatible upper bound, got: {code}"
    );
}

#[test]
fn python_pep440_filter_property_gets_semantic_examples() {
    let mut filter_versions = func(
        "filter_versions",
        vec![
            ("candidates", Some("list[str]")),
            ("specifier", Some("str")),
        ],
        Some("list[str]"),
    );
    filter_versions.declared_properties = vec!["pep440_filter_prerelease".into()];
    let a = make_analysis(vec![filter_versions], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("pep440 filter prerelease:{_filter_label}"),
        "PEP 440 filter harness should label failures, got: {code}"
    );
    assert!(
        code.contains("[\"1.2\", \"1.5a1\"], \">=1.5\", [\"1.5a1\"]"),
        "PEP 440 filter harness should check prerelease fallback, got: {code}"
    );
}

#[test]
fn python_cookie_value_quote_property_gets_semantic_examples() {
    let mut format_cookie_value = func(
        "format_cookie_value",
        vec![("value", Some("str"))],
        Some("str"),
    );
    format_cookie_value.declared_properties = vec!["cookie_value_quote".into()];
    let a = make_analysis(vec![format_cookie_value], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("cookie value quote:{_cookie_value_label}"),
        "cookie value harness should label failures, got: {code}"
    );
    assert!(
        code.contains("'\"two words\"', '\"two words\"'"),
        "cookie value harness should preserve already-quoted values, got: {code}"
    );
}

#[test]
fn python_cookie_header_quote_property_gets_semantic_examples() {
    let mut build_cookie_header = func(
        "build_cookie_header",
        vec![("cookies", Some("Mapping[str, str | None]"))],
        Some("str"),
    );
    build_cookie_header.declared_properties = vec!["cookie_header_quote".into()];
    let a = make_analysis(vec![build_cookie_header], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("cookie header quote:{_cookie_header_label}"),
        "cookie header harness should label failures, got: {code}"
    );
    assert!(
        code.contains("'session=\"two words\"'"),
        "cookie header harness should preserve already-quoted values, got: {code}"
    );
}

#[test]
fn python_non_query_serializer_skips_query_semantics() {
    let a = make_analysis(
        vec![func(
            "serialize_profile",
            vec![("profile", Some("dict[str, object]"))],
            Some("str"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("query semantics:{_query_label}"),
        "non-query serializer should not get query semantics, got: {code}"
    );
}

#[test]
fn python_mapping_annotation_generates_mapping_input() {
    let a = make_analysis(
        vec![func(
            "build_cookie_header",
            vec![("cookies", Some("Mapping[str, str | None]"))],
            Some("str"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("{_fuzz_str(): [_fuzz_str(), None][_fuzz_int_range(0, 1)]"),
        "Mapping annotations should generate mapping-shaped inputs, got: {code}"
    );
    assert!(
        !code.contains("_call_args = [None]"),
        "non-optional Mapping annotations should not generate None as the container, got: {code}"
    );
}

#[test]
fn python_no_idempotency_for_different_types() {
    let a = make_analysis(
        vec![func(
            "normalize_text",
            vec![("s", Some("str"))],
            Some("int"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("idempotent"),
        "str→int should NOT check idempotency, got: {code}"
    );
}

#[test]
fn python_keyword_only_in_fuzz() {
    let a = make_analysis(
        vec![kwonly_func(
            "process",
            vec![("text", Some("str"))],
            vec![("mode", Some("str"))],
            Some("str"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("mode=_call_args[1]"),
        "keyword-only should use name=, got: {code}"
    );
    assert!(
        !code.contains("text="),
        "positional should NOT use name=, got: {code}"
    );
}

#[test]
fn python_skips_private() {
    let a = make_analysis(
        vec![
            func("_private", vec![], None),
            func("public_fn", vec![], None),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(!code.contains("_private("), "should skip private");
    assert!(code.contains("public_fn"), "should include public");
}

#[test]
fn python_uses_dataclass_fields() {
    let classes = vec![ClassInfo {
        name: "Point".into(),
        bases: vec![],
        line: 1,
        fields: vec![
            FieldInfo {
                name: "x".into(),
                type_annotation: Some("float".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "y".into(),
                type_annotation: Some("float".into()),
                optional: false,
                has_default: false,
            },
        ],
    }];
    let funcs = vec![func("process", vec![("p", Some("Point"))], None)];
    let a = make_analysis(funcs, classes);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("Point("),
        "should construct Point, got: {code}"
    );
    assert!(
        code.contains("_fuzz_float()"),
        "should use float generators for fields, got: {code}"
    );
}

// ── TypeScript fuzz harness generation ──────────────────────────────────────

#[test]
fn typescript_generates_fuzz_harness() {
    let a = make_analysis(
        vec![func(
            "add",
            vec![("a", Some("number")), ("b", Some("number"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzNum()"),
        "should use fuzz generators, got: {code}"
    );
    assert!(
        code.contains("_fuzzOne("),
        "should use fuzz runner, got: {code}"
    );
    assert!(
        code.contains("\"number\""),
        "should check return type, got: {code}"
    );
}

#[test]
fn typescript_literal_union_params_generate_declared_domain_values() {
    let a = make_analysis(
        vec![func(
            "methodAllowsBody",
            vec![("method", Some("\"GET\" | \"PATCH\""))],
            Some("boolean"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("[\"GET\", \"PATCH\"][_fuzzIntRange(0, 1)]"),
        "literal union params should generate declared values, got: {code}"
    );
}

#[test]
fn typescript_literal_object_fields_generate_declared_domain_values() {
    let a = make_analysis(
        vec![func(
            "queueName",
            vec![(
                "job",
                Some("{ kind: \"email\" | \"digest\"; attempts: number }"),
            )],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("kind: [\"email\", \"digest\"][_fuzzIntRange(0, 1)]"),
        "literal union object fields should generate declared values, got: {code}"
    );
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"queueName\""))
        .expect("queueName fuzz call should exist");
    assert!(
        !fuzz_call.contains("\"object\""),
        "closed literal-domain objects should not receive broad object edge cases, got: {fuzz_call}"
    );
}

#[test]
fn typescript_zero_arg_entropy_helper_skips_consistency_property() {
    let a = make_analysis(
        vec![func("generateCorrelationId", vec![], Some("string"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"generateCorrelationId\""))
        .expect("generateCorrelationId fuzz call should exist");
    assert!(
        !fuzz_call.contains("\"consistent\""),
        "entropy helper should not get consistency checks, got: {fuzz_call}"
    );
}

#[test]
fn typescript_idempotency_for_normalize() {
    let a = make_analysis(
        vec![func(
            "normalize",
            vec![("s", Some("string"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("true"),
        "normalize string→string should enable idempotency, got: {code}"
    );
}

#[test]
fn typescript_no_idempotency_for_non_idempotent_name() {
    let a = make_analysis(
        vec![func("double", vec![("s", Some("string"))], Some("string"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("false"),
        "double should disable idempotency, got: {code}"
    );
}

#[test]
fn typescript_upper_bound_helper_is_not_treated_as_idempotent() {
    let classes = vec![ClassInfo {
        name: "ParsedVersion".into(),
        bases: vec![],
        line: 1,
        fields: vec![
            FieldInfo {
                name: "major".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "minor".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "patch".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "prerelease".into(),
                type_annotation: Some("string[] | null".into()),
                optional: false,
                has_default: false,
            },
        ],
    }];
    let mut helper = func(
        "caretUpperBound",
        vec![("base", Some("ParsedVersion"))],
        Some("ParsedVersion"),
    );
    helper.is_exported = false;
    let a = make_analysis(vec![helper], classes);
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"caretUpperBound\""))
        .unwrap_or("");
    assert!(
        !fuzz_call.contains("\"idempotent\""),
        "caretUpperBound should not get idempotency checks, got: {fuzz_call}"
    );
}

#[test]
fn typescript_non_exported_string_helper_skips_name_cued_properties() {
    let mut helper = func(
        "normalizeHandle",
        vec![("value", Some("string"))],
        Some("string"),
    );
    helper.is_exported = false;
    let a = make_analysis(vec![helper], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"normalizeHandle\""))
        .unwrap_or("");
    assert!(
        !fuzz_call.contains("\"idempotent\"") && !fuzz_call.contains("\"nonempty_string\""),
        "non-exported helper should not get name-cued semantic properties, got: {fuzz_call}"
    );
}

#[test]
fn typescript_non_exported_structured_identifier_helper_gets_nonempty_string() {
    let mut helper = func(
        "primaryCity",
        vec![("user", Some("{ city?: string | null } | null"))],
        Some("string"),
    );
    helper.is_exported = false;
    let a = make_analysis(vec![helper], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"primaryCity\""))
        .unwrap_or("");
    assert!(
        fuzz_call.contains("\"nonempty_string\""),
        "structured identifier helper should get nonempty_string, got: {fuzz_call}"
    );
}

#[test]
fn typescript_exported_compare_gets_comparator_property() {
    let a = make_analysis(
        vec![func(
            "compareVersions",
            vec![
                ("left", Some("ParsedVersion")),
                ("right", Some("ParsedVersion")),
            ],
            Some("number"),
        )],
        vec![ClassInfo {
            name: "ParsedVersion".into(),
            bases: vec![],
            line: 1,
            fields: vec![],
        }],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"compareVersions\""))
        .unwrap_or("");
    assert!(
        fuzz_call.contains("\"comparator\""),
        "compareVersions should get comparator contract checks, got: {fuzz_call}"
    );
}

#[test]
fn typescript_uses_interface_fields() {
    let classes = vec![ClassInfo {
        name: "User".into(),
        bases: vec![],
        line: 1,
        fields: vec![
            FieldInfo {
                name: "id".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "name".into(),
                type_annotation: Some("string".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "email".into(),
                type_annotation: Some("string".into()),
                optional: true,
                has_default: false,
            },
        ],
    }];
    let funcs = vec![func("process", vec![("u", Some("User"))], None)];
    let a = make_analysis(funcs, classes);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("id: _fuzzNum()"),
        "should generate id field, got: {code}"
    );
    assert!(
        code.contains("name: _fuzzStr()"),
        "should generate name field, got: {code}"
    );
    assert!(
        code.contains("email: _fuzzBool()"),
        "optional should use random null, got: {code}"
    );
}

#[test]
fn typescript_query_string_serializer_gets_semantic_examples() {
    let mut stringify = func(
        "stringifyQuery",
        vec![("params", Some("Record<string, unknown>"))],
        Some("string"),
    );
    stringify.declared_properties = vec!["query_nested_brackets".into()];
    let a = make_analysis(vec![stringify], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("new URLSearchParams"),
        "query-string harness should decode query output, got: {code}"
    );
    assert!(
        code.contains("query semantics:${_queryLabel}"),
        "query-string harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("top-level repeated array"),
        "query-string harness should check repeated array encoding, got: {code}"
    );
    assert!(
        code.contains("filter[tags][]"),
        "query-string harness should check nested array bracket encoding, got: {code}"
    );
    assert!(
        !code.contains("_asciiFold(\"naïve café\")"),
        "qs-style stringification should not inherit canonical accent folding, got: {code}"
    );
}

#[test]
fn typescript_query_string_serializer_does_not_assume_deep_brackets_without_context() {
    let a = make_analysis(
        vec![func(
            "canonicalQuery",
            vec![("params", Some("Record<string, unknown>"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        !code.contains("filter[tags][]"),
        "query-string harness should not infer nested bracket semantics without context, got: {code}"
    );
    assert!(
        code.contains("_asciiFold(\"naïve café\")"),
        "canonical query harness should keep canonical accent folding, got: {code}"
    );
}

#[test]
fn typescript_query_string_parser_gets_semantic_examples() {
    let mut parse = func(
        "parseQuery",
        vec![("query", Some("string"))],
        Some("Record<string, unknown>"),
    );
    parse.declared_properties = vec!["query_nested_brackets".into()];
    let a = make_analysis(vec![parse], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("query parse semantics:${_queryParseLabel}"),
        "query parser harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("tag=pro&tag=beta"),
        "query parser harness should check repeated keys, got: {code}"
    );
    assert!(
        code.contains("filter[tags][]=pro"),
        "query parser harness should check nested array bracket parsing, got: {code}"
    );
}

#[test]
fn typescript_query_string_parser_setting_param_uses_extended_mode() {
    let mut parse = func(
        "parseQueryString",
        vec![
            ("input", Some("string")),
            ("setting", Some("QueryParserSetting")),
        ],
        Some("unknown"),
    );
    parse.declared_properties = vec!["query_nested_brackets".into()];
    let a = make_analysis(vec![parse], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("(parseQueryString as Function)(_queryInput, \"extended\")"),
        "two-argument query parser semantics should exercise extended mode, got: {code}"
    );
    assert!(
        code.contains("filter[tags][]=pro"),
        "two-argument query parser should still get nested bracket examples, got: {code}"
    );
}

#[test]
fn typescript_same_value_zero_exact_standard_name_gets_semantic_examples() {
    let same_value_zero = func(
        "sameValueZero",
        vec![("left", Some("unknown")), ("right", Some("unknown"))],
        Some("boolean"),
    );
    let a = make_analysis(vec![same_value_zero], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("sameValueZero semantics:${_sameValueLabel}"),
        "same-value-zero harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("NaN, NaN, true"),
        "same-value-zero harness should check NaN reflexivity, got: {code}"
    );
    assert!(
        code.contains("0, -0, true"),
        "same-value-zero harness should check signed zero equivalence, got: {code}"
    );
}

#[test]
fn typescript_http_request_metadata_property_gets_side_effect_examples() {
    let mut decorate = func(
        "decorateRequest",
        vec![("request", Some("RequestLike"))],
        Some("void"),
    );
    decorate.declared_properties = vec!["http_request_metadata".into()];
    let a = make_analysis(
        vec![decorate],
        vec![ClassInfo {
            name: "RequestLike".into(),
            bases: vec![],
            line: 1,
            fields: vec![FieldInfo {
                name: "headers".into(),
                type_annotation: Some("Record<string, string | undefined>".into()),
                optional: true,
                has_default: false,
            }],
        }],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("HTTP request metadata"),
        "request metadata property should emit side-effect checks, got: {code}"
    );
    assert!(
        code.contains("X-Forwarded-Proto") && code.contains("extended query decoration"),
        "request metadata should cover forwarded protocol and query decoration, got: {code}"
    );
}

#[test]
fn typescript_http_response_helpers_property_gets_side_effect_examples() {
    let mut decorate = func(
        "decorateResponse",
        vec![
            ("response", Some("ResponseLike")),
            ("request", Some("RequestLike")),
        ],
        Some("void"),
    );
    decorate.declared_properties = vec!["http_response_helpers".into()];
    let a = make_analysis(
        vec![decorate],
        vec![
            ClassInfo {
                name: "ResponseLike".into(),
                bases: vec![],
                line: 1,
                fields: vec![FieldInfo {
                    name: "statusCode".into(),
                    type_annotation: Some("number".into()),
                    optional: true,
                    has_default: false,
                }],
            },
            ClassInfo {
                name: "RequestLike".into(),
                bases: vec![],
                line: 2,
                fields: vec![FieldInfo {
                    name: "headers".into(),
                    type_annotation: Some("Record<string, string | undefined>".into()),
                    optional: true,
                    has_default: false,
                }],
            },
        ],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("HTTP response helpers"),
        "response helper property should emit side-effect checks, got: {code}"
    );
    assert!(
        code.contains("location encodes spaces") && code.contains("sendStatus 204 empty body"),
        "response helper checks should cover location and empty-body status semantics, got: {code}"
    );
}

#[test]
fn typescript_http_static_file_property_gets_middleware_examples() {
    let mut create_static = func(
        "createStaticMiddleware",
        vec![
            ("root", Some("string")),
            ("options", Some("StaticMiddlewareOptions")),
        ],
        Some("Handler"),
    );
    create_static.declared_properties = vec!["http_static_file_middleware".into()];
    let a = make_analysis(
        vec![create_static],
        vec![ClassInfo {
            name: "StaticMiddlewareOptions".into(),
            bases: vec![],
            line: 1,
            fields: vec![FieldInfo {
                name: "index".into(),
                type_annotation: Some("string | false".into()),
                optional: true,
                has_default: false,
            }],
        }],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("HTTP static file middleware"),
        "static file middleware property should emit returned-handler checks, got: {code}"
    );
    assert!(
        code.contains("/hello.txt") && code.contains("hello world\\n"),
        "static file middleware should exercise a known project static file, got: {code}"
    );
}

#[test]
fn typescript_feature_flag_resolver_gets_explicit_false_semantics() {
    let classes = vec![ClassInfo {
        name: "Config".into(),
        bases: vec![],
        line: 1,
        fields: vec![FieldInfo {
            name: "flags".into(),
            type_annotation: Some("{ betaCheckout?: boolean | null } | null".into()),
            optional: true,
            has_default: false,
        }],
    }];
    let a = make_analysis(
        vec![func(
            "betaCheckoutEnabled",
            vec![("config", Some("Config"))],
            Some("boolean"),
        )],
        classes,
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("feature flag semantics:${_flagLabel}"),
        "feature-flag harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("_explicitFalse !== false"),
        "feature-flag harness should preserve explicit false overrides, got: {code}"
    );
    assert!(
        code.contains("flags: { [_flagKey]: null }"),
        "feature-flag harness should compare nested null to fallback, got: {code}"
    );
}

#[test]
fn typescript_semver_compare_gets_prerelease_semantics() {
    let a = make_analysis(
        vec![func(
            "compareVersions",
            vec![("left", Some("string")), ("right", Some("string"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("semver compare semantics:${_semverLabel}"),
        "semver compare harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("\"1.0.0-beta.1\", \"1.0.0\", -1"),
        "semver compare harness should check prerelease-vs-release ordering, got: {code}"
    );
    assert!(
        code.contains("\"1.0.0+build.1\", \"1.0.0+build.9\", 0"),
        "semver compare harness should ignore build metadata, got: {code}"
    );
}

#[test]
fn typescript_semver_caret_gets_zero_major_semantics() {
    let a = make_analysis(
        vec![func(
            "matchesCaret",
            vec![("version", Some("string")), ("range", Some("string"))],
            Some("boolean"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("semver caret semantics:${_caretLabel}"),
        "semver caret harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("\"1.3.0-beta.1\", \"^1.2.3\", false"),
        "semver caret harness should exclude prereleases, got: {code}"
    );
    assert!(
        code.contains("\"1.0.2-beta.3\", \"^1.0.2\", false"),
        "semver caret harness should exclude prereleases for stable same-core ranges, got: {code}"
    );
    assert!(
        code.contains("\"0.3.0\", \"^0.2.3\", false"),
        "semver caret harness should enforce zero-major upper bounds, got: {code}"
    );
}

#[test]
fn typescript_defaults_gets_null_and_inherited_semantics() {
    let a = make_analysis(
        vec![func(
            "defaults",
            vec![
                ("object", Some("T")),
                ("...sources", Some("Array<object | null | undefined>")),
            ],
            Some("T"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"defaults\""),
        "defaults should become fuzzable despite a generic target, got: {code}"
    );
    assert!(
        code.contains("_fuzzObject()"),
        "defaults should use object-shaped fuzz inputs, got: {code}"
    );
    assert!(
        code.contains("defaults semantics:${_defaultsLabel}"),
        "defaults harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("({ a: null }, { a: 1 })"),
        "defaults harness should preserve null targets, got: {code}"
    );
    assert!(
        code.contains("Object.create(_defaultsProto)"),
        "defaults harness should exercise inherited enumerable keys, got: {code}"
    );
}

#[test]
fn typescript_prefers_exported_surface_over_internal_helpers() {
    let mut helper = func(
        "encode",
        vec![("key", Some("string")), ("value", Some("string"))],
        Some("string"),
    );
    helper.is_exported = false;
    let api = func(
        "canonicalQuery",
        vec![("params", Some("Record<string, unknown>"))],
        Some("string"),
    );
    let a = make_analysis(vec![helper, api], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"canonicalQuery\""),
        "exported surface should still be fuzzed, got: {code}"
    );
    assert!(
        !code.contains("_fuzzOne(\"encode\""),
        "internal helper should be skipped when exported surface exists, got: {code}"
    );
}

#[test]
fn typescript_fuzzes_non_exported_parser_helper_with_simple_input() {
    let mut helper = func(
        "parseSignatureHeader",
        vec![("header", Some("string"))],
        Some("Record<string, string>"),
    );
    helper.is_exported = false;
    let api = func(
        "verifyRequest",
        vec![("request", Some("Request"))],
        Some("boolean"),
    );
    let a = make_analysis(vec![helper, api], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"parseSignatureHeader\""),
        "simple parser helper should now be fuzzed, got: {code}"
    );
    assert!(
        code.contains("_fuzzOne(\"verifyRequest\""),
        "exported surface should remain fuzzed, got: {code}"
    );
}

#[test]
fn typescript_skips_unresolved_alias_params_in_fuzz() {
    let a = make_analysis(
        vec![
            func(
                "flattenPathArgs",
                vec![("paths", Some("Array<PathValue | Array<PathValue>>"))],
                Some("PathValue[]"),
            ),
            func(
                "normalizeHandle",
                vec![("value", Some("string"))],
                Some("string"),
            ),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        !code.contains("_fuzzOne(\"flattenPathArgs\""),
        "functions with unresolved TS aliases should be skipped, got: {code}"
    );
    assert!(
        code.contains("_fuzzOne(\"normalizeHandle\""),
        "supported signatures should still be synthesized, got: {code}"
    );
}

#[test]
fn typescript_fuzzes_resolved_alias_params() {
    let analysis = AnalysisResult {
        functions: vec![func(
            "flattenPathArgs",
            vec![("paths", Some("Array<PathValue | Array<PathValue>>"))],
            Some("PathValue[]"),
        )],
        classes: vec![],
        aliases: vec![TypeAliasInfo {
            name: "PathValue".into(),
            type_annotation: "string | number | Array<string | number>".into(),
            line: 1,
        }],
        imports: vec![],
        complexity: 1,
        cognitive_complexity: 0,
        max_nesting_depth: 0,
        complexity_breakdown: BTreeMap::new(),
        parse_error: false,
    };
    let code = synthesize_calls(&analysis, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"flattenPathArgs\""),
        "resolved aliases should keep fuzz coverage, got: {code}"
    );
    assert!(
        code.contains("_fuzzStr()") && code.contains("_fuzzNum()"),
        "resolved alias should expand into concrete generators, got: {code}"
    );
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"flattenPathArgs\""))
        .expect("flattenPathArgs fuzz call should exist");
    assert!(
        !fuzz_call.contains("[\"object\"]"),
        "resolved alias arrays should not inherit object edge cases, got: {fuzz_call}"
    );
}

#[test]
fn typescript_recursive_alias_params_do_not_overflow() {
    let analysis = AnalysisResult {
        functions: vec![func(
            "decorateRequest",
            vec![("request", Some("RequestLike"))],
            Some("void"),
        )],
        classes: vec![],
        aliases: vec![
            TypeAliasInfo {
                name: "RequestLike".into(),
                type_annotation:
                    "{ headers?: Record<string, string | undefined>; app?: ApplicationLike }"
                        .into(),
                line: 1,
            },
            TypeAliasInfo {
                name: "ApplicationLike".into(),
                type_annotation: "RouterLike & { parent?: ApplicationLike }".into(),
                line: 2,
            },
            TypeAliasInfo {
                name: "RouterLike".into(),
                type_annotation:
                    "((req: RequestLike) => void) & { use: (...args: unknown[]) => RouterLike; parent?: ApplicationLike }"
                        .into(),
                line: 3,
            },
        ],
        imports: vec![],
        complexity: 1,
        cognitive_complexity: 0,
        max_nesting_depth: 0,
        complexity_breakdown: BTreeMap::new(),
        parse_error: false,
    };
    let code = synthesize_calls(&analysis, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"decorateRequest\""),
        "recursive but object-shaped aliases should remain fuzzable, got: {code}"
    );
    assert!(
        code.len() < 60_000,
        "recursive aliases should be bounded during generator expansion"
    );
}

#[test]
fn typescript_semver_fields_use_constrained_generators() {
    let classes = vec![ClassInfo {
        name: "ParsedVersion".into(),
        bases: vec![],
        line: 1,
        fields: vec![
            FieldInfo {
                name: "major".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "minor".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "patch".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "prerelease".into(),
                type_annotation: Some("string[] | null".into()),
                optional: false,
                has_default: false,
            },
        ],
    }];
    let funcs = vec![func(
        "compareVersions",
        vec![
            ("left", Some("ParsedVersion")),
            ("right", Some("ParsedVersion")),
        ],
        Some("number"),
    )];
    let a = make_analysis(funcs, classes);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("major: _fuzzSemverPart()"),
        "major should use constrained semver part generator, got: {code}"
    );
    assert!(
        code.contains("minor: _fuzzSemverPart()"),
        "minor should use constrained semver part generator, got: {code}"
    );
    assert!(
        code.contains("patch: _fuzzSemverPart()"),
        "patch should use constrained semver part generator, got: {code}"
    );
    assert!(
        code.contains("_fuzzSemverIdentifier()"),
        "prerelease should use constrained semver identifiers, got: {code}"
    );
}

#[test]
fn typescript_comparator_semver_edge_cases_use_semver_version_objects() {
    let classes = vec![ClassInfo {
        name: "ParsedVersion".into(),
        bases: vec![],
        line: 1,
        fields: vec![
            FieldInfo {
                name: "major".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "minor".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "patch".into(),
                type_annotation: Some("number".into()),
                optional: false,
                has_default: false,
            },
            FieldInfo {
                name: "prerelease".into(),
                type_annotation: Some("string[] | null".into()),
                optional: false,
                has_default: false,
            },
        ],
    }];
    let funcs = vec![func(
        "compareCore",
        vec![
            ("left", Some("ParsedVersion")),
            ("right", Some("ParsedVersion")),
        ],
        Some("number"),
    )];
    let a = make_analysis(funcs, classes);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzSemverVersion()"),
        "comparator with ParsedVersion should use semver version generator, got: {code}"
    );
    assert!(
        code.contains("\"semver_version\""),
        "comparator with ParsedVersion should pass semver version edge cases, got: {code}"
    );
}

// ── Enhancement 1: Edge case corpus ─────────────────────────────────────────

#[test]
fn python_edge_cases_in_harness() {
    let a = make_analysis(
        vec![func(
            "add",
            vec![("a", Some("int")), ("b", Some("int"))],
            Some("int"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("_EDGE_INTS"),
        "should include edge int array, got: {code}"
    );
    assert!(
        code.contains("_edge_cases_for"),
        "should call edge case builder, got: {code}"
    );
    assert!(
        code.contains("_all_inputs"),
        "should build combined input list, got: {code}"
    );
    assert!(
        code.contains("_nan_eq"),
        "should have NaN-safe equality, got: {code}"
    );
}

#[test]
fn typescript_edge_cases_in_harness() {
    let a = make_analysis(
        vec![func(
            "add",
            vec![("a", Some("number")), ("b", Some("number"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_EDGE_NUMS"),
        "should include edge num array, got: {code}"
    );
    assert!(
        code.contains("_edgeCasesFor"),
        "should call edge case builder, got: {code}"
    );
    assert!(
        code.contains("_nanSafeEq"),
        "should have NaN-safe equality, got: {code}"
    );
    assert!(
        code.contains("[\"number\", \"number\"]"),
        "should pass param types, got: {code}"
    );
}

#[test]
fn python_edge_str_param_type_list() {
    let a = make_analysis(
        vec![func("clean", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("[\"str\"]"),
        "should have str in param type list, got: {code}"
    );
    assert!(
        code.contains("_EDGE_STRS"),
        "should include edge str array, got: {code}"
    );
}

#[test]
fn verify_catches_empty_string_crash() {
    // A function that crashes on empty string should be caught by edge cases
    let a = make_analysis(
        vec![func("first_char", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    // Edge cases include "" which would trigger IndexError on s[0]
    assert!(
        code.contains("_EDGE_STRS"),
        "edge strings should be in harness, got: {code}"
    );
    assert!(
        code.contains("\"str\""),
        "param type list should include str, got: {code}"
    );
}

// ── Enhancement 2: Smarter property inference ───────────────────────────────

#[test]
fn python_boundedness_requires_declared_property() {
    let mut normalize_label = func("normalize_label", vec![("s", Some("str"))], Some("str"));
    normalize_label.declared_properties = vec!["bounded".into()];
    let a = make_analysis(vec![normalize_label], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("bounded"),
        "declared bounded str→str should check boundedness, got: {code}"
    );
    assert!(
        code.contains("len(_result) <= len(_args[0])"),
        "should have len check, got: {code}"
    );
}

#[test]
fn python_nonneg_requires_declared_property() {
    let mut count_words = func("count_words", vec![("s", Some("str"))], Some("int"));
    count_words.declared_properties = vec!["nonneg".into()];
    let a = make_analysis(vec![count_words], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains(">= 0"),
        "declared nonneg int return should check non-negativity, got: {code}"
    );
}

#[test]
fn python_symmetry_requires_declared_property() {
    let mut distance = func(
        "distance",
        vec![("a", Some("str")), ("b", Some("str"))],
        Some("int"),
    );
    distance.declared_properties = vec!["symmetric".into()];
    let a = make_analysis(vec![distance], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("symmetric"),
        "declared symmetric function should check symmetry, got: {code}"
    );
    assert!(
        code.contains("_args[1], _args[0]"),
        "should swap args for symmetry check, got: {code}"
    );
}

#[test]
fn python_involution_pair() {
    let a = make_analysis(
        vec![
            func("encode", vec![("s", Some("str"))], Some("str")),
            func("decode", vec![("s", Some("str"))], Some("str")),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("Roundtrip"),
        "encode+decode should have roundtrip check, got: {code}"
    );
    assert!(
        code.contains("encode(") && code.contains("decode("),
        "should call both functions, got: {code}"
    );
}

#[test]
fn python_no_boundedness_for_non_matching_name() {
    let a = make_analysis(
        vec![func("transform", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("bounded"),
        "transform should NOT check boundedness, got: {code}"
    );
}

#[test]
fn python_no_nonneg_for_wrong_return_type() {
    let a = make_analysis(
        vec![func("count_words", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains(">= 0"),
        "count→str should NOT check non-negativity, got: {code}"
    );
}

#[test]
fn typescript_boundedness_requires_declared_property() {
    let mut trim_text = func("trim_text", vec![("s", Some("string"))], Some("string"));
    trim_text.declared_properties = vec!["bounded".into()];
    let a = make_analysis(vec![trim_text], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"bounded\""),
        "declared bounded string→string should have bounded property, got: {code}"
    );
}

#[test]
fn typescript_nonneg_requires_declared_property() {
    let mut count_items = func("count_items", vec![("s", Some("string"))], Some("number"));
    count_items.declared_properties = vec!["nonneg".into()];
    let a = make_analysis(vec![count_items], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"nonneg\""),
        "declared nonneg number return should have nonneg property, got: {code}"
    );
}

#[test]
fn typescript_declared_sorted_and_permutation_properties_are_emitted() {
    let mut reorder = func(
        "reorder",
        vec![("values", Some("string[]"))],
        Some("string[]"),
    );
    reorder.declared_properties = vec!["sorted".into(), "permutation".into()];
    let a = make_analysis(vec![reorder], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"sorted\"") && code.contains("\"permutation\""),
        "declared properties should be emitted into the TS harness, got: {code}"
    );
}

#[test]
fn python_declared_clamped_property_is_emitted() {
    let mut clamp_like = func(
        "choose_range",
        vec![
            ("value", Some("int")),
            ("lower", Some("int")),
            ("upper", Some("int")),
        ],
        Some("int"),
    );
    clamp_like.declared_properties = vec!["clamped".into()];
    let a = make_analysis(vec![clamp_like], vec![]);
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("Clamp bounds violated") && code.contains("Clamp passthrough violated"),
        "declared clamped property should emit clamp assertions, got: {code}"
    );
}

#[test]
fn typescript_nonempty_string_for_label_requires_declared_property_for_array_input() {
    let mut secondary_label = func(
        "secondary_label",
        vec![("labels", Some("string[]"))],
        Some("string"),
    );
    secondary_label.declared_properties = vec!["nonempty_string".into()];
    let a = make_analysis(vec![secondary_label], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"nonempty_string\""),
        "label→string should have nonempty string property, got: {code}"
    );
}

#[test]
fn typescript_normalize_plan_code_not_nonempty_string() {
    let a = make_analysis(
        vec![func(
            "normalizePlanCode",
            vec![("value", Some("string | null | undefined"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"normalizePlanCode\""))
        .expect("normalizePlanCode fuzz call should exist");
    assert!(
        !fuzz_call.contains("\"nonempty_string\""),
        "normalize helper should not force nonempty_string, got: {fuzz_call}"
    );
}

#[test]
fn typescript_string_array_edge_cases_for_label() {
    let a = make_analysis(
        vec![func(
            "secondary_label",
            vec![("labels", Some("string[]"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"string_array\""),
        "string[] params should use string-array edge cases, got: {code}"
    );
    assert!(
        code.contains("[\"primary\", \"   \"]"),
        "string-array edge cases should include blank secondary labels, got: {code}"
    );
}

#[test]
fn typescript_nullable_string_array_uses_string_array_edges() {
    let a = make_analysis(
        vec![func(
            "maxStableVersion",
            vec![("versions", Some("Array<string | null | undefined>"))],
            Some("string | null"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    let fuzz_call = code
        .lines()
        .find(|line| line.contains("_fuzzOne(\"maxStableVersion\""))
        .expect("maxStableVersion fuzz call should exist");
    assert!(
        fuzz_call.contains("[\"string_array\"]"),
        "nullable string arrays should use string-array edge cases, got: {fuzz_call}"
    );
}

#[test]
fn typescript_symmetry_requires_declared_property() {
    let mut hamming_distance = func(
        "hamming_distance",
        vec![("a", Some("string")), ("b", Some("string"))],
        Some("number"),
    );
    hamming_distance.declared_properties = vec!["symmetric".into()];
    let a = make_analysis(vec![hamming_distance], vec![]);
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"symmetric\""),
        "declared symmetric function should have symmetric property, got: {code}"
    );
}

#[test]
fn typescript_involution_pair() {
    let a = make_analysis(
        vec![
            func("base64_encode", vec![("s", Some("string"))], Some("string")),
            func("base64_decode", vec![("s", Some("string"))], Some("string")),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("ROUNDTRIP"),
        "encode+decode should have roundtrip check, got: {code}"
    );
    assert!(
        code.contains("base64_encode") && code.contains("base64_decode"),
        "should reference both, got: {code}"
    );
}

// ── Method skipping (Change 3) ──────────────────────────────────────────────

#[test]
fn python_skips_methods_in_fuzz() {
    let a = make_analysis(
        vec![
            FunctionInfo {
                name: "bar".to_string(),
                params: vec![ParamInfo {
                    name: "x".to_string(),
                    type_annotation: Some("int".to_string()),
                    default_value: None,
                    keyword_only: false,
                }],
                return_type: Some("int".to_string()),
                line: 2,
                end_line: 3,
                complexity: 1,
                cognitive_complexity: 0,
                max_nesting_depth: 0,
                complexity_breakdown: BTreeMap::new(),
                is_method: true,
                is_nested: false,
                is_exported: false,
                declared_properties: vec![],
                invocation_target: None,
                returned_callables: vec![],
            },
            func("standalone", vec![("x", Some("int"))], Some("int")),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("FUZZ bar"),
        "method bar should be skipped, got: {code}"
    );
    assert!(
        code.contains("FUZZ standalone"),
        "free function should be included, got: {code}"
    );
}

// ── Structured fuzz failure reporting (Change 5) ────────────────────────────

#[test]
fn python_fuzz_emits_json_sentinel() {
    let a = make_analysis(
        vec![func("greet", vec![("name", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("__COURT_JESTER_FUZZ_JSON__"),
        "should contain JSON sentinel, got: {code}"
    );
    assert!(
        code.contains("_FUZZ_RESULTS"),
        "should collect results, got: {code}"
    );
    assert!(
        code.contains("_json.dumps"),
        "should JSON-serialize results, got: {code}"
    );
}

#[test]
fn typescript_fuzz_emits_json_sentinel() {
    let a = make_analysis(
        vec![func(
            "add",
            vec![("a", Some("number")), ("b", Some("number"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("__COURT_JESTER_FUZZ_JSON__"),
        "should contain JSON sentinel, got: {code}"
    );
    assert!(
        code.contains("_fuzzResults"),
        "should collect results, got: {code}"
    );
    assert!(
        code.contains("JSON.stringify(_fuzzResults)"),
        "should JSON-serialize results, got: {code}"
    );
}

// ── Built-in type and callback generators ────────────────────────────────────

#[test]
fn typescript_date_generator() {
    let a = make_analysis(
        vec![func("process", vec![("d", Some("Date"))], Some("string"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("new Date("),
        "Date should generate new Date(), got: {code}"
    );
}

#[test]
fn typescript_promise_generator() {
    let a = make_analysis(
        vec![func("process", vec![("p", Some("Promise<string>"))], None)],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("Promise.resolve("),
        "Promise<string> should generate Promise.resolve(), got: {code}"
    );
}

#[test]
fn typescript_web_platform_generators() {
    let a = make_analysis(
        vec![
            func(
                "readHeaders",
                vec![("headers", Some("Headers"))],
                Some("string"),
            ),
            func(
                "readRequest",
                vec![("request", Some("Request"))],
                Some("boolean"),
            ),
            func(
                "rewriteParams",
                vec![("params", Some("URLSearchParams"))],
                Some("string"),
            ),
            func(
                "inspectResponse",
                vec![("response", Some("Response"))],
                Some("number"),
            ),
        ],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzHeaders()"),
        "Headers should use _fuzzHeaders(), got: {code}"
    );
    assert!(
        code.contains("_fuzzRequest()"),
        "Request should use _fuzzRequest(), got: {code}"
    );
    assert!(
        code.contains("_fuzzUrlSearchParams()"),
        "URLSearchParams should use _fuzzUrlSearchParams(), got: {code}"
    );
    assert!(
        code.contains("_fuzzResponse()"),
        "Response should use _fuzzResponse(), got: {code}"
    );
}

#[test]
fn typescript_callback_generator() {
    let a = make_analysis(
        vec![func(
            "process",
            vec![("cb", Some("(value: string) => void"))],
            None,
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("() => undefined"),
        "callback should generate stub function, got: {code}"
    );
}

#[test]
fn typescript_map_set_generator() {
    let a = make_analysis(
        vec![func(
            "process",
            vec![("m", Some("Map")), ("s", Some("Set"))],
            None,
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("new Map()"),
        "Map should generate new Map(), got: {code}"
    );
    assert!(
        code.contains("new Set()"),
        "Set should generate new Set(), got: {code}"
    );
}

#[test]
fn typescript_generic_collection_generators() {
    let a = make_analysis(
        vec![func(
            "process",
            vec![
                ("taken", Some("Set<string>")),
                ("lookup", Some("Map<string, number>")),
            ],
            None,
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("new Set(Array.from"),
        "Set<string> should generate a populated Set, got: {code}"
    );
    assert!(
        code.contains("new Map(Array.from"),
        "Map<string, number> should generate a populated Map, got: {code}"
    );
}

#[test]
fn typescript_readonly_array_generator() {
    let a = make_analysis(
        vec![func(
            "joinLabels",
            vec![("labels", Some("ReadonlyArray<string>"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("_fuzzOne(\"joinLabels\""),
        "ReadonlyArray<string> params should remain fuzzable, got: {code}"
    );
    assert!(
        code.contains("Array.from({length: _fuzzIntRange(0,5)}, () => _fuzzStr())"),
        "ReadonlyArray<string> should use array generation, got: {code}"
    );
}

#[test]
fn typescript_set_param_is_no_longer_skipped_as_unsupported() {
    let analysis = make_analysis(
        vec![func(
            "uniqueName",
            vec![("base", Some("string")), ("taken", Some("Set<string>"))],
            Some("string"),
        )],
        vec![],
    );
    let plan = synthesize_plan(&analysis, &Language::TypeScript);
    assert!(
        plan.code.contains("_fuzzOne(\"uniqueName\""),
        "Set<string> params should remain fuzzable, got: {}",
        plan.code
    );
    let coverage = plan
        .coverage
        .iter()
        .find(|entry| entry.function == "uniqueName")
        .expect("coverage entry for uniqueName");
    assert_eq!(coverage.status, FuzzFunctionStatus::Fuzzed);
}

#[test]
fn python_callable_generator() {
    let a = make_analysis(
        vec![func("process", vec![("cb", Some("Callable"))], None)],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("lambda"),
        "Callable should generate lambda, got: {code}"
    );
}
