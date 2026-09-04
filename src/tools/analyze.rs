use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use tree_sitter::Parser;

use crate::types::*;
const MAX_ANALYSIS_SYNTAX_DEPTH: usize = 512;

fn syntax_depth_violation(root: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut pending = vec![(root, 0usize)];
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_ANALYSIS_SYNTAX_DEPTH {
            return Some(node);
        }
        let mut cursor = node.walk();
        pending.extend(
            node.named_children(&mut cursor)
                .map(|child| (child, depth + 1)),
        );
    }
    None
}

pub fn analyze(code: &str, language: &Language) -> AnalysisResult {
    let context = SourceContext {
        language: *language,
        mode: SourceMode::for_language(language),
        source_file: None,
        virtual_file_path: None,
    };
    analyze_with_context(code, &context)
}

pub fn analyze_with_context(code: &str, context: &SourceContext) -> AnalysisResult {
    let mut parser = Parser::new();
    let grammar_mode = context.mode;
    match grammar_mode {
        SourceMode::Python => {
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .expect("Failed to load Python grammar");
        }
        SourceMode::TypeScript => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .expect("Failed to load TypeScript grammar");
        }
        SourceMode::Tsx => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
                .expect("Failed to load TSX grammar");
        }
    }

    let tree = match parser.parse(code, None) {
        Some(t) => t,
        None => {
            return AnalysisResult {
                functions: vec![],
                classes: vec![],
                aliases: vec![],
                imports: vec![],
                complexity: 1,
                cognitive_complexity: 0,
                max_nesting_depth: 0,
                complexity_breakdown: BTreeMap::new(),
                parse_error: true,
                source_mode: grammar_mode,
                parse_diagnostics: vec![ParseDiagnostic {
                    kind: "error".into(),
                    message: "Parser could not produce a syntax tree at 1:1".into(),
                    start_line: 1,
                    start_column: 1,
                    end_line: 1,
                    end_column: 1,
                    excerpt: code
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect(),
                }],
            };
        }
    };

    let root = tree.root_node();
    let bytes = code.as_bytes();
    if let Some(node) = syntax_depth_violation(root) {
        let start = node.start_position();
        let end = node.end_position();
        return AnalysisResult {
            functions: vec![],
            classes: vec![],
            aliases: vec![],
            imports: vec![],
            complexity: 1,
            cognitive_complexity: 0,
            max_nesting_depth: MAX_ANALYSIS_SYNTAX_DEPTH,
            complexity_breakdown: BTreeMap::new(),
            parse_error: true,
            source_mode: grammar_mode,
            parse_diagnostics: vec![ParseDiagnostic {
                kind: "unsupported".into(),
                message: format!(
                    "Syntax nesting exceeds supported analysis depth of {MAX_ANALYSIS_SYNTAX_DEPTH}"
                ),
                start_line: start.row + 1,
                start_column: start.column + 1,
                end_line: end.row + 1,
                end_column: end.column + 1,
                excerpt: code
                    .lines()
                    .nth(start.row)
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect(),
            }],
        };
    }
    let parse_diagnostics = parse_diagnostics(&root, code);
    let semantic_language = context.language;
    let file_complexity = program_complexity(&root, &semantic_language, bytes);

    let mut functions = vec![];
    let mut classes = vec![];
    let mut aliases = vec![];
    let mut imports = vec![];

    match semantic_language {
        Language::Python => {
            visit_python(&root, bytes, &mut functions, &mut classes, &mut imports, 0);
        }
        Language::TypeScript => {
            visit_typescript(
                &root,
                bytes,
                &mut functions,
                &mut classes,
                &mut aliases,
                &mut imports,
                0,
            );
            mark_typescript_explicit_exports(&root, bytes, &mut functions);
            apply_typescript_const_tuple_alias_domains(&root, bytes, &mut aliases);
            apply_typescript_keyof_alias_domains(&root, bytes, &mut aliases);
        }
    }

    annotate_function_source_directives(code, &semantic_language, &mut functions);

    AnalysisResult {
        functions,
        classes,
        aliases,
        imports,
        complexity: file_complexity.cyclomatic,
        cognitive_complexity: file_complexity.cognitive,
        max_nesting_depth: file_complexity.max_nesting_depth,
        complexity_breakdown: file_complexity.breakdown,
        parse_error: root.has_error(),
        source_mode: grammar_mode,
        parse_diagnostics,
    }
}

fn parse_diagnostics(root: &tree_sitter::Node<'_>, source: &str) -> Vec<ParseDiagnostic> {
    fn visit(node: tree_sitter::Node<'_>, source: &str, diagnostics: &mut Vec<ParseDiagnostic>) {
        if node.is_error() || node.is_missing() {
            let start = node.start_position();
            let end = node.end_position();
            let missing = node.is_missing();
            let node_kind = node.kind();
            let message = if missing {
                format!("Missing syntax node {node_kind}")
            } else {
                format!("Unexpected syntax node {node_kind}")
            };
            let excerpt = source
                .lines()
                .nth(start.row)
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            diagnostics.push(ParseDiagnostic {
                kind: if missing { "missing" } else { "error" }.into(),
                message,
                start_line: start.row + 1,
                start_column: start.column + 1,
                end_line: end.row + 1,
                end_column: end.column + 1,
                excerpt,
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            visit(child, source, diagnostics);
        }
    }

    let mut diagnostics = Vec::new();
    visit(*root, source, &mut diagnostics);
    diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic.start_line,
            diagnostic.start_column,
            diagnostic.end_line,
            diagnostic.end_column,
            diagnostic.kind.clone(),
        )
    });
    diagnostics.dedup_by(|left, right| {
        left.start_line == right.start_line
            && left.start_column == right.start_column
            && left.end_line == right.end_line
            && left.end_column == right.end_column
            && left.kind == right.kind
    });
    diagnostics
}

fn text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

const SOURCE_IGNORE_DIRECTIVE_MARKERS: [&str; 2] = ["@court-jester-ignore", "court-jester-ignore"];
const SOURCE_PROPERTY_DIRECTIVE_MARKERS: [&str; 2] =
    ["@court-jester-properties", "court-jester-properties"];
const TS_SUPPORTED_CONTAINER_CALLEES: [&str; 2] = ["create", "createStore"];

fn directive_targets_stage(line: &str, stage: &str) -> bool {
    let lower = line.to_lowercase();
    let stage = stage.to_lowercase();
    SOURCE_IGNORE_DIRECTIVE_MARKERS.iter().any(|marker| {
        lower.find(marker).is_some_and(|idx| {
            lower[idx + marker.len()..]
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
                .any(|token| token == stage || token == "all")
        })
    })
}

fn normalize_declared_property(token: &str) -> Option<&'static str> {
    match token {
        "idempotent" => Some("idempotent"),
        "sorted" => Some("sorted"),
        "permutation" => Some("permutation"),
        "nonnegative" | "nonneg" => Some("nonneg"),
        "clamped" => Some("clamped"),
        "nonempty" | "nonempty_string" => Some("nonempty_string"),
        "symmetric" => Some("symmetric"),
        "antisymmetric" | "comparator" => Some("antisymmetric"),
        "bounded" => Some("bounded"),
        "involution" | "involutive" => Some("involution"),
        "monotonic" | "monotone" | "nondecreasing" => Some("monotonic"),
        "order_invariant" | "order-independent" | "order_independent" => Some("order_invariant"),
        "no_nullish_string" => Some("no_nullish_string"),
        "palindrome" | "palindrome_sequence" => Some("palindrome"),
        "query_nested_brackets" | "nested_query_brackets" | "query_bracket_notation" => {
            Some("query_nested_brackets")
        }
        "same_value_zero" | "samevaluezero" | "same-value-zero" => Some("same_value_zero"),
        "pep440_version_ordering" | "pep440_ordering" | "pep_440_ordering" => {
            Some("pep440_version_ordering")
        }
        "pep440_specifier_membership" | "pep440_specifier" | "pep_440_specifier" => {
            Some("pep440_specifier_membership")
        }
        "pep440_filter_prerelease" | "pep440_prerelease_fallback" => {
            Some("pep440_filter_prerelease")
        }
        "cookie_value_quote" | "cookie_quote_value" => Some("cookie_value_quote"),
        "cookie_header_quote" | "cookie_header_quoting" => Some("cookie_header_quote"),
        "http_request_metadata" | "request_metadata" | "request_decoration" => {
            Some("http_request_metadata")
        }
        "http_response_helpers" | "response_helpers" | "response_header_helpers" => {
            Some("http_response_helpers")
        }
        "http_static_file_middleware" | "static_file_middleware" | "static_serving" => {
            Some("http_static_file_middleware")
        }
        _ => None,
    }
}

