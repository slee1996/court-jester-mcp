use std::collections::BTreeMap;

use tree_sitter::Parser;

use crate::types::*;

pub fn analyze(code: &str, language: &Language) -> AnalysisResult {
    let mut parser = Parser::new();

    match language {
        Language::Python => {
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .expect("Failed to load Python grammar");
        }
        Language::TypeScript => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .expect("Failed to load TypeScript grammar");
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
            }
        }
    };

    let root = tree.root_node();
    let bytes = code.as_bytes();
    let file_complexity = program_complexity(&root, language, bytes);

    let mut functions = vec![];
    let mut classes = vec![];
    let mut aliases = vec![];
    let mut imports = vec![];

    match language {
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
        }
    }

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
    }
}

fn text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
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

// ── Python ──────────────────────────────────────────────────────────────────

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
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method,
                is_nested: func_depth > 0,
                is_exported,
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
            // Bare `*` separator — all following params are keyword-only
            "keyword_separator" => {
                keyword_only = true;
                continue;
            }
            "identifier" => {
                let name = text(&child, source);
                if name != "self" && name != "cls" {
                    params.push(ParamInfo {
                        name: name.to_string(),
                        type_annotation: None,
                        default_value: None,
                        keyword_only,
                    });
                }
            }
            "typed_parameter" => {
                // typed_parameter has no "name" field — the identifier is the first named child
                let name = child.named_child(0).map(|n| text(&n, source)).unwrap_or("");
                if name != "self" && name != "cls" {
                    let type_ann = child
                        .child_by_field_name("type")
                        .map(|n| text(&n, source).to_string());
                    params.push(ParamInfo {
                        name: name.to_string(),
                        type_annotation: type_ann,
                        default_value: None,
                        keyword_only,
                    });
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
                });
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                params.push(ParamInfo {
                    name: text(&child, source).to_string(),
                    type_annotation: None,
                    default_value: None,
                    keyword_only: false,
                });
            }
            _ => {}
        }
    }

    params
}

/// Extract fields from a Python class body (dataclass-style type-annotated fields).
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
                        // `y: str = "hello"` — annotated assignment with default
                        let full = text(&inner, source);
                        // Look for the pattern: name: type = value
                        if let Some(colon_pos) = full.find(':') {
                            let name = full[..colon_pos].trim();
                            let rest = &full[colon_pos + 1..];
                            let (type_ann, _default) = if let Some(eq_pos) = rest.find('=') {
                                (rest[..eq_pos].trim(), rest[eq_pos + 1..].trim())
                            } else {
                                (rest.trim(), "")
                            };
                            if !name.is_empty() && !name.contains(' ') {
                                fields.push(FieldInfo {
                                    name: name.to_string(),
                                    type_annotation: if type_ann.is_empty() {
                                        None
                                    } else {
                                        Some(type_ann.to_string())
                                    },
                                    optional: false,
                                    has_default: true,
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
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method: false,
                is_nested: func_depth > 0,
                is_exported: ts_is_exported(node),
            });
            child_depth = func_depth + 1;
        }
        "method_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(&n, source).to_string())
                .unwrap_or_default();
            let params = extract_ts_params(node, source);
            let return_type = node
                .child_by_field_name("return_type")
                .map(|n| type_text(&n, source));
            let metrics = callable_complexity(node, &Language::TypeScript, source);
            functions.push(FunctionInfo {
                name,
                params,
                return_type,
                line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                complexity: metrics.cyclomatic,
                cognitive_complexity: metrics.cognitive,
                max_nesting_depth: metrics.max_nesting_depth,
                complexity_breakdown: metrics.breakdown,
                is_method: true,
                is_nested: func_depth > 0,
                is_exported: false,
            });
            child_depth = func_depth + 1;
        }
        "variable_declarator" => {
            // Detect arrow functions: const foo = (x: string): string => ...
            if let Some(value) = node.child_by_field_name("value") {
                if value.kind() == "arrow_function" {
                    let name = node
                        .child_by_field_name("name")
                        .map(|n| text(&n, source).to_string())
                        .unwrap_or_default();
                    let params = extract_ts_params(&value, source);
                    let return_type = value
                        .child_by_field_name("return_type")
                        .map(|n| type_text(&n, source));
                    let metrics = callable_complexity(&value, &Language::TypeScript, source);
                    functions.push(FunctionInfo {
                        name,
                        params,
                        return_type,
                        line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        complexity: metrics.cyclomatic,
                        cognitive_complexity: metrics.cognitive,
                        max_nesting_depth: metrics.max_nesting_depth,
                        complexity_breakdown: metrics.breakdown,
                        is_method: false,
                        is_nested: func_depth > 0,
                        is_exported: ts_is_exported(node),
                    });
                    child_depth = func_depth + 1;
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
        "import_statement" => {
            imports.push(ImportInfo {
                statement: text(node, source).to_string(),
                line: node.start_position().row + 1,
            });
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
                let name = child
                    .child_by_field_name("pattern")
                    .map(|n| text(&n, source).to_string())
                    .unwrap_or_default();
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|n| type_text(&n, source));
                let default_value = child
                    .child_by_field_name("value")
                    .map(|n| text(&n, source).to_string());
                params.push(ParamInfo {
                    name,
                    type_annotation: type_ann,
                    default_value,
                    keyword_only: false,
                });
            }
            _ => {}
        }
    }

    params
}

