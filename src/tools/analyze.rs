use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

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

    let mut requests_by_path: HashMap<String, (std::path::PathBuf, ImportRequest)> = HashMap::new();
    for import in &analysis.imports {
        let parsed = match parse_import(&import.statement, language) {
            Some(parsed) => parsed,
            None => continue,
        };
        if !parsed.path.starts_with('.') {
            continue;
        }
        let request = match request_for_import(&parsed, &unresolved_names) {
            Some(request) => request,
            None => continue,
        };

        let resolved = match resolve_import_file(source_dir, &parsed.path, language) {
            Some(path) => path,
            None => continue,
        };
        let key = resolved.to_string_lossy().to_string();
        requests_by_path
            .entry(key)
            .and_modify(|(_, existing)| merge_import_request(existing, &request))
            .or_insert((resolved, request));
    }

    for (path_key, (resolved, request)) in requests_by_path {
        let delta = match note_import_request(&mut state.processed_requests, &path_key, &request) {
            Some(delta) => delta,
            None => continue,
        };

        let code = match std::fs::read_to_string(&resolved) {
            Ok(code) => code,
            Err(_) => continue,
        };
        let imported = analyze(&code, language);
        let nested =
            resolve_imported_types_for_request(&imported, &resolved, language, delta, state);
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
    if !trimmed.starts_with("import ") {
        return None;
    }
    let from_idx = trimmed.find("from ")?;
    let clause = trimmed["import ".len()..from_idx].trim();
    let rest = &trimmed[from_idx + 5..];
    let quote = rest.chars().find(|c| *c == '"' || *c == '\'')?;
    let start = rest.find(quote)? + 1;
    let end = start + rest[start..].find(quote)?;
    let path = rest[start..end].to_string();
    let mut bindings = Vec::new();

    parse_typescript_import_clause(clause, &mut bindings);
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
            if exported_name.is_empty() || local_name.is_empty() {
                continue;
            }
            bindings.push(ParsedImportBinding::Named {
                local_name: local_name.to_string(),
                exported_name: exported_name.to_string(),
            });
        }
    }

    if bindings.is_empty() {
        return None;
    }

    Some(ParsedImport { path, bindings })
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