fn extract_declared_properties_from_line(line: &str) -> Vec<String> {
    let lower = line.to_lowercase();
    SOURCE_PROPERTY_DIRECTIVE_MARKERS
        .iter()
        .find_map(|marker| {
            lower.find(marker).map(|idx| {
                lower[idx + marker.len()..]
                    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
                    .filter_map(normalize_declared_property)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

fn is_comment_only_line(trimmed: &str, language: &Language) -> bool {
    match language {
        Language::Python => trimmed.starts_with('#'),
        Language::TypeScript => {
            trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
                || trimmed.starts_with("*/")
        }
    }
}

fn source_context_lines<'a>(code: &'a str, language: &Language, line: usize) -> Vec<&'a str> {
    if line == 0 {
        return Vec::new();
    }

    let lines: Vec<&str> = code.lines().collect();
    if line > lines.len() {
        return Vec::new();
    }

    let mut context = vec![lines[line - 1]];

    let mut idx = line.saturating_sub(2);
    let mut have_prior_line = line >= 2;
    while have_prior_line {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() {
            break;
        }
        context.push(lines[idx]);
        if !is_comment_only_line(trimmed, language) {
            break;
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
        have_prior_line = true;
    }

    context
}

pub fn source_directive_suppresses_complexity(
    code: &str,
    language: &Language,
    line: usize,
) -> bool {
    source_context_lines(code, language, line)
        .into_iter()
        .any(|candidate| directive_targets_stage(candidate, "complexity"))
}

pub fn source_declared_properties(code: &str, language: &Language, line: usize) -> Vec<String> {
    let mut properties = Vec::new();
    for candidate in source_context_lines(code, language, line) {
        for property in extract_declared_properties_from_line(candidate) {
            if !properties.iter().any(|existing| existing == &property) {
                properties.push(property);
            }
        }
    }
    properties
}

fn annotate_function_source_directives(
    code: &str,
    language: &Language,
    functions: &mut [FunctionInfo],
) {
    for function in functions {
        function.declared_properties = source_declared_properties(code, language, function.line);
    }
}

#[derive(Debug, Default, Clone)]
struct ComplexityStats {
    cyclomatic: usize,
    cognitive: usize,
    max_nesting_depth: usize,
    breakdown: BTreeMap<String, usize>,
}

#[derive(Clone, Copy)]
struct Decision {
    key: &'static str,
    nesting_sensitive: bool,
    increases_nesting: bool,
}

#[derive(Clone, Copy)]
enum ComplexityEvent {
    Decision(Decision),
    NestingOnly,
}

impl ComplexityStats {
    fn new() -> Self {
        Self {
            cyclomatic: 1,
            ..Self::default()
        }
    }

    fn record_decision(&mut self, key: &'static str, nesting: usize, nesting_sensitive: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nesting_sensitive { 1 + nesting } else { 1 };
        *self.breakdown.entry(key.to_string()).or_insert(0) += 1;
    }

    fn note_nesting(&mut self, nesting: usize) {
        self.max_nesting_depth = self.max_nesting_depth.max(nesting);
    }
}

/// Walk the full file and count control-flow nodes.
fn program_complexity(
    root: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> ComplexityStats {
    let mut stats = ComplexityStats::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_complexity(&child, &mut stats, language, source, 0, false);
    }
    stats
}

/// Walk a callable subtree while ignoring nested callables so parent functions do
/// not inherit child function complexity.
fn callable_complexity(
    root: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> ComplexityStats {
    let mut stats = ComplexityStats::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        walk_complexity(&child, &mut stats, language, source, 0, true);
    }
    stats
}

fn walk_complexity(
    node: &tree_sitter::Node,
    stats: &mut ComplexityStats,
    language: &Language,
    source: &[u8],
    nesting: usize,
    skip_nested_callables: bool,
) {
    if skip_nested_callables && is_callable(node, language) {
        return;
    }

    let mut child_nesting = nesting;
    if let Some(event) = complexity_event(node, language, source) {
        match event {
            ComplexityEvent::Decision(decision) => {
                stats.record_decision(decision.key, nesting, decision.nesting_sensitive);
                if decision.increases_nesting {
                    child_nesting += 1;
                    stats.note_nesting(child_nesting);
                }
            }
            ComplexityEvent::NestingOnly => {
                child_nesting += 1;
                stats.note_nesting(child_nesting);
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_complexity(
            &child,
            stats,
            language,
            source,
            child_nesting,
            skip_nested_callables,
        );
    }
}

fn is_callable(node: &tree_sitter::Node, language: &Language) -> bool {
    match language {
        Language::Python => matches!(node.kind(), "function_definition" | "lambda"),
        Language::TypeScript => matches!(
            node.kind(),
            "function_declaration" | "function_expression" | "method_definition" | "arrow_function"
        ),
    }
}

fn complexity_event(
    node: &tree_sitter::Node,
    language: &Language,
    source: &[u8],
) -> Option<ComplexityEvent> {
    match language {
        Language::Python => match node.kind() {
            "if_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "if",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "elif_clause" => Some(ComplexityEvent::Decision(Decision {
                key: "elif",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "for_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "for",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "while_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "while",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "except_clause" => Some(ComplexityEvent::Decision(Decision {
                key: "except",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "conditional_expression" => Some(ComplexityEvent::Decision(Decision {
                key: "ternary",
                nesting_sensitive: true,
                increases_nesting: false,
            })),
            "boolean_operator" => Some(ComplexityEvent::Decision(Decision {
                key: "boolean_op",
                nesting_sensitive: false,
                increases_nesting: false,
            })),
            "match_statement" => Some(ComplexityEvent::NestingOnly),
            "case_clause" => Some(ComplexityEvent::Decision(Decision {
                key: "case",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            _ => None,
        },
        Language::TypeScript => match node.kind() {
            "if_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "if",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "for_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "for",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "for_in_statement" => {
                let key = match node
                    .child_by_field_name("operator")
                    .map(|n| text(&n, source))
                {
                    Some("of") => "for_of",
                    _ => "for_in",
                };
                Some(ComplexityEvent::Decision(Decision {
                    key,
                    nesting_sensitive: true,
                    increases_nesting: true,
                }))
            }
            "while_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "while",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "do_statement" => Some(ComplexityEvent::Decision(Decision {
                key: "do",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "catch_clause" => Some(ComplexityEvent::Decision(Decision {
                key: "catch",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "ternary_expression" => Some(ComplexityEvent::Decision(Decision {
                key: "ternary",
                nesting_sensitive: true,
                increases_nesting: false,
            })),
            "switch_statement" => Some(ComplexityEvent::NestingOnly),
            "switch_case" => Some(ComplexityEvent::Decision(Decision {
                key: "switch_case",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "switch_default" => Some(ComplexityEvent::Decision(Decision {
                key: "switch_default",
                nesting_sensitive: true,
                increases_nesting: true,
            })),
            "binary_expression" => match node
                .child_by_field_name("operator")
                .map(|n| text(&n, source))
            {
                Some("&&") => Some(ComplexityEvent::Decision(Decision {
                    key: "logical_and",
                    nesting_sensitive: false,
                    increases_nesting: false,
                })),
                Some("||") => Some(ComplexityEvent::Decision(Decision {
                    key: "logical_or",
                    nesting_sensitive: false,
                    increases_nesting: false,
                })),
                Some("??") => Some(ComplexityEvent::Decision(Decision {
                    key: "nullish_coalescing",
                    nesting_sensitive: false,
                    increases_nesting: false,
                })),
                _ => None,
            },
            _ => None,
        },
    }
}

/// Check if a Python function's first parameter is `self` or `cls`.
fn has_self_or_cls_first_param(func_node: &tree_sitter::Node, source: &[u8]) -> bool {
    let params_node = match func_node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return false,
    };
    let mut cursor = params_node.walk();
    if let Some(child) = params_node.named_children(&mut cursor).next() {
        match child.kind() {
            "identifier" => {
                let name = text(&child, source);
                return name == "self" || name == "cls";
            }
            "typed_parameter" => {
                let name = child.named_child(0).map(|n| text(&n, source)).unwrap_or("");
                return name == "self" || name == "cls";
            }
            _ => return false,
        }
    }
    false
}

/// Extract the inner type text from a type_annotation node (strips leading `: `).
fn type_text(node: &tree_sitter::Node, source: &[u8]) -> String {
    let raw = text(node, source);
    raw.trim_start_matches(':').trim().to_string()
}

fn function_effects(
    function: &tree_sitter::Node,
    source: &[u8],
    language: &Language,
) -> Vec<FunctionEffect> {
    fn root_node(mut node: tree_sitter::Node<'_>) -> tree_sitter::Node<'_> {
        while let Some(parent) = node.parent() {
            node = parent;
        }
        node
    }

    fn collect_identifiers(
        node: tree_sitter::Node<'_>,
        source: &[u8],
        names: &mut HashSet<String>,
    ) {
        if node.kind() == "identifier" {
            names.insert(text(&node, source).to_string());
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_identifiers(child, source, names);
        }
    }

    fn collect_module_state(
        node: tree_sitter::Node<'_>,
        source: &[u8],
        language: &Language,
        names: &mut HashSet<String>,
    ) {
        match language {
            Language::TypeScript => {
                if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
                    let declaration = text(&node, source).trim_start();
                    let mutable_declaration =
                        declaration.starts_with("let ") || declaration.starts_with("var ");
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        if child.kind() != "variable_declarator" {
                            continue;
                        }
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let mutable_value =
                            child.child_by_field_name("value").is_some_and(|value| {
                                matches!(
                                    value.kind(),
                                    "object"
                                        | "array"
                                        | "new_expression"
                                        | "call_expression"
                                        | "await_expression"
                                )
                            });
                        if mutable_declaration || mutable_value {
                            collect_identifiers(name, source, names);
                        }
                    }
                    return;
                }
                if node.kind() != "export_statement" {
                    return;
                }
            }
            Language::Python => {
                if matches!(
                    node.kind(),
                    "assignment" | "augmented_assignment" | "annotated_assignment"
                ) {
                    if let Some(target) = node.child_by_field_name("left") {
                        collect_identifiers(target, source, names);
                    }
                    return;
                }
                return;
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_module_state(child, source, language, names);
        }
    }

    fn is_callable_node(kind: &str) -> bool {
        matches!(
            kind,
            "function_definition"
                | "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
        )
    }

    fn collect_binding_names(
        node: tree_sitter::Node<'_>,
        source: &[u8],
        names: &mut HashSet<String>,
    ) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                names.insert(text(&node, source).to_string());
            }
            "pair_pattern" => {
                if let Some(value) = node.child_by_field_name("value") {
                    collect_binding_names(value, source, names);
                }
            }
            "assignment_pattern" => {
                if let Some(left) = node.child_by_field_name("left") {
                    collect_binding_names(left, source, names);
                }
            }
            "rest_pattern"
            | "list_splat_pattern"
            | "dictionary_splat_pattern"
            | "object_pattern"
            | "array_pattern"
            | "tuple_pattern"
            | "list_pattern"
            | "pattern_list" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    collect_binding_names(child, source, names);
                }
            }
            _ => {}
        }
    }

    fn add_bindings(
        binding: tree_sitter::Node<'_>,
        scope: tree_sitter::Node<'_>,
        source: &[u8],
        bindings: &mut HashMap<usize, HashSet<String>>,
    ) {
        collect_binding_names(binding, source, bindings.entry(scope.id()).or_default());
    }

    fn collect_local_bindings(
        node: tree_sitter::Node<'_>,
        function: tree_sitter::Node<'_>,
        source: &[u8],
        language: &Language,
        bindings: &mut HashMap<usize, HashSet<String>>,
    ) {
        if node.id() != function.id() && is_callable_node(node.kind()) {
            if matches!(node.kind(), "function_definition" | "function_declaration") {
                if let Some(name) = node.child_by_field_name("name") {
                    let scope = match language {
                        Language::TypeScript => ts_declaration_binding_scope(node, function, false),
                        Language::Python => function,
                    };
                    add_bindings(name, scope, source, bindings);
                }
            }
            return;
        }

        match language {
            Language::TypeScript => match node.kind() {
                "variable_declarator" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        let function_scoped = node.parent().is_some_and(|declaration| {
                            declaration.kind() == "variable_declaration"
                        });
                        let scope = ts_declaration_binding_scope(node, function, function_scoped);
                        add_bindings(name, scope, source, bindings);
                    }
                }
                "class_declaration" | "enum_declaration" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        let scope = ts_declaration_binding_scope(node, function, false);
                        add_bindings(name, scope, source, bindings);
                    }
                }
                "catch_clause" => {
                    if let Some(parameter) = node.child_by_field_name("parameter") {
                        add_bindings(parameter, node, source, bindings);
                    }
                }
                _ => {}
            },
            Language::Python => match node.kind() {
                "assignment"
                | "augmented_assignment"
                | "annotated_assignment"
                | "named_expression" => {
                    if let Some(target) = node
                        .child_by_field_name("left")
                        .or_else(|| node.child_by_field_name("name"))
                    {
                        add_bindings(target, function, source, bindings);
                    }
                }
                "for_statement" => {
                    if let Some(target) = node.child_by_field_name("left") {
                        add_bindings(target, function, source, bindings);
                    }
                }
                _ => {}
            },
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_local_bindings(child, function, source, language, bindings);
        }
    }

    fn is_locally_bound(
        node: tree_sitter::Node<'_>,
        function: tree_sitter::Node<'_>,
        name: &str,
        bindings: &HashMap<usize, HashSet<String>>,
    ) -> bool {
        let mut ancestor = Some(node);
        while let Some(scope) = ancestor {
            if bindings
                .get(&scope.id())
                .is_some_and(|names| names.contains(name))
            {
                return true;
            }
            if scope.id() == function.id() {
                break;
            }
            ancestor = scope.parent();
        }
        false
    }

    fn mutation_root<'a>(mut node: tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
        loop {
            match node.kind() {
                "identifier" | "this" => return Some(text(&node, source)),
                "attribute" | "member_expression" | "subscript" | "subscript_expression" => {
                    node = node
                        .child_by_field_name("object")
                        .or_else(|| node.named_child(0))?;
                }
                _ => return None,
            }
        }
    }

    fn classify_call(callee: &str) -> Option<FunctionEffect> {
        let compact = callee
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        let lower = compact.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "math.random"
                | "crypto.randomuuid"
                | "crypto.getrandomvalues"
                | "randomuuid"
                | "uuidv4"
                | "nanoid"
                | "os.urandom"
                | "uuid.uuid4"
        ) || lower.starts_with("random.")
            || lower.starts_with("secrets.")
        {
            return Some(FunctionEffect::Randomness);
        }
        if matches!(
            lower.as_str(),
            "date.now"
                | "performance.now"
                | "process.hrtime"
                | "process.hrtime.bigint"
                | "time.time"
                | "time.monotonic"
                | "time.perf_counter"
                | "time.process_time"
                | "datetime.now"
                | "datetime.utcnow"
                | "date.today"
        ) {
            return Some(FunctionEffect::Time);
        }
        if matches!(
            lower.as_str(),
            "settimeout"
                | "setinterval"
                | "setimmediate"
                | "requestanimationframe"
                | "time.sleep"
                | "asyncio.sleep"
                | "threading.timer"
        ) || lower.ends_with(".call_later")
        {
            return Some(FunctionEffect::Timer);
        }
        if lower == "fetch"
            || lower == "open"
            || lower == "input"
            || lower.starts_with("fs.")
            || lower.starts_with("deno.")
            || lower.starts_with("bun.file")
            || lower.starts_with("bun.write")
            || lower.starts_with("localstorage.")
            || lower.starts_with("sessionstorage.")
            || lower.starts_with("requests.")
            || lower.starts_with("httpx.")
            || lower.starts_with("urllib.")
            || lower.starts_with("socket.")
            || lower.starts_with("process.stdin.")
            || lower.starts_with("process.stdout.")
            || lower.starts_with("process.stderr.")
            || [
                ".read_text",
                ".read_bytes",
                ".write_text",
                ".write_bytes",
                ".read_file",
                ".write_file",
            ]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
        {
            return Some(FunctionEffect::Io);
        }
        None
    }

    fn visit_effects(
        node: tree_sitter::Node<'_>,
        root: tree_sitter::Node<'_>,
        source: &[u8],
        module_state: &HashSet<String>,
        parameter_names: &HashSet<String>,
        local_bindings: &HashMap<usize, HashSet<String>>,
        effects: &mut Vec<FunctionEffect>,
    ) {
        if node != root && is_callable_node(node.kind()) {
            return;
        }

        if node.kind() == "identifier" {
            let name = text(&node, source);
            if module_state.contains(name) && !is_locally_bound(node, root, name, local_bindings) {
                effects.push(FunctionEffect::MutableState);
            }
        }

        if matches!(node.kind(), "call" | "call_expression") {
            if let Some(callee) = node
                .child_by_field_name("function")
                .or_else(|| node.named_child(0))
            {
                if let Some(effect) = classify_call(text(&callee, source)) {
                    effects.push(effect);
                }
            }
        } else if node.kind() == "new_expression"
            && text(&node, source).trim_start().starts_with("new Date")
        {
            effects.push(FunctionEffect::Time);
        } else if matches!(
            node.kind(),
            "assignment_expression"
                | "augmented_assignment_expression"
                | "augmented_assignment"
                | "update_expression"
        ) {
            if let Some(target) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"))
                .or_else(|| node.named_child(0))
            {
                if mutation_root(target, source).is_some_and(|root_name| {
                    root_name == "this"
                        || root_name == "self"
                        || parameter_names.contains(root_name)
                        || (module_state.contains(root_name)
                            && !is_locally_bound(target, root, root_name, local_bindings))
                }) {
                    effects.push(FunctionEffect::MutableState);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit_effects(
                child,
                root,
                source,
                module_state,
                parameter_names,
                local_bindings,
                effects,
            );
        }
    }

    let mut module_state = HashSet::new();
    let root = root_node(*function);
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        collect_module_state(child, source, language, &mut module_state);
    }

    let mut parameter_names = HashSet::new();
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            let binding = match language {
                Language::TypeScript => parameter.child_by_field_name("pattern").or_else(|| {
                    (parameter.kind() == "rest_pattern")
                        .then(|| parameter.named_child(0))
                        .flatten()
                }),
                Language::Python => match parameter.kind() {
                    "identifier" => Some(parameter),
                    "typed_parameter" => parameter.named_child(0),
                    "default_parameter" | "typed_default_parameter" => {
                        parameter.child_by_field_name("name")
                    }
                    "list_splat" | "dictionary_splat" => parameter.named_child(0),
                    _ => None,
                },
            };
            if let Some(binding) = binding {
                collect_binding_names(binding, source, &mut parameter_names);
            }
        }
    }

    let mut local_bindings = HashMap::new();
    local_bindings
        .entry(function.id())
        .or_insert_with(HashSet::new)
        .extend(parameter_names.iter().cloned());
    if let Some(body) = function.child_by_field_name("body") {
        collect_local_bindings(body, *function, source, language, &mut local_bindings);
    }

    let mut effects = Vec::new();
    visit_effects(
        *function,
        *function,
        source,
        &module_state,
        &parameter_names,
        &local_bindings,
        &mut effects,
    );
    effects.sort_by_key(|effect| match effect {
        FunctionEffect::Randomness => 0,
        FunctionEffect::Time => 1,
        FunctionEffect::Timer => 2,
        FunctionEffect::Io => 3,
        FunctionEffect::MutableState => 4,
    });
    effects.dedup();
    effects
}

fn contains_identifier(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    source.match_indices(name).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let after = source[start + name.len()..].chars().next();
        !before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && !after.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

fn predicate_literal(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<serde_json::Value> {
    let raw = text(&node, source).trim();
    match node.kind() {
        "true" => return Some(serde_json::Value::Bool(true)),
        "false" => return Some(serde_json::Value::Bool(false)),
        "none" | "null" => return Some(serde_json::Value::Null),
        "undefined" => return None,
        "integer" | "float" | "number" => {
            let normalized = raw.replace('_', "");
            if let Ok(value) = normalized.parse::<i64>() {
                return Some(serde_json::json!(value));
            }
            if let Ok(value) = normalized.parse::<f64>() {
                return serde_json::Number::from_f64(value).map(serde_json::Value::Number);
            }
            return None;
        }
        "string" | "string_fragment" | "template_string" => {}
        _ => return None,
    }
    if raw.starts_with('"') {
        return serde_json::from_str(raw).ok();
    }
    let unquoted = raw
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            raw.strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
        })?;
    Some(serde_json::Value::String(
        unquoted
            .replace("\\'", "'")
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\t", "\t"),
    ))
}

fn predicate_boundary_values(value: serde_json::Value) -> Vec<serde_json::Value> {
    let Some(number) = value.as_number() else {
        return vec![value];
    };
    if let Some(integer) = number.as_i64() {
        return [
            integer.checked_sub(1),
            Some(integer),
            integer.checked_add(1),
        ]
        .into_iter()
        .flatten()
        .map(|candidate| serde_json::json!(candidate))
        .collect();
    }
    if let Some(unsigned) = number.as_u64() {
        return [
            unsigned.checked_sub(1),
            Some(unsigned),
            unsigned.checked_add(1),
        ]
        .into_iter()
        .flatten()
        .map(|candidate| serde_json::json!(candidate))
        .collect();
    }
    let value = number.as_f64().unwrap_or(0.0);
    [value - 1.0, value, value + 1.0]
        .into_iter()
        .filter_map(serde_json::Number::from_f64)
        .map(serde_json::Value::Number)
        .collect()
}

fn is_typeof_parameter_expression(
    node: tree_sitter::Node<'_>,
    parameter: &str,
    source: &[u8],
) -> bool {
    let raw = text(&node, source).trim();
    let Some(operand) = raw.strip_prefix("typeof") else {
        return false;
    };
    if !operand
        .chars()
        .next()
        .is_some_and(|character| character.is_whitespace() || character == '(')
    {
        return false;
    }
    let mut operand = operand.trim();
    while let Some(inner) = operand
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    {
        operand = inner.trim();
    }
    predicate_property_path(operand, parameter).is_some()
}

fn collect_parameter_predicate_literals(
    node: tree_sitter::Node<'_>,
    parameter: &str,
    source: &[u8],
    values: &mut Vec<serde_json::Value>,
) {
    if matches!(node.kind(), "binary_expression" | "comparison_operator") {
        let left = node
            .child_by_field_name("left")
            .or_else(|| node.named_child(0));
        let right = node
            .child_by_field_name("right")
            .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)));
        if left.is_some_and(|child| is_typeof_parameter_expression(child, parameter, source))
            || right.is_some_and(|child| is_typeof_parameter_expression(child, parameter, source))
        {
            return;
        }
    }
    if let Some(value) = predicate_literal(node, source) {
        values.extend(predicate_boundary_values(value));
        return;
    }
    let raw = text(&node, source).trim();
    if raw == "[]" {
        values.push(serde_json::json!([]));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_parameter_predicate_literals(child, parameter, source, values);
    }
}

fn collect_predicate_literals(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    values: &mut Vec<serde_json::Value>,
) {
    if let Some(value) = predicate_literal(node, source) {
        values.extend(predicate_boundary_values(value));
        return;
    }
    let raw = text(&node, source).trim();
    if raw == "[]" {
        values.push(serde_json::json!([]));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_predicate_literals(child, source, values);
    }
}

fn predicate_expression<'tree>(
    node: tree_sitter::Node<'tree>,
    source: &[u8],
) -> Option<tree_sitter::Node<'tree>> {
    match node.kind() {
        "if_statement" | "while_statement" | "conditional_expression" => node
            .child_by_field_name("condition")
            .or_else(|| node.child_by_field_name("condition_clause")),
        "comparison_operator" => Some(node),
        "binary_expression" => {
            let expression = text(&node, source);
            [
                "===",
                "!==",
                "==",
                "!=",
                "<=",
                ">=",
                "<",
                ">",
                " in ",
                ".includes(",
            ]
            .iter()
            .any(|operator| expression.contains(operator))
            .then_some(node)
        }
        "switch_statement" | "match_statement" => Some(node),
        _ => None,
    }
}