// ── Import resolution ───────────────────────────────────────────────────────

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
    let known_classes: std::collections::HashSet<&str> =
        analysis.classes.iter().map(|c| c.name.as_str()).collect();
    let known_aliases: std::collections::HashSet<&str> =
        analysis.aliases.iter().map(|a| a.name.as_str()).collect();

    let mut resolved_types = ResolvedTypeInfo::default();
    let mut resolved_paths = std::collections::HashSet::new();

    for import in &analysis.imports {
        let path = match parse_import_path(&import.statement, language) {
            Some(p) => p,
            None => continue,
        };

        // Only resolve relative imports
        if !path.starts_with('.') {
            continue;
        }

        let resolved = resolve_import_file(source_dir, &path, language);
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

        let imported = analyze(&code, language);
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

/// Extract the module path from an import statement.
fn parse_import_path(statement: &str, language: &Language) -> Option<String> {
    match language {
        Language::TypeScript => {
            // Match: from "path" or from 'path'
            let from_idx = statement.find("from ")?;
            let rest = &statement[from_idx + 5..];
            let quote = rest.chars().find(|c| *c == '"' || *c == '\'')?;
            let start = rest.find(quote)? + 1;
            let end = start + rest[start..].find(quote)?;
            Some(rest[start..end].to_string())
        }
        Language::Python => {
            // Match: from .module import ... or from .pkg.module import ...
            let trimmed = statement.trim();
            if trimmed.starts_with("from .") {
                let rest = &trimmed[5..]; // after "from "
                let end = rest.find(' ').unwrap_or(rest.len());
                Some(rest[..end].to_string())
            } else {
                None
            }
        }
    }
}

/// Resolve a relative import path to an actual file.
fn resolve_import_file(
    source_dir: &std::path::Path,
    import_path: &str,
    language: &Language,
) -> Option<std::path::PathBuf> {
    match language {
        Language::TypeScript => {
            // "./types" or "../../types/foo" → try .ts/.tsx and index files.
            let base = source_dir.join(import_path);

            if base.exists() {
                return Some(base);
            }
            for ext in &[".ts", ".tsx", "/index.ts", "/index.tsx"] {
                let candidate = std::path::PathBuf::from(format!("{}{}", base.display(), ext));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            None
        }
        Language::Python => {
            // ".module" → module.py, "..pkg.module" → ../pkg/module.py
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
            // Try as package: module/__init__.py
            let candidate = if rel.is_empty() {
                base_dir.join("__init__.py")
            } else {
                base_dir.join(&rel).join("__init__.py")
            };
            if candidate.exists() {
                return Some(candidate);
            }
            None
        }
    }
}

// ── Complexity threshold ────────────────────────────────────────────────────

pub fn check_complexity_threshold(
    analysis: &AnalysisResult,
    threshold: usize,
) -> Vec<ComplexityViolation> {
    check_complexity_threshold_for_functions(&analysis.functions, threshold)
}

pub fn check_complexity_threshold_for_functions(
    functions: &[FunctionInfo],
    threshold: usize,
) -> Vec<ComplexityViolation> {
    functions
        .iter()
        .filter(|f| f.complexity > threshold)
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
