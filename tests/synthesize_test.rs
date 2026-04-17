use court_jester_mcp::tools::synthesize::synthesize_calls;
use court_jester_mcp::types::*;
use std::collections::BTreeMap;

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
fn python_idempotency_for_clean_function() {
    let a = make_analysis(
        vec![func("clean_text", vec![("s", Some("str"))], Some("str"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("idempotent"),
        "clean_text str→str should check idempotency, got: {code}"
    );
}

#[test]
fn python_no_idempotency_for_non_clean_names() {
    let a = make_analysis(
        vec![func("double", vec![("x", Some("int"))], Some("int"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        !code.contains("idempotent"),
        "double should NOT check idempotency, got: {code}"
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
        code.contains("mode=_args[1]"),
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
        fuzz_call.ends_with(", [\"object\"], []);"),
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
        code.contains("new URLSearchParams"),
        "query-string harness should decode query output, got: {code}"
    );
    assert!(
        code.contains("query semantics:${_queryLabel}"),
        "query-string harness should label semantic failures, got: {code}"
    );
    assert!(
        code.contains("_asciiFold(\"naïve café\")"),
        "query-string harness should check accent folding, got: {code}"
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
fn python_boundedness_for_normalize() {
    let a = make_analysis(
        vec![func(
            "normalize_label",
            vec![("s", Some("str"))],
            Some("str"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("bounded"),
        "normalize str→str should check boundedness, got: {code}"
    );
    assert!(
        code.contains("len(_result) <= len(_args[0])"),
        "should have len check, got: {code}"
    );
}

#[test]
fn python_nonneg_for_count() {
    let a = make_analysis(
        vec![func("count_words", vec![("s", Some("str"))], Some("int"))],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains(">= 0"),
        "count_words → int should check non-negativity, got: {code}"
    );
}

#[test]
fn python_symmetry_for_distance() {
    let a = make_analysis(
        vec![func(
            "distance",
            vec![("a", Some("str")), ("b", Some("str"))],
            Some("int"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::Python);
    assert!(
        code.contains("symmetric"),
        "distance(str,str) should check symmetry, got: {code}"
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
fn typescript_boundedness_for_trim() {
    let a = make_analysis(
        vec![func(
            "trim_text",
            vec![("s", Some("string"))],
            Some("string"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"bounded\""),
        "trim string→string should have bounded property, got: {code}"
    );
}

#[test]
fn typescript_nonneg_for_count() {
    let a = make_analysis(
        vec![func(
            "count_items",
            vec![("s", Some("string"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"nonneg\""),
        "count→number should have nonneg property, got: {code}"
    );
}

#[test]
fn typescript_nonempty_string_for_label() {
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
        fuzz_call.ends_with(", []);"),
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
fn typescript_symmetry_for_distance() {
    let a = make_analysis(
        vec![func(
            "hamming_distance",
            vec![("a", Some("string")), ("b", Some("string"))],
            Some("number"),
        )],
        vec![],
    );
    let code = synthesize_calls(&a, &Language::TypeScript);
    assert!(
        code.contains("\"symmetric\""),
        "hamming_distance(string,string) should have symmetric property, got: {code}"
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