fn predicate_property_path(raw: &str, parameter: &str) -> Option<Vec<String>> {
    let mut rest = raw.trim().strip_prefix(parameter)?;
    if rest.is_empty() {
        return Some(Vec::new());
    }
    if rest
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }

    let mut path = Vec::new();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if let Some(after_dot) = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.')) {
            let length = after_dot
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                })
                .map(char::len_utf8)
                .sum::<usize>();
            if length == 0 {
                return None;
            }
            path.push(after_dot[..length].to_string());
            rest = &after_dot[length..];
            continue;
        }
        if let Some(after_bracket) = rest.strip_prefix('[') {
            let closing = after_bracket.find(']')?;
            let key = after_bracket[..closing].trim();
            let key = serde_json::from_str::<String>(key).ok().or_else(|| {
                key.strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
                    .map(str::to_string)
            })?;
            path.push(key);
            rest = &after_bracket[closing + 1..];
            continue;
        }
        return None;
    }
    Some(path)
}

fn collect_property_predicate_seeds(
    node: tree_sitter::Node<'_>,
    parameter: &str,
    source: &[u8],
    predicate_line: usize,
    seeds: &mut Vec<PredicateSeed>,
) {
    if matches!(node.kind(), "binary_expression" | "comparison_operator") {
        let left = node
            .child_by_field_name("left")
            .or_else(|| node.named_child(0));
        let right = node
            .child_by_field_name("right")
            .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)));
        for (access, values_node) in [(left, right), (right, left)] {
            let (Some(access), Some(values_node)) = (access, values_node) else {
                continue;
            };
            let Some(property_path) = predicate_property_path(text(&access, source), parameter)
            else {
                continue;
            };
            if property_path.is_empty() {
                continue;
            }
            let mut values = Vec::new();
            collect_predicate_literals(values_node, source, &mut values);
            for value in values {
                seeds.push(PredicateSeed {
                    parameter: parameter.to_string(),
                    property_path: property_path.clone(),
                    value,
                    line: predicate_line,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_property_predicate_seeds(child, parameter, source, predicate_line, seeds);
    }
}

fn extract_predicate_seeds(
    function: tree_sitter::Node<'_>,
    params: &[ParamInfo],
    source: &[u8],
) -> Vec<PredicateSeed> {
    let names = params
        .iter()
        .filter(|param| !param.is_variadic())
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();
    let mut seeds = Vec::new();
    let mut grouped_predicate_ranges = Vec::<(usize, usize)>::new();
    let mut stack = function
        .child_by_field_name("body")
        .into_iter()
        .collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        if grouped_predicate_ranges
            .iter()
            .any(|(start, end)| node.start_byte() >= *start && node.end_byte() <= *end)
        {
            continue;
        }
        if matches!(
            node.kind(),
            "function_definition"
                | "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
        ) {
            continue;
        }
        if let Some(expression) = predicate_expression(node, source) {
            let expression_text = text(&expression, source);
            let predicate_line = expression.start_position().row + 1;
            if expression.id() != node.id() {
                grouped_predicate_ranges.push((expression.start_byte(), expression.end_byte()));
            } else if matches!(node.kind(), "binary_expression" | "comparison_operator") {
                grouped_predicate_ranges.push((node.start_byte(), node.end_byte()));
            }
            for name in &names {
                if !contains_identifier(expression_text, name) {
                    continue;
                }
                let mut candidates = Vec::new();
                collect_property_predicate_seeds(
                    expression,
                    name,
                    source,
                    predicate_line,
                    &mut candidates,
                );
                if candidates.is_empty() {
                    let mut values = Vec::new();
                    collect_parameter_predicate_literals(expression, name, source, &mut values);
                    candidates.extend(values.into_iter().map(|value| PredicateSeed {
                        parameter: (*name).to_string(),
                        property_path: Vec::new(),
                        value,
                        line: predicate_line,
                    }));
                }
                for seed in candidates {
                    if !seeds.iter().any(|existing: &PredicateSeed| {
                        existing.parameter == seed.parameter
                            && existing.property_path == seed.property_path
                            && existing.value == seed.value
                            && existing.line == seed.line
                    }) {
                        seeds.push(seed);
                    }
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    seeds
}

// ── Python ──────────────────────────────────────────────────────────────────

fn python_returned_callable_names(callable: tree_sitter::Node<'_>, source: &[u8]) -> Vec<String> {
    fn returned_expression_names(node: tree_sitter::Node<'_>, source: &[u8]) -> Vec<String> {
        if node.kind() == "identifier" {
            return vec![text(&node, source).trim().to_string()];
        }
        if node.kind() != "dictionary" {
            return Vec::new();
        }

        let mut names = Vec::new();
        let mut cursor = node.walk();
        for pair in node
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "pair")
        {
            let Some(key) = pair.child_by_field_name("key") else {
                continue;
            };
            let Some(value) = pair.child_by_field_name("value") else {
                continue;
            };
            if value.kind() != "identifier" {
                continue;
            }
            let value_name = text(&value, source).trim();
            let key_text = text(&key, source).trim();
            let key_name = key_text
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    key_text
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                });
            if key_name == Some(value_name) {
                names.push(value_name.to_string());
            }
        }
        names
    }

    fn collect_returns(
        node: tree_sitter::Node<'_>,
        root_id: usize,
        source: &[u8],
        names: &mut Vec<String>,
    ) {
        if node.id() != root_id && matches!(node.kind(), "function_definition" | "lambda") {
            return;
        }
        if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                for name in returned_expression_names(child, source) {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_returns(child, root_id, source, names);
        }
    }

    fn collect_nested_functions(
        node: tree_sitter::Node<'_>,
        root_id: usize,
        source: &[u8],
        names: &mut Vec<String>,
    ) {
        if node.id() != root_id && node.kind() == "function_definition" {
            if let Some(name) = node.child_by_field_name("name") {
                names.push(text(&name, source).trim().to_string());
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_nested_functions(child, root_id, source, names);
        }
    }

    let mut nested = Vec::new();
    collect_nested_functions(callable, callable.id(), source, &mut nested);
    let mut names = Vec::new();
    collect_returns(callable, callable.id(), source, &mut names);
    names.retain(|name| nested.contains(name));
    names
}

fn visit_python(
    node: &tree_sitter::Node,
    source: &[u8],
    functions: &mut Vec<FunctionInfo>,
    classes: &mut Vec<ClassInfo>,
    imports: &mut Vec<ImportInfo>,
    func_depth: usize,
) {
    let mut child_depth = func_depth;
    match node.kind() {
        "function_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let is_method = has_self_or_cls_first_param(node, source);
            let is_exported = !is_method && func_depth == 0 && !name.starts_with('_');
            let params = extract_python_params(node, source);
            let return_type = node
                .child_by_field_name("return_type")
                .map(|n| text(&n, source).to_string());
            let metrics = callable_complexity(node, &Language::Python, source);
            let predicate_seeds = extract_predicate_seeds(*node, &params, source);
            let returned_callables = python_returned_callable_names(*node, source);
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                type_parameters: vec![],
                type_parameter_constraints: BTreeMap::new(),
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method,
                is_nested: func_depth > 0,
                is_exported,
                declared_properties: vec![],
                predicate_seeds,
                effects: function_effects(node, source, &Language::Python),
                invocation_target: None,
                returned_callables,
            });
            child_depth = func_depth + 1;
        }
        "class_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let bases = extract_python_bases(node, source);
            let fields = extract_python_class_fields(node, source);
            classes.push(ClassInfo {
                name,
                bases,
                line: node.start_position().row + 1,
                fields,
            });
        }
        "import_statement" | "import_from_statement" => {
            imports.push(ImportInfo {
                statement: text(node, source).to_string(),
                line: node.start_position().row + 1,
            });
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_python(&child, source, functions, classes, imports, child_depth);
    }
}

fn extract_python_params(func: &tree_sitter::Node, source: &[u8]) -> Vec<ParamInfo> {
    let params_node = match func.child_by_field_name("parameters") {
        Some(n) => n,
        None => return vec![],
    };

    let mut params = vec![];
    let mut cursor = params_node.walk();
    let mut keyword_only = false;

    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            // Bare `*` separator — all following params are keyword-only.
            "keyword_separator" => {
                keyword_only = true;
            }
            "identifier" => {
                let name = text(&child, source);
                if name != "self" && name != "cls" {
                    params.push(ParamInfo {
                        name: name.to_string(),
                        type_annotation: None,
                        default_value: None,
                        keyword_only,
                        optional: false,
                        variadic: None,
                    });
                }
            }
            "typed_parameter" => {
                let parameter = child.named_child(0);
                let variadic = parameter.and_then(|node| match node.kind() {
                    "list_splat_pattern" => Some(VariadicKind::Positional),
                    "dictionary_splat_pattern" => Some(VariadicKind::Keyword),
                    _ => None,
                });
                let name = parameter
                    .map(|n| text(&n, source))
                    .unwrap_or("")
                    .trim_start_matches('*');
                if name != "self" && name != "cls" {
                    let type_ann = child
                        .child_by_field_name("type")
                        .map(|n| text(&n, source).to_string());
                    params.push(ParamInfo {
                        name: name.to_string(),
                        type_annotation: type_ann,
                        default_value: None,
                        keyword_only,
                        optional: variadic.is_some(),
                        variadic,
                    });
                }
                if variadic.is_some() {
                    keyword_only = true;
                }
            }
            "default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| text(&n, source))
                    .unwrap_or("");
                let value = child
                    .child_by_field_name("value")
                    .map(|n| text(&n, source).to_string());
                params.push(ParamInfo {
                    name: name.to_string(),
                    type_annotation: None,
                    default_value: value,
                    keyword_only,
                    optional: true,
                    variadic: None,
                });
            }
            "typed_default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| text(&n, source))
                    .unwrap_or("");
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|n| text(&n, source).to_string());
                let value = child
                    .child_by_field_name("value")
                    .map(|n| text(&n, source).to_string());
                params.push(ParamInfo {
                    name: name.to_string(),
                    type_annotation: type_ann,
                    default_value: value,
                    keyword_only,
                    optional: true,
                    variadic: None,
                });
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                let raw_name = text(&child, source);
                let is_keyword = child.kind() == "dictionary_splat_pattern";
                params.push(ParamInfo {
                    name: raw_name.trim_start_matches('*').to_string(),
                    type_annotation: child
                        .named_child(0)
                        .and_then(|node| node.child_by_field_name("type"))
                        .map(|node| text(&node, source).to_string()),
                    default_value: None,
                    keyword_only: is_keyword,
                    optional: true,
                    variadic: Some(if is_keyword {
                        VariadicKind::Keyword
                    } else {
                        VariadicKind::Positional
                    }),
                });
                keyword_only = true;
            }
            _ => {}
        }
    }

    params
}

/// Extract fields from a Python class body (dataclass-style type-annotated fields).
#[allow(clippy::single_match)]
fn extract_python_class_fields(class_node: &tree_sitter::Node, source: &[u8]) -> Vec<FieldInfo> {
    let body = match class_node.child_by_field_name("body") {
        Some(n) => n,
        None => return vec![],
    };

    let mut fields = vec![];
    let mut cursor = body.walk();

    for child in body.named_children(&mut cursor) {
        match child.kind() {
            // `x: int` — type annotation without default
            "expression_statement" => {
                if let Some(inner) = child.named_child(0) {
                    if inner.kind() == "type" {
                        // `type` node wraps the annotation: `x: int`
                        let full = text(&inner, source);
                        if let Some(colon_pos) = full.find(':') {
                            let name = full[..colon_pos].trim();
                            let type_ann = full[colon_pos + 1..].trim();
                            if !name.is_empty() && !name.contains(' ') {
                                fields.push(FieldInfo {
                                    name: name.to_string(),
                                    type_annotation: if type_ann.is_empty() {
                                        None
                                    } else {
                                        Some(type_ann.to_string())
                                    },
                                    optional: false,
                                    has_default: false,
                                });
                            }
                        }
                    } else if inner.kind() == "assignment" {
                        // An annotated assignment may have no default; punctuation
                        // inside its annotation is not an assignment operator.
                        if let (Some(left), Some(annotation)) = (
                            inner.child_by_field_name("left"),
                            inner.child_by_field_name("type"),
                        ) {
                            let name = text(&left, source);
                            let type_ann = text(&annotation, source);
                            if !name.is_empty() && !name.contains(' ') {
                                fields.push(FieldInfo {
                                    name: name.to_string(),
                                    type_annotation: if type_ann.is_empty() {
                                        None
                                    } else {
                                        Some(type_ann.to_string())
                                    },
                                    optional: false,
                                    has_default: inner.child_by_field_name("right").is_some(),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fields
}

fn extract_python_bases(class: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let args = match class.child_by_field_name("superclasses") {
        Some(n) => n,
        None => return vec![],
    };

    let mut bases = vec![];
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        bases.push(text(&child, source).to_string());
    }
    bases
}

// ── TypeScript ──────────────────────────────────────────────────────────────

fn visit_typescript(
    node: &tree_sitter::Node,
    source: &[u8],
    functions: &mut Vec<FunctionInfo>,
    classes: &mut Vec<ClassInfo>,
    aliases: &mut Vec<TypeAliasInfo>,
    imports: &mut Vec<ImportInfo>,
    func_depth: usize,
) {
    let mut child_depth = func_depth;
    match node.kind() {
        "function_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let params = extract_ts_params(node, source);
            let return_type = node
                .child_by_field_name("return_type")
                .map(|n| type_text(&n, source));
            let metrics = callable_complexity(node, &Language::TypeScript, source);
            let returned_callables = extract_ts_returned_object_callables(node, source);
            let predicate_seeds = extract_predicate_seeds(*node, &params, source);
            let (type_parameters, type_parameter_constraints) =
                extract_ts_type_parameters(node, source);
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                type_parameters,
                type_parameter_constraints,
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method: false,
                is_nested: func_depth > 0,
                is_exported: ts_is_exported(node),
                declared_properties: vec![],
                predicate_seeds,
                effects: function_effects(node, source, &Language::TypeScript),
                invocation_target: None,
                returned_callables,
            });
            child_depth = func_depth + 1;
        }
        "method_definition" => {
            let method_name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let (name, is_exported, invocation_target) =
                if let Some((qualified_name, call_target)) =
                    ts_exported_class_method_surface(node, source)
                {
                    (qualified_name, true, call_target)
                } else if node
                    .parent()
                    .is_some_and(|parent| parent.kind() == "object")
                {
                    if !ts_is_returned_object_method(*node) {
                        let mut cursor = node.walk();
                        for child in node.named_children(&mut cursor) {
                            visit_typescript(
                                &child,
                                source,
                                functions,
                                classes,
                                aliases,
                                imports,
                                child_depth,
                            );
                        }
                        return;
                    }
                    (method_name.clone(), false, None)
                } else {
                    (method_name.clone(), false, None)
                };
            let params = extract_ts_params(node, source);
            let return_type = node
                .child_by_field_name("return_type")
                .map(|n| type_text(&n, source));
            let metrics = callable_complexity(node, &Language::TypeScript, source);
            let returned_callables = extract_ts_returned_object_callables(node, source);
            let predicate_seeds = extract_predicate_seeds(*node, &params, source);
            let (type_parameters, type_parameter_constraints) =
                extract_ts_type_parameters(node, source);
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                type_parameters,
                type_parameter_constraints,
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method: true,
                is_nested: func_depth > 0,
                is_exported,
                declared_properties: vec![],
                predicate_seeds,
                effects: function_effects(node, source, &Language::TypeScript),
                invocation_target,
                returned_callables,
            });
            child_depth = func_depth + 1;
        }
        "variable_declarator" => {
            // Detect function-valued bindings: const foo = (...) => ... / function (...) { ... }
            if let Some(value) = node.child_by_field_name("value") {
                if matches!(value.kind(), "arrow_function" | "function_expression") {
                    let name = node
                        .child_by_field_name("name")
                        .map(|n| text(&n, source).to_string())
                        .unwrap_or_default();
                    let params = extract_ts_params(&value, source);
                    let return_type = value
                        .child_by_field_name("return_type")
                        .map(|n| type_text(&n, source));
                    let metrics = callable_complexity(&value, &Language::TypeScript, source);
                    let returned_callables = extract_ts_returned_object_callables(&value, source);
                    let predicate_seeds = extract_predicate_seeds(value, &params, source);
                    let (type_parameters, type_parameter_constraints) =
                        extract_ts_type_parameters(&value, source);
                    functions.push(FunctionInfo {
                        name,
                        params,
                        return_type,
                        type_parameters,
                        type_parameter_constraints,
                        line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        complexity: metrics.cyclomatic,
                        cognitive_complexity: metrics.cognitive,
                        max_nesting_depth: metrics.max_nesting_depth,
                        complexity_breakdown: metrics.breakdown,
                        is_method: false,
                        is_nested: func_depth > 0,
                        is_exported: ts_is_exported(node),
                        declared_properties: vec![],
                        predicate_seeds,
                        effects: function_effects(&value, source, &Language::TypeScript),
                        invocation_target: None,
                        returned_callables,
                    });
                    child_depth = func_depth + 1;
                } else if value.kind() == "object" && ts_is_exported(node) {
                    let base_name = node
                        .child_by_field_name("name")
                        .map(|n| text(&n, source).to_string())
                        .unwrap_or_default();
                    if !base_name.is_empty() {
                        collect_exported_object_callables(
                            &base_name, &value, source, functions, func_depth,
                        );
                    }
                } else if value.kind() == "call_expression" && ts_is_exported(node) {
                    let base_name = node
                        .child_by_field_name("name")
                        .map(|n| text(&n, source).to_string())
                        .unwrap_or_default();
                    if !base_name.is_empty() {
                        collect_exported_container_callables(&base_name, &value, source, functions);
                    }
                }
            }
        }
        "class_declaration" | "interface_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let fields = extract_ts_interface_fields(node, source);
            classes.push(ClassInfo {
                name,
                bases: vec![],
                line: node.start_position().row + 1,
                fields,
            });
        }
        "type_alias_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            if let Some(value) = node
                .child_by_field_name("value")
                .or_else(|| node.child_by_field_name("type"))
            {
                if !name.is_empty() {
                    aliases.push(TypeAliasInfo {
                        name: name.clone(),
                        type_annotation: text(&value, source).trim().to_string(),
                        line: node.start_position().row + 1,
                    });
                }

                // Extract `type Foo = { bar: string; baz?: number }` as ClassInfo
                let object_type = if value.kind() == "object_type" {
                    Some(value)
                } else {
                    let mut cursor = value.walk();
                    let found = value
                        .named_children(&mut cursor)
                        .find(|child| child.kind() == "object_type");
                    found
                };

                if let Some(object_type) = object_type {
                    let fields = extract_ts_object_type_fields(&object_type, source);
                    if !name.is_empty() {
                        classes.push(ClassInfo {
                            name,
                            bases: vec![],
                            line: node.start_position().row + 1,
                            fields,
                        });
                    }
                }
            }
        }
        "enum_declaration" => {
            if let Some(alias) = parse_typescript_enum_alias(node, source) {
                aliases.push(alias);
            }
        }
        "import_statement" => {
            imports.push(ImportInfo {
                statement: text(node, source).to_string(),
                line: node.start_position().row + 1,
            });
        }
        "export_statement" => {
            let statement = text(node, source);
            if statement.contains(" from ") {
                imports.push(ImportInfo {
                    statement: statement.to_string(),
                    line: node.start_position().row + 1,
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_typescript(
            &child,
            source,
            functions,
            classes,
            aliases,
            imports,
            child_depth,
        );
    }
}

fn parse_typescript_enum_alias(node: &tree_sitter::Node, source: &[u8]) -> Option<TypeAliasInfo> {
    let raw = text(node, source);
    let enum_idx = raw.find("enum")?;
    let rest = raw[enum_idx + "enum".len()..].trim_start();
    let name = rest
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
        .next()
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return None;
    }

    let body_start = raw.find('{')?;
    let body_end = raw.rfind('}')?;
    if body_end <= body_start {
        return None;
    }
    let body = &raw[body_start + 1..body_end];
    let mut literals = Vec::new();
    let mut next_numeric = 0i64;
    for member in split_typescript_top_level_commas(body) {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        let literal = if let Some((_, initializer)) = member.split_once('=') {
            let initializer = initializer.trim();
            let literal = typescript_literal_expr(initializer)?;
            if let Ok(value) = literal.parse::<i64>() {
                next_numeric = value.saturating_add(1);
            } else {
                next_numeric = next_numeric.saturating_add(1);
            }
            literal
        } else {
            let literal = next_numeric.to_string();
            next_numeric = next_numeric.saturating_add(1);
            literal
        };
        literals.push(literal);
    }
    if literals.is_empty() {
        return None;
    }

    Some(TypeAliasInfo {
        name: name.to_string(),
        type_annotation: literals.join(" | "),
        line: node.start_position().row + 1,
    })
}

fn apply_typescript_const_tuple_alias_domains(
    root: &tree_sitter::Node,
    source: &[u8],
    aliases: &mut [TypeAliasInfo],
) {
    let domains = collect_typescript_const_tuple_domains(root, source);
    if domains.is_empty() {
        return;
    }

    for alias in aliases {
        let Some(tuple_name) = typescript_typeof_tuple_name(&alias.type_annotation) else {
            continue;
        };
        if let Some(domain) = domains.get(tuple_name.as_str()) {
            alias.type_annotation = domain.clone();
        }
    }
}

fn apply_typescript_keyof_alias_domains(
    root: &tree_sitter::Node,
    source: &[u8],
    aliases: &mut [TypeAliasInfo],
) {
    let domains = collect_typescript_keyof_domains(root, source);
    if domains.is_empty() {
        return;
    }

    for alias in aliases {
        let Some(value_name) = typescript_keyof_typeof_name(&alias.type_annotation) else {
            continue;
        };
        if let Some(domain) = domains.get(value_name.as_str()) {
            alias.type_annotation = domain.clone();
        }
    }
}

fn collect_typescript_keyof_domains(
    root: &tree_sitter::Node,
    source: &[u8],
) -> HashMap<String, String> {
    fn visit(node: &tree_sitter::Node, source: &[u8], domains: &mut HashMap<String, String>) {
        if node.kind() == "variable_declarator" {
            if let (Some(name), Some(value)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) {
                let name = text(&name, source).trim();
                let value = text(&value, source).trim();
                if let Some(domain) = parse_typescript_keyof_source_domain(value, domains) {
                    domains.insert(name.to_string(), domain);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(&child, source, domains);
        }
    }

    let mut domains = HashMap::new();
    visit(root, source, &mut domains);
    domains
}

fn parse_typescript_keyof_source_domain(
    value: &str,
    domains: &HashMap<String, String>,
) -> Option<String> {
    if value.contains(".enum(") || value.starts_with("z.enum(") {
        let start = value.find('[')?;
        let end = matching_bracket_index(value, start, '[', ']')?;
        let literals = split_typescript_top_level_commas(&value[start + 1..end])
            .into_iter()
            .filter_map(|item| typescript_literal_expr(item.trim()))
            .collect::<Vec<_>>();
        return (!literals.is_empty()).then(|| literals.join(" | "));
    }

    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    if let Some(source_name) = compact.strip_suffix(".enum") {
        if let Some(domain) = domains.get(source_name) {
            return Some(domain.clone());
        }
    }

    let start = value.find('{')?;
    let end = matching_bracket_index(value, start, '{', '}')?;
    let keys = split_typescript_top_level_commas(&value[start + 1..end])
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() || entry.starts_with("...") {
                return None;
            }
            let raw_key = entry
                .split_once(':')
                .map(|(key, _)| key)
                .unwrap_or(entry)
                .trim();
            let key = raw_key.trim_matches(&['"', '\'', '`'][..]);
            (!key.is_empty())
                .then(|| serde_json::to_string(key).expect("object property key serializes"))
        })
        .collect::<Vec<_>>();
    (!keys.is_empty()).then(|| keys.join(" | "))
}

fn typescript_keyof_typeof_name(type_annotation: &str) -> Option<String> {
    let compact: String = type_annotation
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let raw = compact.strip_prefix("keyoftypeof")?;
    let name = raw.split('.').next()?;
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return None;
    }
    Some(name.to_string())
}

fn collect_typescript_const_tuple_domains(
    root: &tree_sitter::Node,
    source: &[u8],
) -> HashMap<String, String> {
    fn visit(node: &tree_sitter::Node, source: &[u8], domains: &mut HashMap<String, String>) {
        if node.kind() == "variable_declarator" {
            if let (Some(name), Some(value)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) {
                let name = text(&name, source).trim();
                let value = text(&value, source);
                if let Some(domain) = parse_typescript_const_tuple_domain(value) {
                    domains.insert(name.to_string(), domain);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(&child, source, domains);
        }
    }

    let mut domains = HashMap::new();
    visit(root, source, &mut domains);
    domains
}

fn parse_typescript_const_tuple_domain(value: &str) -> Option<String> {
    if !value.contains("as const") {
        return None;
    }
    let start = value.find('[')?;
    let end = matching_bracket_index(value, start, '[', ']')?;
    let inner = &value[start + 1..end];
    let literals: Vec<String> = split_typescript_top_level_commas(inner)
        .into_iter()
        .filter_map(|item| typescript_literal_expr(item.trim()))
        .collect();
    if literals.is_empty() {
        None
    } else {
        Some(literals.join(" | "))
    }
}

fn typescript_typeof_tuple_name(type_annotation: &str) -> Option<String> {
    let compact: String = type_annotation
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let raw = compact
        .strip_prefix("typeof")
        .and_then(|rest| rest.strip_suffix("[number]"))
        .or_else(|| {
            compact
                .strip_prefix("(typeof")
                .and_then(|rest| rest.strip_suffix(")[number]"))
        })?;
    if raw.is_empty()
        || !raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return None;
    }
    Some(raw.to_string())
}

fn split_typescript_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0usize;

    for (idx, ch) in text.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '[' | '{' | '(' | '<' => depth += 1,
            ']' | '}' | ')' | '>' => depth -= 1,
            ',' if depth == 0 => {
                let part = text[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn matching_bracket_index(text: &str, start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in text[start..].char_indices() {
        let absolute = start + idx;
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            _ if ch == open => depth += 1,
            _ if ch == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(absolute);
                }
            }
            _ => {}
        }
    }
    None
}

fn typescript_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches(';').trim();
    if let Some(quoted) = quoted_string_literal_expr(trimmed) {
        return Some(quoted);
    }
    match trimmed {
        "true" | "false" | "null" | "undefined" => Some(trimmed.to_string()),
        _ => numeric_literal_expr(trimmed),
    }
}

fn quoted_string_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    if !trimmed.ends_with(quote) || trimmed.len() < 2 {
        return None;
    }
    let inner = &trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()];
    serde_json::to_string(inner).ok()
}

fn numeric_literal_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|ch| !(ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | '_')))
    {
        return None;
    }
    let normalized = trimmed.replace('_', "");
    normalized.parse::<f64>().ok()?;
    Some(normalized)
}

fn ts_exported_class_method_surface(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<(String, Option<String>)> {
    let parent = node.parent()?;
    if parent.kind() != "class_body" {
        return None;
    }
    let class_node = parent.parent()?;
    if class_node.kind() != "class_declaration" || !ts_is_exported(&class_node) {
        return None;
    }
    if !ts_class_has_zero_arg_constructor(&class_node, source) {
        return None;
    }

    let class_name = class_node
        .child_by_field_name("name")
        .map(|n| text(&n, source))?;
    let method_name_node = node.child_by_field_name("name")?;
    let method_name = text(&method_name_node, source);
    if method_name.is_empty() || method_name == "constructor" || method_name.starts_with('#') {
        return None;
    }
    let prefix = std::str::from_utf8(&source[node.start_byte()..method_name_node.start_byte()])
        .unwrap_or_default();
    let is_accessor = prefix
        .split_whitespace()
        .any(|token| matches!(token, "get" | "set"));

    Some((
        format!("{class_name}#{method_name}"),
        (!is_accessor).then(|| format!("(new {class_name}()).{method_name}")),
    ))
}

fn ts_class_has_zero_arg_constructor(class_node: &tree_sitter::Node, source: &[u8]) -> bool {
    let Some(body) = class_node.child_by_field_name("body") else {
        return true;
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() != "method_definition" {
            continue;
        }
        let name = child
            .child_by_field_name("name")
            .map(|n| text(&n, source))
            .unwrap_or_default();
        if name != "constructor" {
            continue;
        }
        let params = extract_ts_params(&child, source);
        return params.is_empty();
    }
    true
}

fn collect_exported_object_callables(
    base_name: &str,
    object: &tree_sitter::Node,
    source: &[u8],
    functions: &mut Vec<FunctionInfo>,
    func_depth: usize,
) {
    collect_ts_surfaced_object_callables(
        base_name,
        base_name,
        object,
        source,
        functions,
        func_depth > 0,
    );
}

fn collect_exported_container_callables(
    base_name: &str,
    value: &tree_sitter::Node,
    source: &[u8],
    functions: &mut Vec<FunctionInfo>,
) {
    let Some(object) = ts_supported_container_return_object(*value, source) else {
        return;
    };
    let invocation_root = format!("{base_name}.getState()");
    collect_ts_surfaced_object_callables(
        base_name,
        &invocation_root,
        &object,
        source,
        functions,
        false,
    );
}

fn collect_ts_surfaced_object_callables(
    base_name: &str,
    invocation_root: &str,
    object: &tree_sitter::Node,
    source: &[u8],
    functions: &mut Vec<FunctionInfo>,
    is_nested: bool,
) {
    let mut cursor = object.walk();
    for child in object.named_children(&mut cursor) {
        match child.kind() {
            "method_definition" => {
                let Some(method_name) = child
                    .child_by_field_name("name")
                    .and_then(|n| ts_property_name(&n, source))
                else {
                    continue;
                };
                push_ts_surfaced_callable(
                    functions,
                    base_name,
                    invocation_root,
                    &method_name,
                    &child,
                    &child,
                    source,
                    is_nested,
                );
            }
            "pair" => {
                let Some(key_node) = child.child_by_field_name("key") else {
                    continue;
                };
                let Some(value_node) = child.child_by_field_name("value") else {
                    continue;
                };
                if !matches!(value_node.kind(), "arrow_function" | "function_expression") {
                    continue;
                }
                let Some(method_name) = ts_property_name(&key_node, source) else {
                    continue;
                };
                push_ts_surfaced_callable(
                    functions,
                    base_name,
                    invocation_root,
                    &method_name,
                    &child,
                    &value_node,
                    source,
                    is_nested,
                );
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_ts_surfaced_callable(
    functions: &mut Vec<FunctionInfo>,
    base_name: &str,
    invocation_root: &str,
    method_name: &str,
    property_node: &tree_sitter::Node,
    callable_node: &tree_sitter::Node,
    source: &[u8],
    is_nested: bool,
) {
    if method_name.is_empty() || method_name.starts_with('#') {
        return;
    }
    let params = extract_ts_params(callable_node, source);
    let return_type = callable_node
        .child_by_field_name("return_type")
        .map(|n| type_text(&n, source));
    let metrics = callable_complexity(callable_node, &Language::TypeScript, source);
    let predicate_seeds = extract_predicate_seeds(*callable_node, &params, source);
    let (type_parameters, type_parameter_constraints) =
        extract_ts_type_parameters(callable_node, source);
    functions.push(FunctionInfo {
        name: format!("{base_name}.{method_name}"),
        params,
        return_type,
        type_parameters,
        type_parameter_constraints,
        line: property_node.start_position().row + 1,
        end_line: property_node.end_position().row + 1,
        complexity: metrics.cyclomatic,
        cognitive_complexity: metrics.cognitive,
        max_nesting_depth: metrics.max_nesting_depth,
        complexity_breakdown: metrics.breakdown,
        is_method: true,
        is_nested,
        is_exported: true,
        declared_properties: vec![],
        predicate_seeds,
        effects: function_effects(callable_node, source, &Language::TypeScript),
        invocation_target: Some(format!("{invocation_root}.{method_name}")),
        returned_callables: vec![],
    });
}

fn ts_property_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let name = text(node, source)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return None;
    }
    Some(name)
}

fn ts_supported_container_return_object<'a>(
    value: tree_sitter::Node<'a>,
    source: &[u8],
) -> Option<tree_sitter::Node<'a>> {
    if value.kind() != "call_expression" || !ts_call_has_supported_container_callee(value, source) {
        return None;
    }
    ts_find_object_returning_callback_in_call(value)
}

fn ts_call_has_supported_container_callee(call: tree_sitter::Node, source: &[u8]) -> bool {
    call.child_by_field_name("function")
        .is_some_and(|function| ts_expr_contains_supported_container_callee(function, source))
}

fn ts_expr_contains_supported_container_callee(expr: tree_sitter::Node, source: &[u8]) -> bool {
    match expr.kind() {
        "identifier" | "property_identifier" => TS_SUPPORTED_CONTAINER_CALLEES
            .iter()
            .any(|candidate| text(&expr, source).trim() == *candidate),
        "member_expression" => {
            expr.child_by_field_name("property")
                .is_some_and(|property| {
                    TS_SUPPORTED_CONTAINER_CALLEES
                        .iter()
                        .any(|candidate| text(&property, source).trim() == *candidate)
                })
                || expr.child_by_field_name("object").is_some_and(|object| {
                    ts_expr_contains_supported_container_callee(object, source)
                })
        }
        _ => {
            if let Some(function) = expr.child_by_field_name("function") {
                return ts_expr_contains_supported_container_callee(function, source);
            }
            let mut cursor = expr.walk();
            let has_supported_name = expr
                .named_children(&mut cursor)
                .any(|child| ts_expr_contains_supported_container_callee(child, source));
            has_supported_name
        }
    }
}

fn ts_find_object_returning_callback_in_call(
    call: tree_sitter::Node<'_>,
) -> Option<tree_sitter::Node<'_>> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        if let Some(object) = ts_find_object_returning_callback_in_expression(arg) {
            return Some(object);
        }
    }
    None
}

fn ts_find_object_returning_callback_in_expression(
    expr: tree_sitter::Node<'_>,
) -> Option<tree_sitter::Node<'_>> {
    match expr.kind() {
        "arrow_function" | "function_expression" => ts_returned_object_node(expr),
        "call_expression" => ts_find_object_returning_callback_in_call(expr),
        _ => {
            let mut cursor = expr.walk();
            for child in expr.named_children(&mut cursor) {
                if let Some(object) = ts_find_object_returning_callback_in_expression(child) {
                    return Some(object);
                }
            }
            None
        }
    }
}

fn ts_returned_object_node(callable: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let body = callable.child_by_field_name("body")?;
    if body.kind() == "statement_block" {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            if child.kind() != "return_statement" {
                continue;
            }
            let mut stmt_cursor = child.walk();
            for stmt_child in child.named_children(&mut stmt_cursor) {
                if let Some(object) = ts_expression_object_node(stmt_child) {
                    return Some(object);
                }
            }
        }
        None
    } else {
        ts_expression_object_node(body)
    }
}

fn ts_is_returned_object_method(method: tree_sitter::Node<'_>) -> bool {
    let Some(object) = method.parent().filter(|parent| parent.kind() == "object") else {
        return false;
    };

    let mut ancestor = object.parent();
    while let Some(node) = ancestor {
        if matches!(
            node.kind(),
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        ) {
            return ts_returned_object_node(node)
                .is_some_and(|returned_object| returned_object.id() == object.id());
        }
        ancestor = node.parent();
    }
    false
}

fn ts_expression_object_node(expr: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if expr.kind() == "object" {
        return Some(expr);
    }

    let mut cursor = expr.walk();
    for child in expr.named_children(&mut cursor) {
        if let Some(object) = ts_expression_object_node(child) {
            return Some(object);
        }
    }
    None
}

fn ts_expression_is_function(expression: tree_sitter::Node<'_>) -> bool {
    match expression.kind() {
        "arrow_function" | "function_expression" => true,
        "parenthesized_expression"
        | "as_expression"
        | "satisfies_expression"
        | "type_assertion"
        | "non_null_expression" => {
            let mut cursor = expression.walk();
            let contains_function = expression
                .named_children(&mut cursor)
                .any(ts_expression_is_function);
            contains_function
        }
        _ => false,
    }
}

fn ts_is_lexical_binding_scope(node: tree_sitter::Node<'_>) -> bool {
    matches!(
        node.kind(),
        "statement_block"
            | "for_statement"
            | "for_in_statement"
            | "catch_clause"
            | "switch_case"
            | "switch_default"
    )
}

fn ts_declaration_binding_scope<'a>(
    declaration: tree_sitter::Node<'a>,
    factory_body: tree_sitter::Node<'a>,
    function_scoped: bool,
) -> tree_sitter::Node<'a> {
    if function_scoped {
        return factory_body;
    }

    let mut ancestor = declaration.parent();
    while let Some(node) = ancestor {
        if node.id() == factory_body.id() || ts_is_lexical_binding_scope(node) {
            return node;
        }
        ancestor = node.parent();
    }
    factory_body
}

fn ts_update_callable_binding(
    best: &mut Option<(usize, bool)>,
    declaration: tree_sitter::Node<'_>,
    callable: bool,
) {
    let position = declaration.start_byte();
    if best.is_none_or(|(best_position, _)| position >= best_position) {
        *best = Some((position, callable));
    }
}

fn ts_find_callable_binding_in_scope(
    node: tree_sitter::Node<'_>,
    factory_body: tree_sitter::Node<'_>,
    target_scope: tree_sitter::Node<'_>,
    reference_byte: usize,
    name: &str,
    source: &[u8],
    best: &mut Option<(usize, bool)>,
) {
    match node.kind() {
        "function_declaration" => {
            let binding_matches = node
                .child_by_field_name("name")
                .is_some_and(|binding| text(&binding, source).trim() == name);
            if binding_matches
                && ts_declaration_binding_scope(node, factory_body, false).id() == target_scope.id()
            {
                // Function declarations are initialized when their lexical scope is entered.
                ts_update_callable_binding(best, node, true);
            }
            return;
        }
        "variable_declarator" => {
            let binding_matches = node
                .child_by_field_name("name")
                .filter(|binding| binding.kind() == "identifier")
                .is_some_and(|binding| text(&binding, source).trim() == name);
            if binding_matches {
                let function_scoped = node
                    .parent()
                    .is_some_and(|declaration| declaration.kind() == "variable_declaration");
                if ts_declaration_binding_scope(node, factory_body, function_scoped).id()
                    == target_scope.id()
                {
                    let callable = node.start_byte() < reference_byte
                        && node
                            .child_by_field_name("value")
                            .is_some_and(ts_expression_is_function);
                    // A declaration after the return still shadows outer bindings, but its
                    // value is unavailable at the returned object expression.
                    ts_update_callable_binding(best, node, callable);
                }
            }
            return;
        }
        "class_declaration" | "enum_declaration" => {
            let binding_matches = node
                .child_by_field_name("name")
                .is_some_and(|binding| text(&binding, source).trim() == name);
            if binding_matches
                && ts_declaration_binding_scope(node, factory_body, false).id() == target_scope.id()
            {
                ts_update_callable_binding(best, node, false);
            }
            return;
        }
        "arrow_function" | "function_expression" | "method_definition" => return,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        ts_find_callable_binding_in_scope(
            child,
            factory_body,
            target_scope,
            reference_byte,
            name,
            source,
            best,
        );
    }
}

fn ts_shorthand_resolves_to_callable(
    callable: tree_sitter::Node<'_>,
    returned_object: tree_sitter::Node<'_>,
    name: &str,
    source: &[u8],
) -> bool {
    let Some(factory_body) = callable.child_by_field_name("body") else {
        return false;
    };

    // Resolve from the returned expression's innermost lexical scope outward. A
    // non-callable binding is conclusive and prevents a same-named outer function
    // from being mistaken for the returned value.
    let mut scopes = Vec::new();
    let mut ancestor = returned_object.parent();
    while let Some(node) = ancestor {
        if node.id() == factory_body.id() {
            scopes.push(node);
            break;
        }
        if ts_is_lexical_binding_scope(node) {
            scopes.push(node);
        }
        ancestor = node.parent();
    }

    for scope in scopes {
        let mut best = None;
        ts_find_callable_binding_in_scope(
            factory_body,
            factory_body,
            scope,
            returned_object.start_byte(),
            name,
            source,
            &mut best,
        );
        if let Some((_, is_callable)) = best {
            return is_callable;
        }
    }
    false
}

fn extract_ts_returned_object_callables(
    callable: &tree_sitter::Node,
    source: &[u8],
) -> Vec<String> {
    let Some(object) = ts_returned_object_node(*callable) else {
        return Vec::new();
    };

    let mut callables = Vec::new();
    let mut cursor = object.walk();
    for child in object.named_children(&mut cursor) {
        let name = match child.kind() {
            "method_definition" => child
                .child_by_field_name("name")
                .and_then(|name| ts_property_name(&name, source)),
            "pair" => {
                let Some(key) = child.child_by_field_name("key") else {
                    continue;
                };
                let Some(value) = child.child_by_field_name("value") else {
                    continue;
                };
                ts_expression_is_function(value)
                    .then(|| ts_property_name(&key, source))
                    .flatten()
            }
            "shorthand_property_identifier" | "property_identifier" => {
                let name = text(&child, source).trim();
                (!name.is_empty()
                    && ts_shorthand_resolves_to_callable(*callable, object, name, source))
                .then(|| name.to_string())
            }
            _ => None,
        };

        if let Some(name) = name {
            if !callables.iter().any(|existing| existing == &name) {
                callables.push(name);
            }
        }
    }
    callables
}

fn ts_is_exported(node: &tree_sitter::Node) -> bool {
    let mut current = Some(*node);
    while let Some(candidate) = current {
        if candidate.kind() == "export_statement" {
            return true;
        }
        current = candidate.parent();
    }
    false
}

fn mark_typescript_explicit_exports(
    root: &tree_sitter::Node,
    source: &[u8],
    functions: &mut [FunctionInfo],
) {
    let exported_names = collect_typescript_explicit_exports(root, source);
    if exported_names.is_empty() {
        return;
    }

    for func in functions.iter_mut() {
        if !func.is_method && !func.is_nested && exported_names.contains(&func.name) {
            func.is_exported = true;
        }
    }
}

fn collect_typescript_explicit_exports(root: &tree_sitter::Node, source: &[u8]) -> HashSet<String> {
    let mut exported_names = HashSet::new();
    let mut cursor = root.walk();

    for child in root.named_children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }

        let stmt = text(&child, source).trim().trim_end_matches(';').trim();
        let body = stmt.strip_prefix("export").unwrap_or(stmt).trim();
        if body.starts_with('{') {
            if body.contains(" from ") {
                continue;
            }
            collect_typescript_named_exports(body, &mut exported_names);
            continue;
        }

        if let Some(rest) = body.strip_prefix("default ").map(str::trim) {
            if let Some(local_name) = parse_typescript_default_export_local(rest) {
                exported_names.insert(local_name);
            }
        }
    }

    exported_names
}

fn collect_typescript_named_exports(clause: &str, names: &mut HashSet<String>) {
    let start = match clause.find('{') {
        Some(idx) => idx,
        None => return,
    };
    let end = match clause.rfind('}') {
        Some(idx) if idx > start => idx,
        _ => return,
    };
    let inner = &clause[start + 1..end];

    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let local_name = match part.split_once(" as ") {
            Some((local_name, _exported_name)) => local_name.trim(),
            None => part,
        };
        if !local_name.is_empty() && local_name != "default" {
            names.insert(local_name.to_string());
        }
    }
}

fn parse_typescript_default_export_local(rest: &str) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    let candidate = rest
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
        .next()
        .unwrap_or("")
        .trim();
    if candidate.is_empty() {
        return None;
    }
    if matches!(
        candidate,
        "function" | "class" | "async" | "const" | "let" | "var"
    ) {
        return None;
    }
    Some(candidate.to_string())
}

/// Extract fields from a TypeScript interface or class body.
fn extract_ts_interface_fields(node: &tree_sitter::Node, source: &[u8]) -> Vec<FieldInfo> {
    let body = match node.child_by_field_name("body") {
        Some(n) => n,
        None => return vec![],
    };

    let mut fields = vec![];
    let mut cursor = body.walk();

    for child in body.named_children(&mut cursor) {
        // interface properties are "property_signature", class properties are "public_field_definition"
        match child.kind() {
            "property_signature" | "public_field_definition" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| text(&n, source).to_string())
                    .unwrap_or_default();
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|n| type_text(&n, source));

                // Check for optional marker (the `?` in `items?: string[]`)
                let is_optional = text(&child, source).contains('?');

                if !name.is_empty() {
                    fields.push(FieldInfo {
                        name,
                        type_annotation: type_ann,
                        optional: is_optional,
                        has_default: false,
                    });
                }
            }
            _ => {}
        }
    }

    fields
}

/// Extract fields from a TypeScript `type Foo = { ... }` object_type node.
fn extract_ts_object_type_fields(object_type: &tree_sitter::Node, source: &[u8]) -> Vec<FieldInfo> {
    let mut fields = vec![];
    let mut cursor = object_type.walk();

    for child in object_type.named_children(&mut cursor) {
        if child.kind() == "property_signature" {
            let name = child
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let type_ann = child
                .child_by_field_name("type")
                .map(|n| type_text(&n, source));
            let is_optional = text(&child, source).contains('?');

            if !name.is_empty() {
                fields.push(FieldInfo {
                    name,
                    type_annotation: type_ann,
                    optional: is_optional,
                    has_default: false,
                });
            }
        }
    }

    fields
}

fn infer_ts_default_type(value: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match value.kind() {
        "string" | "template_string" => Some("string".into()),
        "number" => Some(
            if text(value, source).trim().ends_with('n') {
                "bigint"
            } else {
                "number"
            }
            .into(),
        ),
        "true" | "false" => Some("boolean".into()),
        "parenthesized_expression" => value
            .named_child(0)
            .and_then(|inner| infer_ts_default_type(&inner, source)),
        "unary_expression" => {
            let raw = text(value, source).trim();
            if !raw.starts_with(['-', '+']) {
                return None;
            }
            value
                .named_child(0)
                .and_then(|inner| infer_ts_default_type(&inner, source))
                .filter(|inferred| matches!(inferred.as_str(), "number" | "bigint"))
        }
        "array" => {
            let mut element_types = Vec::new();
            let mut cursor = value.walk();
            for element in value.named_children(&mut cursor) {
                let inferred = infer_ts_default_type(&element, source)?;
                if !element_types.contains(&inferred) {
                    element_types.push(inferred);
                }
            }
            match element_types.as_slice() {
                [] => Some("never[]".into()),
                [element] => Some(format!("{element}[]")),
                elements => Some(format!("Array<{}>", elements.join(" | "))),
            }
        }
        "object" => {
            let mut fields = Vec::new();
            let mut cursor = value.walk();
            for member in value.named_children(&mut cursor) {
                if member.kind() != "pair" {
                    return None;
                }
                let key = member.child_by_field_name("key")?;
                if key.kind() == "computed_property_name" {
                    return None;
                }
                let field_value = member.child_by_field_name("value")?;
                let field_type = infer_ts_default_type(&field_value, source)?;
                fields.push(format!("{}: {field_type}", text(&key, source)));
            }
            Some(format!("{{ {} }}", fields.join("; ")))
        }
        _ => None,
    }
}

fn inferred_ts_default_type(
    explicit_type: Option<String>,
    default_value: Option<&tree_sitter::Node>,
    source: &[u8],
) -> Option<String> {
    explicit_type.or_else(|| default_value.and_then(|value| infer_ts_default_type(value, source)))
}

fn extract_ts_type_parameters(
    func: &tree_sitter::Node,
    source: &[u8],
) -> (Vec<String>, BTreeMap<String, String>) {
    let Some(parameters) = func.child_by_field_name("type_parameters") else {
        return (vec![], BTreeMap::new());
    };
    let mut names = vec![];
    let mut constraints = BTreeMap::new();
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        let name = if matches!(parameter.kind(), "identifier" | "type_identifier") {
            Some(parameter)
        } else {
            parameter
                .child_by_field_name("name")
                .or_else(|| parameter.named_child(0))
        };
        let Some(name) = name else {
            continue;
        };
        let name = text(&name, source).trim();
        if name.is_empty() {
            continue;
        }
        names.push(name.to_string());
        if let Some(constraint) = parameter.child_by_field_name("constraint") {
            let constraint = text(&constraint, source)
                .trim()
                .strip_prefix("extends ")
                .unwrap_or_else(|| text(&constraint, source).trim())
                .trim();
            if !constraint.is_empty() {
                constraints.insert(name.to_string(), constraint.to_string());
            }
        }
    }
    (names, constraints)
}

fn extract_ts_params(func: &tree_sitter::Node, source: &[u8]) -> Vec<ParamInfo> {
    let params_node = match func.child_by_field_name("parameters") {
        Some(n) => n,
        None => return vec![],
    };

    let mut params = vec![];
    let mut cursor = params_node.walk();

    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            "required_parameter" | "optional_parameter" => {
                let raw_name = child
                    .child_by_field_name("pattern")
                    .map(|n| text(&n, source).to_string())
                    .unwrap_or_default();
                let variadic = raw_name.starts_with("...");
                let name = raw_name.trim_start_matches("...").to_string();
                let default_node = child.child_by_field_name("value");
                let type_ann = inferred_ts_default_type(
                    child
                        .child_by_field_name("type")
                        .map(|n| type_text(&n, source)),
                    default_node.as_ref(),
                    source,
                );
                let default_value = default_node.map(|n| text(&n, source).to_string());
                let optional =
                    child.kind() == "optional_parameter" || default_value.is_some() || variadic;
                params.push(ParamInfo {
                    name,
                    type_annotation: type_ann,
                    default_value,
                    keyword_only: false,
                    optional,
                    variadic: variadic.then_some(VariadicKind::Positional),
                });
            }
            "rest_pattern" => {
                let raw_name = child
                    .named_child(0)
                    .map(|n| text(&n, source))
                    .unwrap_or_else(|| text(&child, source));
                params.push(ParamInfo {
                    name: raw_name.trim_start_matches("...").to_string(),
                    type_annotation: child
                        .child_by_field_name("type")
                        .map(|n| type_text(&n, source)),
                    default_value: None,
                    keyword_only: false,
                    optional: true,
                    variadic: Some(VariadicKind::Positional),
                });
            }
            _ => {}
        }
    }

    params
}

// ── Import resolution ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ParsedImportBinding {
    Named {
        local_name: String,
        exported_name: String,
    },
    Namespace {
        local_name: String,
    },
    Default {
        local_name: String,
    },
    Wildcard,
    ReExportWildcard,
}

#[derive(Debug, Clone)]
struct ParsedImport {
    path: String,
    bindings: Vec<ParsedImportBinding>,
}

type ImportRequest = Option<HashSet<String>>;

#[derive(Default)]
struct ImportResolutionState {
    known_classes: HashSet<String>,
    known_aliases: HashSet<String>,
    processed_requests: HashMap<String, ImportRequest>,
}

/// Return referenced type names from the function surface that verify is about
/// to fuzz.
pub fn referenced_type_names_for_functions(functions: &[FunctionInfo]) -> HashSet<String> {
    let mut names = HashSet::new();
    for func in functions {
        for param in &func.params {
            collect_annotation_names(param.type_annotation.as_deref(), &mut names);
        }
        collect_annotation_names(func.return_type.as_deref(), &mut names);
        for constraint in func.type_parameter_constraints.values() {
            collect_annotation_names(Some(constraint), &mut names);
        }
    }
    names
}

/// Resolve relative imports from analyzed code, analyze those files, and return
/// additional named type definitions found in imported modules.
/// This allows the fuzzer to construct proper objects or expand aliases.
pub fn resolve_imported_types(
    analysis: &AnalysisResult,
    source_file: &str,
    language: &Language,
) -> ResolvedTypeInfo {
    let source_path = std::path::Path::new(source_file);
    let source_dir = match source_path.parent() {
        Some(d) => d,
        None => return ResolvedTypeInfo::default(),
    };

    // Collect known type names so we don't duplicate.
    let known_classes: HashSet<&str> = analysis.classes.iter().map(|c| c.name.as_str()).collect();
    let known_aliases: HashSet<&str> = analysis.aliases.iter().map(|a| a.name.as_str()).collect();

    let mut resolved_types = ResolvedTypeInfo::default();
    let mut resolved_paths = HashSet::new();

    for import in &analysis.imports {
        let parsed = match parse_import(&import.statement, language) {
            Some(parsed) => parsed,
            None => continue,
        };

        // Only resolve relative imports
        if !parsed.path.starts_with('.') {
            continue;
        }

        let resolved = resolve_import_file(source_dir, &parsed.path, language);
        let resolved = match resolved {
            Some(r) => r,
            None => continue,
        };

        // Avoid re-analyzing the same file
        let key = resolved.to_string_lossy().to_string();
        if !resolved_paths.insert(key) {
            continue;
        }

        let code = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let imported_context = SourceContext {
            language: *language,
            mode: if matches!(language, Language::Python) {
                SourceMode::Python
            } else {
                match resolved
                    .extension()
                    .and_then(|extension| extension.to_str())
                {
                    Some(extension)
                        if extension.eq_ignore_ascii_case("tsx")
                            || extension.eq_ignore_ascii_case("jsx") =>
                    {
                        SourceMode::Tsx
                    }
                    _ => SourceMode::TypeScript,
                }
            },
            source_file: Some(resolved.clone()),
            virtual_file_path: None,
        };
        let imported = analyze_with_context(&code, &imported_context);

        for class in imported.classes {
            if !known_classes.contains(class.name.as_str()) {
                resolved_types.classes.push(class);
            }
        }
        for alias in imported.aliases {
            if !known_aliases.contains(alias.name.as_str()) {
                resolved_types.aliases.push(alias);
            }
        }
    }

    resolved_types
}

/// Resolve only the imported type definitions reachable from the referenced
/// names that the current verify pass will exercise.
pub fn resolve_imported_types_for_names(
    analysis: &AnalysisResult,
    source_file: &str,
    language: &Language,
    referenced_names: &HashSet<String>,
) -> ResolvedTypeInfo {
    let mut state = ImportResolutionState {
        known_classes: analysis.classes.iter().map(|c| c.name.clone()).collect(),
        known_aliases: analysis.aliases.iter().map(|a| a.name.clone()).collect(),
        processed_requests: HashMap::new(),
    };

    resolve_imported_types_for_request(
        analysis,
        std::path::Path::new(source_file),
        language,
        Some(referenced_names.clone()),
        &mut state,
    )
}

fn resolve_imported_types_for_request(
    analysis: &AnalysisResult,
    source_path: &std::path::Path,
    language: &Language,
    requested_names: ImportRequest,
    state: &mut ImportResolutionState,
) -> ResolvedTypeInfo {
    let source_dir = match source_path.parent() {
        Some(d) => d,
        None => return ResolvedTypeInfo::default(),
    };

    let closure = expand_local_type_closure(analysis, requested_names.as_ref());
    let local_class_names: HashSet<&str> =
        analysis.classes.iter().map(|c| c.name.as_str()).collect();
    let local_alias_names: HashSet<&str> =
        analysis.aliases.iter().map(|a| a.name.as_str()).collect();

    let mut resolved_types = ResolvedTypeInfo::default();

    for class in &analysis.classes {
        if closure.contains(class.name.as_str()) && state.known_classes.insert(class.name.clone()) {
            resolved_types.classes.push(class.clone());
        }
    }
    for alias in &analysis.aliases {
        if closure.contains(alias.name.as_str()) && state.known_aliases.insert(alias.name.clone()) {
            resolved_types.aliases.push(alias.clone());
        }
    }

    let unresolved_names: HashSet<String> = closure
        .iter()
        .filter(|name| {
            !local_class_names.contains(name.as_str()) && !local_alias_names.contains(name.as_str())
        })
        .cloned()
        .collect();

    let mut requests_by_path: HashMap<
        String,
        (std::path::PathBuf, ImportRequest, HashMap<String, String>),
    > = HashMap::new();
    for import in &analysis.imports {
        let parsed = match parse_import(&import.statement, language) {
            Some(parsed) => parsed,
            None => continue,
        };
        let request = match request_for_import(&parsed, &unresolved_names) {
            Some(request) => request,
            None => continue,
        };

        let resolved = match resolve_import_file(source_dir, &parsed.path, language) {
            Some(path) => path,
            None => continue,
        };
        let aliases = parsed
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                ParsedImportBinding::Named {
                    local_name,
                    exported_name,
                } if local_name != exported_name && unresolved_names.contains(local_name) => {
                    Some((exported_name.clone(), local_name.clone()))
                }
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let key = resolved.to_string_lossy().to_string();
        requests_by_path
            .entry(key)
            .and_modify(|(_, existing, existing_aliases)| {
                merge_import_request(existing, &request);
                existing_aliases.extend(aliases.clone());
            })
            .or_insert((resolved, request, aliases));
    }

    for (path_key, (resolved, request, import_aliases)) in requests_by_path {
        let delta = match note_import_request(&mut state.processed_requests, &path_key, &request) {
            Some(delta) => delta,
            None => continue,
        };

        let code = match std::fs::read_to_string(&resolved) {
            Ok(code) => code,
            Err(_) => continue,
        };
        let imported_context = SourceContext {
            language: *language,
            mode: if matches!(language, Language::Python) {
                SourceMode::Python
            } else {
                match resolved
                    .extension()
                    .and_then(|extension| extension.to_str())
                {
                    Some(extension)
                        if extension.eq_ignore_ascii_case("tsx")
                            || extension.eq_ignore_ascii_case("jsx") =>
                    {
                        SourceMode::Tsx
                    }
                    _ => SourceMode::TypeScript,
                }
            },
            source_file: Some(resolved.clone()),
            virtual_file_path: None,
        };
        let imported = analyze_with_context(&code, &imported_context);
        let nested =
            resolve_imported_types_for_request(&imported, &resolved, language, delta, state);
        for class in &nested.classes {
            if let Some(local_name) = import_aliases.get(&class.name) {
                let mut local_class = class.clone();
                local_class.name = local_name.clone();
                resolved_types.classes.push(local_class);
            }
        }
        for alias in &nested.aliases {
            if let Some(local_name) = import_aliases.get(&alias.name) {
                let mut local_alias = alias.clone();
                local_alias.name = local_name.clone();
                resolved_types.aliases.push(local_alias);
            }
        }
        resolved_types.classes.extend(nested.classes);
        resolved_types.aliases.extend(nested.aliases);
    }

    resolved_types
}

fn expand_local_type_closure(
    analysis: &AnalysisResult,
    requested_names: Option<&HashSet<String>>,
) -> HashSet<String> {
    let mut closure = HashSet::new();
    let class_map: HashMap<&str, &ClassInfo> = analysis
        .classes
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect();
    let alias_map: HashMap<&str, &TypeAliasInfo> = analysis
        .aliases
        .iter()
        .map(|alias| (alias.name.as_str(), alias))
        .collect();

    let seed_names: Vec<String> = match requested_names {
        Some(names) => names.iter().cloned().collect(),
        None => analysis
            .classes
            .iter()
            .map(|class| class.name.clone())
            .chain(analysis.aliases.iter().map(|alias| alias.name.clone()))
            .collect(),
    };

    let mut queue: VecDeque<String> = seed_names.into();
    while let Some(name) = queue.pop_front() {
        if !closure.insert(name.clone()) {
            continue;
        }

        if let Some(class) = class_map.get(name.as_str()) {
            for field in &class.fields {
                enqueue_annotation_names(field.type_annotation.as_deref(), &mut queue, &closure);
            }
            continue;
        }

        if let Some(alias) = alias_map.get(name.as_str()) {
            // Object aliases are already represented as ClassInfo fields, which avoids
            // mistaking property names for imported type names.
            if !class_map.contains_key(name.as_str()) {
                enqueue_annotation_names(
                    Some(alias.type_annotation.as_str()),
                    &mut queue,
                    &closure,
                );
            }
        }
    }

    closure
}

fn enqueue_annotation_names(
    annotation: Option<&str>,
    queue: &mut VecDeque<String>,
    seen: &HashSet<String>,
) {
    let mut names = HashSet::new();
    collect_annotation_names(annotation, &mut names);
    for name in names {
        if !seen.contains(&name) {
            queue.push_back(name);
        }
    }
}

fn collect_annotation_names(annotation: Option<&str>, names: &mut HashSet<String>) {
    let Some(annotation) = annotation else {
        return;
    };

    let mut current = String::new();
    for ch in annotation.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || ch == '.' {
            current.push(ch);
            continue;
        }

        flush_type_token(&mut current, names);
    }
    flush_type_token(&mut current, names);
}

fn flush_type_token(current: &mut String, names: &mut HashSet<String>) {
    if current.is_empty() {
        return;
    }
    let token = std::mem::take(current);
    let trimmed = token.trim_matches('.');
    if trimmed.is_empty() {
        return;
    }
    names.insert(trimmed.to_string());
    if let Some(root) = trimmed.split('.').next() {
        if !root.is_empty() {
            names.insert(root.to_string());
        }
    }
}

fn request_for_import(
    parsed: &ParsedImport,
    unresolved_names: &HashSet<String>,
) -> Option<ImportRequest> {
    if unresolved_names.is_empty() {
        return None;
    }

    let mut requested_exports = HashSet::new();
    let mut needs_full_module = false;

    for binding in &parsed.bindings {
        match binding {
            ParsedImportBinding::Named {
                local_name,
                exported_name,
            } => {
                if unresolved_names.contains(local_name) {
                    requested_exports.insert(exported_name.clone());
                }
            }
            ParsedImportBinding::Namespace { local_name }
            | ParsedImportBinding::Default { local_name } => {
                if unresolved_names.contains(local_name) {
                    needs_full_module = true;
                }
            }
            ParsedImportBinding::Wildcard => {
                needs_full_module = true;
            }
            ParsedImportBinding::ReExportWildcard => {
                requested_exports.extend(unresolved_names.iter().cloned());
            }
        }
    }

    if needs_full_module {
        Some(None)
    } else if requested_exports.is_empty() {
        None
    } else {
        Some(Some(requested_exports))
    }
}

fn merge_import_request(existing: &mut ImportRequest, request: &ImportRequest) {
    match (&mut *existing, request) {
        (_, None) => *existing = None,
        (Some(existing_names), Some(request_names)) => {
            existing_names.extend(request_names.iter().cloned());
        }
        (None, _) => {}
    }
}

fn note_import_request(
    processed_requests: &mut HashMap<String, ImportRequest>,
    path_key: &str,
    request: &ImportRequest,
) -> Option<ImportRequest> {
    match processed_requests.get_mut(path_key) {
        Some(existing) => match (&mut *existing, request) {
            (None, _) => None,
            (_, None) => {
                *existing = None;
                Some(None)
            }
            (Some(existing_names), Some(request_names)) => {
                let delta: HashSet<String> = request_names
                    .iter()
                    .filter(|name| !existing_names.contains(*name))
                    .cloned()
                    .collect();
                if delta.is_empty() {
                    None
                } else {
                    existing_names.extend(delta.iter().cloned());
                    Some(Some(delta))
                }
            }
        },
        None => {
            processed_requests.insert(path_key.to_string(), request.clone());
            Some(request.clone())
        }
    }
}

/// Extract the module path plus imported symbol bindings from an import statement.
fn parse_import(statement: &str, language: &Language) -> Option<ParsedImport> {
    match language {
        Language::TypeScript => parse_typescript_import(statement),
        Language::Python => parse_python_import(statement),
    }
}

fn parse_typescript_import(statement: &str) -> Option<ParsedImport> {
    let trimmed = statement.trim().trim_end_matches(';');
    let (clause_start, wildcard_reexport) = if trimmed.starts_with("import ") {
        ("import ".len(), false)
    } else if trimmed.starts_with("export ") {
        ("export ".len(), true)
    } else {
        return None;
    };
    let from_idx = trimmed.find("from ")?;
    let clause = trimmed[clause_start..from_idx].trim();
    let rest = &trimmed[from_idx + 5..];
    let quote = rest.chars().find(|c| *c == '"' || *c == '\'')?;
    let start = rest.find(quote)? + 1;
    let end = start + rest[start..].find(quote)?;
    let path = rest[start..end].to_string();
    let mut bindings = Vec::new();

    let reexport_clause = clause.strip_prefix("type ").unwrap_or(clause).trim();
    if wildcard_reexport && reexport_clause == "*" {
        bindings.push(ParsedImportBinding::ReExportWildcard);
    } else {
        parse_typescript_import_clause(clause, &mut bindings);
    }
    if bindings.is_empty() {
        return None;
    }

    Some(ParsedImport { path, bindings })
}

fn parse_typescript_import_clause(clause: &str, bindings: &mut Vec<ParsedImportBinding>) {
    let trimmed = clause.trim();
    if trimmed.is_empty() {
        return;
    }

    let trimmed = trimmed.strip_prefix("type ").unwrap_or(trimmed).trim();
    if trimmed.starts_with('{') {
        parse_typescript_named_imports(trimmed, bindings);
        return;
    }
    if let Some(local_name) = trimmed.strip_prefix("* as ").map(str::trim) {
        if !local_name.is_empty() {
            bindings.push(ParsedImportBinding::Namespace {
                local_name: local_name.to_string(),
            });
        }
        return;
    }
    if let Some((default_part, rest)) = trimmed.split_once(',') {
        let default_local = default_part.trim();
        if !default_local.is_empty() {
            bindings.push(ParsedImportBinding::Default {
                local_name: default_local.to_string(),
            });
        }
        parse_typescript_import_clause(rest, bindings);
        return;
    }

    bindings.push(ParsedImportBinding::Default {
        local_name: trimmed.to_string(),
    });
}

fn parse_typescript_named_imports(clause: &str, bindings: &mut Vec<ParsedImportBinding>) {
    let start = match clause.find('{') {
        Some(idx) => idx,
        None => return,
    };
    let end = match clause.rfind('}') {
        Some(idx) if idx > start => idx,
        _ => return,
    };
    let inner = &clause[start + 1..end];
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let part = part.strip_prefix("type ").unwrap_or(part).trim();
        let (exported_name, local_name) = match part.split_once(" as ") {
            Some((exported_name, local_name)) => (exported_name.trim(), local_name.trim()),
            None => (part, part),
        };
        if exported_name.is_empty() || local_name.is_empty() {
            continue;
        }
        bindings.push(ParsedImportBinding::Named {
            local_name: local_name.to_string(),
            exported_name: exported_name.to_string(),
        });
    }
}

fn parse_python_import(statement: &str) -> Option<ParsedImport> {
    let trimmed = statement.trim();
    if !trimmed.starts_with("from ") {
        return None;
    }

    let rest = &trimmed["from ".len()..];
    let import_idx = rest.find(" import ")?;
    let path = rest[..import_idx].trim().to_string();
    let imported = rest[import_idx + " import ".len()..].trim();
    if imported.is_empty() {
        return None;
    }

    let mut bindings = Vec::new();
    if imported == "*" {
        bindings.push(ParsedImportBinding::Wildcard);
    } else {
        for part in imported.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (exported_name, local_name) = match part.split_once(" as ") {
                Some((exported_name, local_name)) => (exported_name.trim(), local_name.trim()),
                None => (part, part),
            };
            bindings.push(ParsedImportBinding::Named {
                local_name: local_name.to_string(),
                exported_name: exported_name.to_string(),
            });
        }
    }

    Some(ParsedImport { path, bindings })
}

/// Resolve an import path to a source file available from the importing module.
fn resolve_typescript_path_candidate(base: &std::path::Path) -> Option<std::path::PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    for ext in &[
        ".ts",
        ".tsx",
        ".jsx",
        ".js",
        ".d.ts",
        "/index.ts",
        "/index.tsx",
        "/index.jsx",
        "/index.js",
    ] {
        let candidate = std::path::PathBuf::from(format!("{}{}", base.display(), ext));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn typescript_package_parts(import_path: &str) -> Option<(String, String)> {
    let mut parts = import_path.split('/');
    let first = parts.next()?;
    let package = if first.starts_with('@') {
        format!("{first}/{}", parts.next()?)
    } else {
        first.to_string()
    };
    Some((package, parts.collect::<Vec<_>>().join("/")))
}

fn resolve_typescript_export_target(
    value: &serde_json::Value,
    wildcard_replacement: Option<&str>,
) -> Option<String> {
    match value {
        serde_json::Value::String(target) => {
            let target = match wildcard_replacement {
                Some(replacement) => target.replace('*', replacement),
                None => target.clone(),
            };
            target.starts_with("./").then_some(target)
        }
        serde_json::Value::Array(targets) => targets
            .iter()
            .find_map(|target| resolve_typescript_export_target(target, wildcard_replacement)),
        serde_json::Value::Object(conditions) => ["types", "import", "default"]
            .into_iter()
            .filter_map(|condition| conditions.get(condition))
            .find_map(|target| resolve_typescript_export_target(target, wildcard_replacement)),
        _ => None,
    }
}

fn typescript_package_export_target(manifest: &serde_json::Value, subpath: &str) -> Option<String> {
    let exports = manifest.get("exports")?;
    let requested = if subpath.is_empty() {
        ".".to_string()
    } else {
        format!("./{subpath}")
    };

    let serde_json::Value::Object(entries) = exports else {
        return subpath
            .is_empty()
            .then(|| resolve_typescript_export_target(exports, None))
            .flatten();
    };

    if let Some(target) = entries.get(&requested) {
        return resolve_typescript_export_target(target, None);
    }

    let wildcard = entries
        .iter()
        .filter_map(|(key, target)| {
            let (prefix, suffix) = key.split_once('*')?;
            let replacement = requested
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .to_string();
            Some((prefix.len() + suffix.len(), replacement, target))
        })
        .max_by_key(|(specificity, _, _)| *specificity);
    if let Some((_, replacement, target)) = wildcard {
        return resolve_typescript_export_target(target, Some(&replacement));
    }

    if subpath.is_empty() && !entries.keys().any(|key| key.starts_with('.')) {
        return resolve_typescript_export_target(exports, None);
    }

    None
}

fn resolve_typescript_package_entry(
    source_dir: &std::path::Path,
    import_path: &str,
) -> Option<std::path::PathBuf> {
    let (package, subpath) = typescript_package_parts(import_path)?;
    for ancestor in source_dir.ancestors() {
        let package_dir = ancestor.join("node_modules").join(&package);
        if !package_dir.is_dir() {
            continue;
        }

        let manifest = std::fs::read_to_string(package_dir.join("package.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        if let Some(manifest) = manifest.as_ref() {
            if let Some(entry) = typescript_package_export_target(manifest, &subpath) {
                if let Some(path) = resolve_typescript_path_candidate(&package_dir.join(entry)) {
                    return Some(path);
                }
            }
        }

        if !subpath.is_empty() {
            if let Some(path) = resolve_typescript_path_candidate(&package_dir.join(&subpath)) {
                return Some(path);
            }
            continue;
        }

        if let Some(manifest) = manifest {
            for field in ["types", "typings", "module", "main"] {
                let Some(entry) = manifest.get(field).and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if let Some(path) = resolve_typescript_path_candidate(&package_dir.join(entry)) {
                    return Some(path);
                }
            }
        }
        if let Some(path) = resolve_typescript_path_candidate(&package_dir.join("index")) {
            return Some(path);
        }
    }
    None
}

fn resolve_import_file(
    source_dir: &std::path::Path,
    import_path: &str,
    language: &Language,
) -> Option<std::path::PathBuf> {
    match language {
        Language::TypeScript => {
            if import_path.starts_with('.') {
                resolve_typescript_path_candidate(&source_dir.join(import_path))
            } else {
                resolve_typescript_package_entry(source_dir, import_path)
            }
        }
        Language::Python => {
            let leading_dots = import_path.chars().take_while(|c| *c == '.').count();
            if leading_dots == 0 {
                return None;
            }

            let mut base_dir = source_dir.to_path_buf();
            for _ in 1..leading_dots {
                base_dir = base_dir.parent()?.to_path_buf();
            }

            let rel = import_path[leading_dots..].replace('.', "/");
            let candidate = if rel.is_empty() {
                base_dir.join("__init__.py")
            } else {
                base_dir.join(format!("{rel}.py"))
            };
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate = if rel.is_empty() {
                base_dir.join("__init__.py")
            } else {
                base_dir.join(&rel).join("__init__.py")
            };
            candidate.exists().then_some(candidate)
        }
    }
}

// ── Complexity threshold ────────────────────────────────────────────────────

pub fn check_complexity_threshold(
    analysis: &AnalysisResult,
    threshold: usize,
) -> Vec<ComplexityViolation> {
    check_complexity_threshold_for_functions_with_metric(
        &analysis.functions,
        threshold,
        ComplexityMetric::Cyclomatic,
    )
}

pub fn check_complexity_threshold_for_functions(
    functions: &[FunctionInfo],
    threshold: usize,
) -> Vec<ComplexityViolation> {
    check_complexity_threshold_for_functions_with_metric(
        functions,
        threshold,
        ComplexityMetric::Cyclomatic,
    )
}

fn function_complexity_value(function: &FunctionInfo, metric: ComplexityMetric) -> usize {
    match metric {
        ComplexityMetric::Cyclomatic => function.complexity,
        ComplexityMetric::Cognitive => function.cognitive_complexity,
    }
}

pub fn check_complexity_threshold_for_functions_with_metric(
    functions: &[FunctionInfo],
    threshold: usize,
    metric: ComplexityMetric,
) -> Vec<ComplexityViolation> {
    functions
        .iter()
        .filter(|f| function_complexity_value(f, metric) > threshold)
        .map(|f| ComplexityViolation {
            function: f.name.clone(),
            complexity: f.complexity,
            cognitive_complexity: f.cognitive_complexity,
            max_nesting_depth: f.max_nesting_depth,
            complexity_breakdown: f.complexity_breakdown.clone(),
            threshold,
            line: f.line,
        })
        .collect()
}

// ── Diff-aware filtering ────────────────────────────────────────────────────

pub fn filter_changed_functions(
    analysis: &AnalysisResult,
    changed_ranges: &[crate::tools::diff::ChangedRange],
) -> Vec<FunctionInfo> {
    analysis
        .functions
        .iter()
        .filter(|f| {
            changed_ranges
                .iter()
                .any(|r| f.line <= r.end_line && f.end_line >= r.start_line)
        })
        .cloned()
        .collect()
}

fn parser_for_mode(mode: SourceMode) -> Option<Parser> {
    let mut parser = Parser::new();
    let grammar = match mode {
        SourceMode::Python => tree_sitter_python::LANGUAGE.into(),
        SourceMode::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceMode::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser.set_language(&grammar).ok()?;
    Some(parser)
}

fn source_mode_for_file(language: &Language, path: &str) -> SourceMode {
    if matches!(language, Language::Python) {
        return SourceMode::Python;
    }
    match path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("tsx") | Some("jsx") => SourceMode::Tsx,
        _ => SourceMode::TypeScript,
    }
}

fn node_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// deliberately bounded and never treats dynamic calls as coverage proof.
/// `files` is a deterministic `(path, source)` list; ignored directories are
/// skipped and at most 80 files are inspected.
pub fn collect_call_edges(files: &[(String, String)], language: &Language) -> Vec<CallEdge> {
    let ignored = [
        ".git",
        "node_modules",
        "target",
        "dist",
        "build",
        "coverage",
        "__pycache__",
        ".venv",
        "venv",
    ];
    let mut ordered = files
        .iter()
        .filter(|(path, _)| !path.split(['/', '\\']).any(|part| ignored.contains(&part)))
        .take(80)
        .collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    let mut symbols = std::collections::HashMap::<String, String>::new();
    let mut imported = std::collections::HashMap::<(String, String), CallResolution>::new();
    for (path, source) in &ordered {
        let context = SourceContext {
            language: *language,
            mode: source_mode_for_file(language, path),
            source_file: None,
            virtual_file_path: None,
        };
        let analysis = analyze_with_context(source, &context);
        for function in analysis.functions.iter().filter(|f| !f.is_nested) {
            symbols
                .entry(function.name.clone())
                .or_insert_with(|| format!("{}:{}", function.name, function.line));
        }
        for imp in &analysis.imports {
            let statement = imp.statement.trim();
            if let Some((module, names)) = statement
                .strip_prefix("from ")
                .and_then(|rest| rest.split_once(" import "))
            {
                for binding in names.split(',') {
                    let parts = binding.trim().split(" as ").collect::<Vec<_>>();
                    let local = parts.last().copied().unwrap_or("").trim();
                    let symbol = parts.first().copied().unwrap_or(local).trim();
                    if !local.is_empty() {
                        imported.insert(
                            (path.clone(), local.to_string()),
                            CallResolution::Imported {
                                module: module.trim().to_string(),
                                symbol: symbol.to_string(),
                            },
                        );
                    }
                }
            } else if let Some(rest) = statement.strip_prefix("import ") {
                for binding in rest.split(',') {
                    let parts = binding.trim().split(" as ").collect::<Vec<_>>();
                    let local = parts.last().copied().unwrap_or("").trim();
                    let module = parts.first().copied().unwrap_or(local).trim();
                    if !local.is_empty() {
                        imported.insert(
                            (path.clone(), local.to_string()),
                            CallResolution::Imported {
                                module: module.to_string(),
                                symbol: module.rsplit('.').next().unwrap_or(module).to_string(),
                            },
                        );
                    }
                }
            }
        }
    }
    let mut edges = Vec::new();
    for (path, source) in ordered {
        let mode = source_mode_for_file(language, path);
        let Some(mut parser) = parser_for_mode(mode) else {
            continue;
        };
        let Some(tree) = parser.parse(source, None) else {
            continue;
        };
        let mut cursor = tree.root_node().walk();
        let mut stack = vec![(tree.root_node(), String::new())];
        while let Some((node, current_caller)) = stack.pop() {
            let mut caller = current_caller.clone();
            if is_callable_node(node, language) {
                if let Some(name) = node
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source.as_bytes()))
                {
                    caller = format!("{}:{}", name, node.start_position().row + 1);
                }
            }
            if matches!(
                (language, node.kind()),
                (Language::Python, "call") | (Language::TypeScript, "call_expression")
            ) {
                if let Some(callee) = node.child_by_field_name("function") {
                    let raw = node_text(&callee, source.as_bytes());
                    let simple = raw
                        .rsplit(['.', ':'])
                        .next()
                        .unwrap_or(raw.as_str())
                        .to_string();
                    if !caller.is_empty() && !simple.is_empty() {
                        let resolution = imported
                            .get(&(path.clone(), simple.clone()))
                            .cloned()
                            .or_else(|| symbols.get(&simple).map(|_| CallResolution::Local))
                            .unwrap_or_else(|| {
                                if raw.contains('.') {
                                    CallResolution::Dynamic
                                } else {
                                    CallResolution::Unresolved
                                }
                            });
                        let callee_id = symbols
                            .get(&simple)
                            .cloned()
                            .unwrap_or_else(|| format!("{}:0", simple));
                        edges.push(CallEdge {
                            caller_surface_id: caller.clone(),
                            callee_surface_id: callee_id,
                            source_file: path.clone(),
                            line: node.start_position().row + 1,
                            resolution,
                        });
                    }
                }
            }
            let mut children = node.named_children(&mut cursor).collect::<Vec<_>>();
            children.reverse();
            for child in children {
                stack.push((child, caller.clone()));
            }
        }
    }
    edges.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then(a.line.cmp(&b.line))
            .then(a.caller_surface_id.cmp(&b.caller_surface_id))
    });
    edges.dedup();
    edges
}

fn is_callable_node(node: tree_sitter::Node<'_>, language: &Language) -> bool {
    match language {
        Language::Python => matches!(
            node.kind(),
            "function_definition" | "async_function_definition"
        ),
        Language::TypeScript => matches!(
            node.kind(),
            "function_declaration" | "method_definition" | "arrow_function" | "function_expression"
        ),
    }
}
