use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use serde::Serialize;
use tree_sitter::{Node, Parser};

use crate::types::{FunctionInfo, Language, SourceMode};

pub const DEFAULT_MAX_MUTANTS: usize = 8;
pub const MAX_MUTANTS: usize = 32;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperator {
    ComparisonBoundary,
    EqualityNegation,
    ConditionNegation,
    BooleanLiteral,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MutationCandidate {
    pub id: String,
    pub operator: MutationOperator,
    pub surface_id: String,
    pub line: usize,
    pub column: usize,
    pub original: String,
    pub replacement: String,
    pub witness: String,
    #[serde(skip)]
    pub start_byte: usize,
    #[serde(skip)]
    pub end_byte: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CouplingKind {
    PrivateTargetAccess,
    PrivateTargetImport,
    PrivateTargetSpy,
    TargetSourceIntrospection,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CouplingFinding {
    pub kind: CouplingKind,
    pub line: usize,
    pub column: usize,
    pub symbol: String,
    pub evidence: String,
    pub message: String,
    pub test_source_file: String,
}

fn parser_for_mode(mode: SourceMode) -> Result<Parser, String> {
    let mut parser = Parser::new();
    let grammar = match mode {
        SourceMode::Python => tree_sitter_python::LANGUAGE.into(),
        SourceMode::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceMode::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser
        .set_language(&grammar)
        .map_err(|error| format!("test-quality parser unavailable: {error}"))?;
    Ok(parser)
}

#[derive(Debug)]
struct CallableSurface<'a> {
    start_byte: usize,
    end_byte: usize,
    function: Option<&'a FunctionInfo>,
}

fn callable_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::to_owned)
}

fn enclosing_named_node(mut node: Node<'_>, kinds: &[&str], source: &[u8]) -> Option<String> {
    while let Some(parent) = node.parent() {
        if kinds.contains(&parent.kind()) {
            return callable_name(parent, source);
        }
        if matches!(
            parent.kind(),
            "function_definition"
                | "function_declaration"
                | "method_definition"
                | "arrow_function"
                | "function_expression"
        ) {
            return None;
        }
        node = parent;
    }
    None
}
fn callable_identity(node: Node<'_>, simple_name: &str, source: &[u8]) -> String {
    if node.kind() == "method_definition" {
        if let Some(class_name) =
            enclosing_named_node(node, &["class_declaration", "class"], source)
        {
            return format!("{class_name}#{simple_name}");
        }
    }
    if matches!(node.kind(), "method_definition" | "pair") {
        if let Some(object_name) = enclosing_named_node(node, &["variable_declarator"], source) {
            return format!("{object_name}.{simple_name}");
        }
    }
    simple_name.to_string()
}

fn matching_function<'a>(
    identity: &str,
    simple_name: &str,
    line: usize,
    functions: &'a [&FunctionInfo],
) -> Option<&'a FunctionInfo> {
    if let Some(function) = functions
        .iter()
        .copied()
        .find(|function| function.line == line && function.name == identity)
    {
        return Some(function);
    }
    let mut fallback = functions.iter().copied().filter(|function| {
        function.line == line
            && (function.name == simple_name
                || function
                    .name
                    .strip_suffix(simple_name)
                    .is_some_and(|prefix| prefix.ends_with('.')))
    });
    let matched = fallback.next()?;
    fallback.next().is_none().then_some(matched)
}

fn collect_callable_surfaces<'functions>(
    node: Node<'_>,
    source: &[u8],
    functions: &'functions [&FunctionInfo],
    surfaces: &mut Vec<CallableSurface<'functions>>,
) {
    let line = node.start_position().row + 1;
    let callable = match node.kind() {
        "function_definition" | "function_declaration" | "method_definition" => {
            let function = callable_name(node, source).and_then(|name| {
                let identity = callable_identity(node, &name, source);
                matching_function(&identity, &name, line, functions)
            });
            Some((node.start_byte(), node.end_byte(), function))
        }
        "arrow_function" | "function_expression" | "lambda" => {
            let function = node
                .parent()
                .filter(|parent| matches!(parent.kind(), "variable_declarator" | "pair"))
                .and_then(|parent| {
                    let name_node = if parent.kind() == "pair" {
                        parent.child_by_field_name("key")
                    } else {
                        parent.child_by_field_name("name")
                    };
                    name_node
                        .and_then(|name| name.utf8_text(source).ok())
                        .and_then(|name| {
                            let name = name.trim_matches(['\'', '"']);
                            let identity = callable_identity(parent, name, source);
                            matching_function(
                                &identity,
                                name,
                                parent.start_position().row + 1,
                                functions,
                            )
                        })
                });
            Some((node.start_byte(), node.end_byte(), function))
        }
        _ => None,
    };
    if let Some((start_byte, end_byte, function)) = callable {
        surfaces.push(CallableSurface {
            start_byte,
            end_byte,
            function,
        });
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_callable_surfaces(child, source, functions, surfaces);
    }
}

fn surface_for_range<'surface, 'function>(
    start_byte: usize,
    end_byte: usize,
    surfaces: &'surface [CallableSurface<'function>],
) -> Option<&'surface CallableSurface<'function>> {
    surfaces
        .iter()
        .filter(|surface| start_byte >= surface.start_byte && end_byte <= surface.end_byte)
        .min_by_key(|surface| surface.end_byte.saturating_sub(surface.start_byte))
}

fn is_typescript_type_literal(node: Node<'_>) -> bool {
    if node.kind() == "literal_type" {
        return true;
    }
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.kind() == "literal_type" {
            return true;
        }
        if matches!(
            parent.kind(),
            "statement_block"
                | "expression_statement"
                | "lexical_declaration"
                | "return_statement"
                | "if_statement"
                | "arrow_function"
                | "function_declaration"
                | "function_expression"
                | "method_definition"
        ) {
            return false;
        }
        ancestor = parent.parent();
    }
    false
}

fn comparison_replacement(
    operator: &str,
) -> Option<(&'static str, MutationOperator, &'static str)> {
    match operator {
        "<" => Some((
            "<=",
            MutationOperator::ComparisonBoundary,
            "boundary where both operands are equal",
        )),
        "<=" => Some((
            "<",
            MutationOperator::ComparisonBoundary,
            "boundary where both operands are equal",
        )),
        ">" => Some((
            ">=",
            MutationOperator::ComparisonBoundary,
            "boundary where both operands are equal",
        )),
        ">=" => Some((
            ">",
            MutationOperator::ComparisonBoundary,
            "boundary where both operands are equal",
        )),
        "==" => Some((
            "!=",
            MutationOperator::EqualityNegation,
            "case where equality changes the branch result",
        )),
        "!=" => Some((
            "==",
            MutationOperator::EqualityNegation,
            "case where equality changes the branch result",
        )),
        "===" => Some((
            "!==",
            MutationOperator::EqualityNegation,
            "case where strict equality changes the branch result",
        )),
        "!==" => Some((
            "===",
            MutationOperator::EqualityNegation,
            "case where strict equality changes the branch result",
        )),
        _ => None,
    }
}

fn parent_accepts_operator(node: Node<'_>, language: Language) -> bool {
    node.parent().is_some_and(|parent| match language {
        Language::Python => parent.kind() == "comparison_operator",
        Language::TypeScript => parent.kind() == "binary_expression",
    })
}

fn has_mutatable_operator(node: Node<'_>, language: Language, source: &[u8]) -> bool {
    if node
        .utf8_text(source)
        .ok()
        .and_then(comparison_replacement)
        .is_some()
        && parent_accepts_operator(node, language)
    {
        return true;
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| has_mutatable_operator(child, language, source));
    found
}

fn collect_mutations(
    node: Node<'_>,
    code: &str,
    language: Language,
    surfaces: &[CallableSurface<'_>],
    candidates: &mut Vec<MutationCandidate>,
) {
    let line = node.start_position().row + 1;
    let source = code.as_bytes();
    let text = node.utf8_text(source).unwrap_or_default();
    if let Some(function) = surface_for_range(node.start_byte(), node.end_byte(), surfaces)
        .and_then(|surface| surface.function)
    {
        if parent_accepts_operator(node, language) {
            if let Some((replacement, operator, witness)) = comparison_replacement(text) {
                candidates.push(MutationCandidate {
                    id: String::new(),
                    operator,
                    surface_id: format!("{}:{}", function.name, function.line),
                    line,
                    column: node.start_position().column + 1,
                    original: text.to_string(),
                    replacement: replacement.to_string(),
                    witness: witness.to_string(),
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                });
            }
        }

        if node.kind() == "if_statement" {
            if let Some(condition) = node.child_by_field_name("condition") {
                if !has_mutatable_operator(condition, language, source) {
                    let original = condition.utf8_text(source).unwrap_or_default();
                    if !original.trim().is_empty() {
                        let replacement = match language {
                            Language::Python => format!("not ({original})"),
                            Language::TypeScript if original.trim().starts_with('(') => {
                                format!("(!{original})")
                            }
                            Language::TypeScript => format!("(!({original}))"),
                        };
                        candidates.push(MutationCandidate {
                            id: String::new(),
                            operator: MutationOperator::ConditionNegation,
                            surface_id: format!("{}:{}", function.name, function.line),
                            line: condition.start_position().row + 1,
                            column: condition.start_position().column + 1,
                            original: original.to_string(),
                            replacement,
                            witness:
                                "case where the condition truth value controls observable behavior"
                                    .into(),
                            start_byte: condition.start_byte(),
                            end_byte: condition.end_byte(),
                        });
                    }
                }
            }
        }

        let boolean_replacement = match text {
            "True" => Some("False"),
            "False" => Some("True"),
            "true" => Some("false"),
            "false" => Some("true"),
            _ => None,
        };
        if let Some(replacement) = boolean_replacement
            .filter(|_| language != Language::TypeScript || !is_typescript_type_literal(node))
        {
            candidates.push(MutationCandidate {
                id: String::new(),
                operator: MutationOperator::BooleanLiteral,
                surface_id: format!("{}:{}", function.name, function.line),
                line,
                column: node.start_position().column + 1,
                original: text.to_string(),
                replacement: replacement.to_string(),
                witness: "case where this boolean value reaches the public result".into(),
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_mutations(child, code, language, surfaces, candidates);
    }
}

pub fn plan_mutations(
    code: &str,
    language: Language,
    source_mode: SourceMode,
    functions: &[&FunctionInfo],
    max_mutants: usize,
) -> Result<Vec<MutationCandidate>, String> {
    if functions.is_empty() || max_mutants == 0 {
        return Ok(Vec::new());
    }
    let mut parser = parser_for_mode(source_mode)?;
    let tree = parser
        .parse(code, None)
        .ok_or_else(|| "test-quality parser produced no tree".to_string())?;
    if tree.root_node().has_error() {
        return Err("test-quality mutation planning rejected malformed target source".into());
    }
    let mut surfaces = Vec::new();
    collect_callable_surfaces(tree.root_node(), code.as_bytes(), functions, &mut surfaces);
    let mut candidates = Vec::new();
    collect_mutations(tree.root_node(), code, language, &surfaces, &mut candidates);
    candidates.sort_by(|left, right| {
        left.start_byte
            .cmp(&right.start_byte)
            .then_with(|| left.operator.cmp(&right.operator))
    });
    candidates.dedup_by(|left, right| {
        left.start_byte == right.start_byte
            && left.end_byte == right.end_byte
            && left.replacement == right.replacement
    });
    candidates.retain(|candidate| {
        apply_mutation(code, candidate)
            .ok()
            .and_then(|mutated| parser.parse(&mutated, None))
            .is_some_and(|tree| !tree.root_node().has_error())
    });

    let mut by_surface = BTreeMap::<String, VecDeque<MutationCandidate>>::new();
    for candidate in candidates {
        by_surface
            .entry(candidate.surface_id.clone())
            .or_default()
            .push_back(candidate);
    }
    let surface_order = functions
        .iter()
        .map(|function| format!("{}:{}", function.name, function.line))
        .collect::<Vec<_>>();
    let mut selected = Vec::new();
    while selected.len() < max_mutants {
        let mut advanced = false;
        for surface in &surface_order {
            if selected.len() == max_mutants {
                break;
            }
            if let Some(candidate) = by_surface.get_mut(surface).and_then(VecDeque::pop_front) {
                selected.push(candidate);
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    for (index, candidate) in selected.iter_mut().enumerate() {
        candidate.id = format!("mutant-{:03}", index + 1);
    }
    Ok(selected)
}

pub fn apply_mutation(code: &str, candidate: &MutationCandidate) -> Result<String, String> {
    let current = code
        .get(candidate.start_byte..candidate.end_byte)
        .ok_or_else(|| format!("{} points outside the source", candidate.id))?;
    if current != candidate.original {
        return Err(format!(
            "{} expected {:?} at byte range {}..{}, found {:?}",
            candidate.id, candidate.original, candidate.start_byte, candidate.end_byte, current
        ));
    }
    let mut mutated = String::with_capacity(
        code.len()
            + candidate
                .replacement
                .len()
                .saturating_sub(candidate.original.len()),
    );
    mutated.push_str(&code[..candidate.start_byte]);
    mutated.push_str(&candidate.replacement);
    mutated.push_str(&code[candidate.end_byte..]);
    Ok(mutated)
}

fn identifier_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character == '_' || character == '$' || character.is_ascii_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            if current
                .chars()
                .next()
                .is_some_and(|first| first == '_' || first == '$' || first.is_ascii_alphabetic())
            {
                tokens.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if !current.is_empty()
        && current
            .chars()
            .next()
            .is_some_and(|first| first == '_' || first == '$' || first.is_ascii_alphabetic())
    {
        tokens.push(current);
    }
    tokens
}

fn string_literal_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?.trim();
    (text.len() >= 2).then(|| text[1..text.len() - 1].to_string())
}
fn normalized_path(path: &Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalized_source_module(path: &Path) -> std::path::PathBuf {
    let mut path = normalized_path(path);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "py" | "ts" | "tsx" | "js" | "jsx"
            )
        })
    {
        path.set_extension("");
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "index" | "__init__"))
    {
        path.pop();
    }
    path
}

fn module_matches_target(
    module: &str,
    language: Language,
    target_source_file: Option<&str>,
    test_source_file: Option<&str>,
) -> bool {
    let (Some(target_source_file), Some(test_source_file)) = (target_source_file, test_source_file)
    else {
        return false;
    };
    let target = normalized_source_module(Path::new(target_source_file));
    let test_dir = normalized_path(Path::new(test_source_file))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    match language {
        Language::TypeScript => {
            if !module.starts_with('.') && !Path::new(module).is_absolute() {
                return false;
            }
            let resolved = if Path::new(module).is_absolute() {
                normalized_source_module(Path::new(module))
            } else {
                normalized_source_module(&test_dir.join(module))
            };
            resolved == target
        }
        Language::Python => {
            let leading_dots = module
                .chars()
                .take_while(|character| *character == '.')
                .count();
            let module_tail = module[leading_dots..].replace('.', "/");
            if leading_dots > 0 {
                let mut base = test_dir;
                for _ in 1..leading_dots {
                    base.pop();
                }
                return normalized_source_module(&base.join(module_tail)) == target;
            }

            let mut fallback_match = false;
            let mut base = Some(test_dir.as_path());
            while let Some(directory) = base {
                let module_path = directory.join(&module_tail);
                fallback_match |= normalized_source_module(&module_path) == target;
                let mut module_file = module_path.clone();
                module_file.set_extension("py");
                for candidate in [module_file, module_path.join("__init__.py")] {
                    if candidate.is_file() {
                        return normalized_source_module(&candidate) == target;
                    }
                }
                base = directory.parent();
            }
            fallback_match
        }
    }
}

fn import_source(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(value) = node.child_by_field_name("source") {
        return string_literal_text(value, source);
    }
    let text = node.utf8_text(source).ok()?;
    if let Some(from_index) = text.rfind("from") {
        let tail = text[from_index + 4..].trim();
        if let Some(quote) = tail
            .chars()
            .next()
            .filter(|value| matches!(value, '\'' | '"'))
        {
            let value = tail[1..].split(quote).next().unwrap_or_default();
            return Some(value.to_string());
        }
        return tail
            .split_whitespace()
            .next()
            .map(|value| value.trim_matches(['\'', '"']).to_string());
    }
    if let Some(tail) = text.trim_start().strip_prefix("import ") {
        return tail
            .split_whitespace()
            .next()
            .map(|value| value.trim_matches(['\'', '"']).to_string());
    }
    None
}
#[derive(Debug)]
struct ImportBinding {
    module: String,
    imported: String,
    local: String,
    required_path: Vec<String>,
}

fn aliased_binding(part: &str) -> Option<(String, String)> {
    let tokens = identifier_tokens(part);
    let imported = tokens.first()?.clone();
    let local = tokens
        .iter()
        .position(|token| token == "as")
        .and_then(|alias| tokens.get(alias + 1))
        .cloned()
        .unwrap_or_else(|| imported.clone());
    Some((imported, local))
}

fn python_import_bindings(text: &str) -> Vec<ImportBinding> {
    let trimmed = text.trim();
    if let Some((module, bindings)) = trimmed
        .strip_prefix("from ")
        .and_then(|tail| tail.split_once(" import "))
    {
        return bindings
            .trim_matches(['(', ')'])
            .split(',')
            .filter_map(aliased_binding)
            .map(|(imported, local)| ImportBinding {
                module: module.trim().to_string(),
                imported,
                local,
                required_path: Vec::new(),
            })
            .collect();
    }
    trimmed
        .strip_prefix("import ")
        .into_iter()
        .flat_map(|imports| imports.split(','))
        .filter_map(|part| {
            let mut tokens = part.split_whitespace();
            let module = tokens.next()?.trim().to_string();
            let alias = match (tokens.next(), tokens.next()) {
                (Some("as"), Some(alias)) => Some(alias.to_string()),
                _ => None,
            };
            let mut module_parts = module
                .trim_start_matches('.')
                .split('.')
                .filter(|part| !part.is_empty());
            let root = module_parts.next()?.to_string();
            let required_path = alias
                .as_ref()
                .map(|_| Vec::new())
                .unwrap_or_else(|| module_parts.map(str::to_string).collect());
            Some(ImportBinding {
                module,
                imported: "*".into(),
                local: alias.unwrap_or(root),
                required_path,
            })
        })
        .collect()
}

fn typescript_import_bindings(node: Node<'_>, module: &str, source: &[u8]) -> Vec<ImportBinding> {
    let import_text = node.utf8_text(source).unwrap_or_default().trim_start();
    if import_text.starts_with("import type ") {
        return Vec::new();
    }
    let mut node_cursor = node.walk();
    let Some(clause) = node
        .named_children(&mut node_cursor)
        .find(|child| child.kind() == "import_clause")
    else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    let mut clause_cursor = clause.walk();
    for child in clause.named_children(&mut clause_cursor) {
        match child.kind() {
            "identifier" => bindings.push(ImportBinding {
                module: module.to_string(),
                imported: "default".into(),
                local: child.utf8_text(source).unwrap_or_default().to_string(),
                required_path: Vec::new(),
            }),
            "namespace_import" => {
                let mut cursor = child.walk();
                if let Some(local) = child
                    .named_children(&mut cursor)
                    .filter(|candidate| candidate.kind() == "identifier")
                    .last()
                {
                    bindings.push(ImportBinding {
                        module: module.to_string(),
                        imported: "*".into(),
                        local: local.utf8_text(source).unwrap_or_default().to_string(),
                        required_path: Vec::new(),
                    });
                }
            }
            "named_imports" => {
                let mut cursor = child.walk();
                for specifier in child
                    .named_children(&mut cursor)
                    .filter(|candidate| candidate.kind() == "import_specifier")
                {
                    if specifier
                        .utf8_text(source)
                        .unwrap_or_default()
                        .trim_start()
                        .starts_with("type ")
                    {
                        continue;
                    }
                    let imported_node = specifier
                        .child_by_field_name("name")
                        .or_else(|| specifier.named_child(0));
                    let Some(imported) = imported_node
                        .and_then(|name| name.utf8_text(source).ok())
                        .map(str::to_owned)
                    else {
                        continue;
                    };
                    let local = specifier
                        .child_by_field_name("alias")
                        .or_else(|| {
                            (specifier.named_child_count() > 1)
                                .then(|| specifier.named_child(1))
                                .flatten()
                        })
                        .and_then(|alias| alias.utf8_text(source).ok())
                        .map(str::to_owned)
                        .unwrap_or_else(|| imported.clone());
                    bindings.push(ImportBinding {
                        module: module.to_string(),
                        imported,
                        local,
                        required_path: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    bindings
}

fn import_bindings(node: Node<'_>, source: &[u8], language: Language) -> Vec<ImportBinding> {
    let text = node.utf8_text(source).unwrap_or_default();
    match language {
        Language::Python => python_import_bindings(text),
        Language::TypeScript => import_source(node, source)
            .map(|module| typescript_import_bindings(node, &module, source))
            .unwrap_or_default(),
    }
}
#[derive(Debug)]
enum LexicalBindingKind {
    Target { required_path: Vec<String> },
    Shadow,
}

#[derive(Debug)]
struct LexicalBinding {
    local: String,
    scope_start: usize,
    scope_end: usize,
    visible_from: usize,
    prelude_range: Option<(usize, usize)>,
    kind: LexicalBindingKind,
}

fn is_scope_kind(kind: &str, language: Language) -> bool {
    match language {
        Language::Python => matches!(
            kind,
            "module"
                | "function_definition"
                | "lambda"
                | "class_definition"
                | "list_comprehension"
                | "set_comprehension"
                | "dictionary_comprehension"
                | "generator_expression"
        ),
        Language::TypeScript => matches!(
            kind,
            "program"
                | "statement_block"
                | "function_declaration"
                | "function_expression"
                | "for_statement"
                | "for_in_statement"
                | "catch_clause"
                | "arrow_function"
                | "method_definition"
                | "class_declaration"
        ),
    }
}

fn containing_scope(mut node: Node<'_>, language: Language) -> Node<'_> {
    loop {
        if is_scope_kind(node.kind(), language) {
            return node;
        }
        let Some(parent) = node.parent() else {
            return node;
        };
        node = parent;
    }
}

fn push_lexical_binding(
    bindings: &mut Vec<LexicalBinding>,
    local: String,
    scope: Node<'_>,
    kind: LexicalBindingKind,
) {
    if !local.is_empty() {
        bindings.push(LexicalBinding {
            local,
            scope_start: scope.start_byte(),
            scope_end: scope.end_byte(),
            visible_from: scope.start_byte(),
            prelude_range: None,
            kind,
        });
    }
}

fn collect_pattern_identifiers(node: Node<'_>, source: &[u8], names: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            if let Ok(name) = node.utf8_text(source) {
                names.push(name.to_string());
            }
        }
        "required_parameter"
        | "optional_parameter"
        | "default_parameter"
        | "typed_parameter"
        | "typed_default_parameter"
        | "assignment_pattern" => {
            if let Some(pattern) = node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.child_by_field_name("left"))
                .or_else(|| node.named_child(0))
            {
                collect_pattern_identifiers(pattern, source, names);
            }
        }
        "rest_pattern" | "list_splat_pattern" | "dictionary_splat_pattern" => {
            if let Some(pattern) = node
                .child_by_field_name("argument")
                .or_else(|| node.named_child(0))
            {
                collect_pattern_identifiers(pattern, source, names);
            }
        }
        "pair_pattern" | "as_pattern" => {
            if let Some(pattern) = node
                .child_by_field_name("value")
                .or_else(|| node.child_by_field_name("alias"))
                .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)))
            {
                collect_pattern_identifiers(pattern, source, names);
            }
        }
        "object_pattern" | "array_pattern" | "list_pattern" | "tuple_pattern" | "pattern_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_pattern_identifiers(child, source, names);
            }
        }
        _ => {}
    }
}

fn typescript_var_scope<'tree>(mut node: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    let mut declaration = Some(node);
    while let Some(current) = declaration {
        if matches!(
            current.kind(),
            "variable_declaration" | "lexical_declaration"
        ) {
            if !current
                .utf8_text(source)
                .unwrap_or_default()
                .trim_start()
                .starts_with("var ")
            {
                return None;
            }
            break;
        }
        declaration = current.parent();
    }
    loop {
        if matches!(
            node.kind(),
            "program"
                | "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
        ) {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn collect_shadow_bindings(
    node: Node<'_>,
    source: &[u8],
    language: Language,
    bindings: &mut Vec<LexicalBinding>,
) {
    let mut patterns = Vec::new();
    let scope = match node.kind() {
        "formal_parameters" | "parameters" | "lambda_parameters" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_pattern_identifiers(child, source, &mut patterns);
            }
            Some(containing_scope(node.parent().unwrap_or(node), language))
        }
        "lambda" | "arrow_function" => {
            if let Some(pattern) = node.child_by_field_name("parameters") {
                if !matches!(
                    pattern.kind(),
                    "formal_parameters" | "parameters" | "lambda_parameters"
                ) {
                    collect_pattern_identifiers(pattern, source, &mut patterns);
                }
            }
            Some(node)
        }
        "variable_declarator" => {
            if let Some(pattern) = node.child_by_field_name("name") {
                collect_pattern_identifiers(pattern, source, &mut patterns);
            }
            Some(
                (language == Language::TypeScript)
                    .then(|| typescript_var_scope(node, source))
                    .flatten()
                    .unwrap_or_else(|| containing_scope(node, language)),
            )
        }
        "assignment" | "augmented_assignment" | "named_expression" => {
            if let Some(pattern) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("name"))
            {
                collect_pattern_identifiers(pattern, source, &mut patterns);
            }
            Some(containing_scope(node, language))
        }
        "for_in_clause" if language == Language::Python => {
            let scope = containing_scope(node, language);
            let pattern = node
                .child_by_field_name("left")
                .or_else(|| node.named_child(0));
            let iterable = node
                .child_by_field_name("right")
                .or_else(|| node.named_child(1));
            let body = scope
                .child_by_field_name("body")
                .or_else(|| scope.named_child(0));
            let mut clause_names = Vec::new();
            if let Some(pattern) = pattern {
                collect_pattern_identifiers(pattern, source, &mut clause_names);
            }
            if let Some(iterable) = iterable {
                for local in clause_names {
                    bindings.push(LexicalBinding {
                        local,
                        scope_start: scope.start_byte(),
                        scope_end: scope.end_byte(),
                        visible_from: iterable.end_byte(),
                        prelude_range: body.map(|body| (body.start_byte(), body.end_byte())),
                        kind: LexicalBindingKind::Shadow,
                    });
                }
            }
            None
        }
        "for_statement" | "for_in_statement" => {
            if let Some(pattern) = node.child_by_field_name("left") {
                collect_pattern_identifiers(pattern, source, &mut patterns);
            }
            Some(containing_scope(node, language))
        }
        "catch_clause" => {
            if let Some(pattern) = node.child_by_field_name("parameter") {
                collect_pattern_identifiers(pattern, source, &mut patterns);
            }
            Some(containing_scope(node, language))
        }
        "with_item" | "except_clause" => {
            let mut cursor = node.walk();
            if let Some(pattern) = node
                .child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| {
                    node.named_children(&mut cursor)
                        .find(|child| child.kind() == "as_pattern")
                })
            {
                collect_pattern_identifiers(pattern, source, &mut patterns);
            }
            Some(containing_scope(node, language))
        }
        "function_definition"
        | "function_declaration"
        | "class_definition"
        | "class_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                collect_pattern_identifiers(name, source, &mut patterns);
            }
            Some(containing_scope(node.parent().unwrap_or(node), language))
        }
        _ => None,
    };
    if let Some(scope) = scope {
        for name in patterns {
            push_lexical_binding(bindings, name, scope, LexicalBindingKind::Shadow);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_shadow_bindings(child, source, language, bindings);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_target_bindings(
    node: Node<'_>,
    source: &[u8],
    language: Language,
    target_source_file: Option<&str>,
    test_source_file: Option<&str>,
    normalized_test_source_file: &str,
    bindings: &mut Vec<LexicalBinding>,
    findings: &mut Vec<CouplingFinding>,
) {
    if matches!(
        node.kind(),
        "import_statement" | "import_from_statement" | "import_declaration"
    ) {
        let text = node.utf8_text(source).unwrap_or_default();
        let scope = containing_scope(node, language);
        for binding in import_bindings(node, source, language) {
            let direct_target_import = module_matches_target(
                &binding.module,
                language,
                target_source_file,
                test_source_file,
            );
            let imported_module = if binding.module.ends_with('.') {
                format!("{}{}", binding.module, binding.imported)
            } else {
                format!("{}.{}", binding.module, binding.imported)
            };
            let imports_target = direct_target_import
                || (language == Language::Python
                    && text.trim_start().starts_with("from ")
                    && module_matches_target(
                        &imported_module,
                        language,
                        target_source_file,
                        test_source_file,
                    ));
            if imports_target {
                if binding.imported.starts_with('_') && !binding.imported.starts_with("__") {
                    findings.push(CouplingFinding {
                        kind: CouplingKind::PrivateTargetImport,
                        line: node.start_position().row + 1,
                        column: node.start_position().column + 1,
                        symbol: binding.imported.clone(),
                        evidence: text.trim().to_string(),
                        message: format!("test imports private target symbol {}", binding.imported),
                        test_source_file: normalized_test_source_file.to_string(),
                    });
                }
                push_lexical_binding(
                    bindings,
                    binding.local,
                    scope,
                    LexicalBindingKind::Target {
                        required_path: binding.required_path,
                    },
                );
            } else {
                push_lexical_binding(bindings, binding.local, scope, LexicalBindingKind::Shadow);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_target_bindings(
            child,
            source,
            language,
            target_source_file,
            test_source_file,
            normalized_test_source_file,
            bindings,
            findings,
        );
    }
}

fn lexical_binding_visible_at(binding: &LexicalBinding, use_byte: usize) -> bool {
    use_byte >= binding.scope_start
        && use_byte < binding.scope_end
        && (use_byte >= binding.visible_from
            || binding
                .prelude_range
                .is_some_and(|(start, end)| use_byte >= start && use_byte < end))
}

fn target_reference(node: Node<'_>, source: &[u8], bindings: &[LexicalBinding]) -> Option<String> {
    fn access_path(node: Node<'_>, source: &[u8]) -> Option<(String, Vec<String>, usize)> {
        match node.kind() {
            "identifier" => Some((
                node.utf8_text(source).ok()?.to_string(),
                Vec::new(),
                node.start_byte(),
            )),
            "attribute" | "member_expression" => {
                let object = node
                    .child_by_field_name("object")
                    .or_else(|| node.named_child(0))?;
                let property = node
                    .child_by_field_name("attribute")
                    .or_else(|| node.child_by_field_name("property"))
                    .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)))?;
                let (root, mut path, use_byte) = access_path(object, source)?;
                path.push(property.utf8_text(source).ok()?.to_string());
                Some((root, path, use_byte))
            }
            "subscript" | "subscript_expression" => {
                let object = node
                    .child_by_field_name("value")
                    .or_else(|| node.child_by_field_name("object"))
                    .or_else(|| node.named_child(0))?;
                let index = node
                    .child_by_field_name("subscript")
                    .or_else(|| node.child_by_field_name("index"))
                    .or_else(|| node.named_child(1))?;
                let (root, mut path, use_byte) = access_path(object, source)?;
                path.push(string_literal_text(index, source)?);
                Some((root, path, use_byte))
            }
            "parenthesized_expression"
            | "as_expression"
            | "type_assertion"
            | "non_null_expression"
            | "satisfies_expression"
            | "optional_chain" => node
                .child_by_field_name("expression")
                .or_else(|| node.named_child(0))
                .and_then(|inner| access_path(inner, source)),
            _ => None,
        }
    }

    let (root, path, use_byte) = access_path(node, source)?;
    let smallest_scope = bindings
        .iter()
        .filter(|binding| binding.local == root && lexical_binding_visible_at(binding, use_byte))
        .map(|binding| binding.scope_end.saturating_sub(binding.scope_start))
        .min()?;
    let nearest = bindings.iter().filter(|binding| {
        binding.local == root
            && lexical_binding_visible_at(binding, use_byte)
            && binding.scope_end.saturating_sub(binding.scope_start) == smallest_scope
    });
    let mut matched_required_path: Option<&[String]> = None;
    for binding in nearest {
        match &binding.kind {
            LexicalBindingKind::Shadow => return None,
            LexicalBindingKind::Target { required_path } => {
                if matched_required_path.is_some() {
                    return None;
                }
                matched_required_path = Some(required_path);
            }
        }
    }
    let required_path = matched_required_path?;
    if !path.starts_with(required_path) {
        return None;
    }
    Some(
        std::iter::once(root.as_str())
            .chain(path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("."),
    )
}

fn call_argument(call: Node<'_>, index: usize) -> Option<Node<'_>> {
    call.child_by_field_name("arguments")
        .and_then(|arguments| arguments.named_child(index))
}

fn introspected_binding(
    call: Node<'_>,
    source: &[u8],
    bindings: &[LexicalBinding],
) -> Option<String> {
    let function = call
        .child_by_field_name("function")
        .or_else(|| call.named_child(0))?;
    let function_text = function.utf8_text(source).ok()?;
    if matches!(function_text, "inspect.getsource" | "inspect.getmembers")
        || function_text == "Function.prototype.toString.call"
    {
        return call_argument(call, 0)
            .and_then(|operand| target_reference(operand, source, bindings));
    }
    if matches!(function.kind(), "attribute" | "member_expression") {
        let property = function
            .child_by_field_name("attribute")
            .or_else(|| function.child_by_field_name("property"))
            .or_else(|| function.named_child(function.named_child_count().saturating_sub(1)));
        if property.and_then(|property| property.utf8_text(source).ok()) == Some("toString") {
            return function
                .child_by_field_name("object")
                .or_else(|| function.named_child(0))
                .and_then(|operand| target_reference(operand, source, bindings));
        }
    }
    None
}

fn private_spy(
    call: Node<'_>,
    source: &[u8],
    bindings: &[LexicalBinding],
) -> Option<(String, String)> {
    let function = call
        .child_by_field_name("function")
        .or_else(|| call.named_child(0))?;
    let function_text = function.utf8_text(source).ok()?;
    if !matches!(
        function_text,
        "spyOn" | "jest.spyOn" | "vi.spyOn" | "patch.object" | "mock.patch.object"
    ) {
        return None;
    }
    let binding =
        call_argument(call, 0).and_then(|operand| target_reference(operand, source, bindings))?;
    let member_node = call_argument(call, 1)?;
    if member_node.kind() != "string" {
        return None;
    }
    let member = string_literal_text(member_node, source)?;
    (member.starts_with('_') && !member.starts_with("__")).then_some((binding, member))
}

fn collect_coupling_findings(
    node: Node<'_>,
    source: &[u8],
    bindings: &[LexicalBinding],
    test_source_file: &str,
    findings: &mut Vec<CouplingFinding>,
) {
    if matches!(node.kind(), "attribute" | "member_expression") {
        let object = node
            .child_by_field_name("object")
            .or_else(|| node.named_child(0));
        let property = node
            .child_by_field_name("attribute")
            .or_else(|| node.child_by_field_name("property"))
            .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)));
        if let (Some(object), Some(property)) = (object, property) {
            let property_name = property.utf8_text(source).unwrap_or_default();
            if property_name.starts_with('_') && !property_name.starts_with("__") {
                if let Some(root) = target_reference(object, source, bindings) {
                    let evidence = node.utf8_text(source).unwrap_or_default().to_string();
                    findings.push(CouplingFinding {
                        kind: CouplingKind::PrivateTargetAccess,
                        line: node.start_position().row + 1,
                        column: node.start_position().column + 1,
                        symbol: format!("{root}.{property_name}"),
                        evidence,
                        message: format!(
                            "test reaches through imported target {root} to private member {property_name}"
                        ),
                        test_source_file: test_source_file.to_string(),
                    });
                }
            }
        }
    }
    if matches!(node.kind(), "subscript" | "subscript_expression") {
        let object = node
            .child_by_field_name("value")
            .or_else(|| node.child_by_field_name("object"))
            .or_else(|| node.named_child(0));
        let index = node
            .child_by_field_name("subscript")
            .or_else(|| node.child_by_field_name("index"))
            .or_else(|| node.named_child(1));
        if let (Some(object), Some(index)) = (object, index) {
            let member = index
                .utf8_text(source)
                .unwrap_or_default()
                .trim_matches(['\'', '"']);
            if member.starts_with('_') && !member.starts_with("__") {
                if let Some(root) = target_reference(object, source, bindings) {
                    findings.push(CouplingFinding {
                        kind: CouplingKind::PrivateTargetAccess,
                        line: node.start_position().row + 1,
                        column: node.start_position().column + 1,
                        symbol: format!("{root}.{member}"),
                        evidence: node.utf8_text(source).unwrap_or_default().to_string(),
                        message: format!(
                            "test reaches through imported target {root} to private member {member}"
                        ),
                        test_source_file: test_source_file.to_string(),
                    });
                }
            }
        }
    }
    if node.kind() == "call" || node.kind() == "call_expression" {
        let text = node.utf8_text(source).unwrap_or_default();
        if let Some(binding) = introspected_binding(node, source, bindings) {
            findings.push(CouplingFinding {
                kind: CouplingKind::TargetSourceIntrospection,
                line: node.start_position().row + 1,
                column: node.start_position().column + 1,
                symbol: binding.clone(),
                evidence: text.to_string(),
                message: format!(
                    "test introspects source or runtime structure of target {binding}"
                ),
                test_source_file: test_source_file.to_string(),
            });
        }
        if let Some((binding, member)) = private_spy(node, source, bindings) {
            findings.push(CouplingFinding {
                kind: CouplingKind::PrivateTargetSpy,
                line: node.start_position().row + 1,
                column: node.start_position().column + 1,
                symbol: format!("{binding}.{member}"),
                evidence: text.to_string(),
                message: format!("test spies on private target member {member}"),
                test_source_file: test_source_file.to_string(),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_coupling_findings(child, source, bindings, test_source_file, findings);
    }
}

pub fn analyze_coupling(
    _source_code: &str,
    test_code: &str,
    language: Language,
    test_source_mode: SourceMode,
    target_source_file: Option<&str>,
    test_source_file: Option<&str>,
) -> Result<Vec<CouplingFinding>, String> {
    let mut test_parser = parser_for_mode(test_source_mode)?;
    let test_tree = test_parser
        .parse(test_code, None)
        .ok_or_else(|| "test-quality test parser produced no tree".to_string())?;
    if test_tree.root_node().has_error() {
        return Err(
            "test-quality coupling analysis rejected malformed authoritative test source".into(),
        );
    }
    let normalized_test_source_file = test_source_file
        .map(|path| {
            normalized_path(Path::new(path))
                .to_string_lossy()
                .replace('\\', "/")
        })
        .unwrap_or_else(|| "<inline>".into());

    let mut bindings = Vec::new();
    let mut findings = Vec::new();
    collect_target_bindings(
        test_tree.root_node(),
        test_code.as_bytes(),
        language,
        target_source_file,
        test_source_file,
        &normalized_test_source_file,
        &mut bindings,
        &mut findings,
    );
    collect_shadow_bindings(
        test_tree.root_node(),
        test_code.as_bytes(),
        language,
        &mut bindings,
    );
    collect_coupling_findings(
        test_tree.root_node(),
        test_code.as_bytes(),
        &bindings,
        &normalized_test_source_file,
        &mut findings,
    );
    findings.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.column.cmp(&right.column))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    findings.dedup_by(|left, right| {
        left.kind == right.kind
            && left.line == right.line
            && left.column == right.column
            && left.symbol == right.symbol
    });
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(name: &str, line: usize, end_line: usize) -> FunctionInfo {
        FunctionInfo {
            name: name.into(),
            params: Vec::new(),
            return_type: None,
            type_parameters: Vec::new(),
            type_parameter_constraints: BTreeMap::new(),
            line,
            end_line,
            complexity: 1,
            cognitive_complexity: 0,
            max_nesting_depth: 0,
            complexity_breakdown: BTreeMap::new(),
            is_method: false,
            is_nested: false,
            is_exported: true,
            declared_properties: Vec::new(),
            predicate_seeds: Vec::new(),
            effects: Vec::new(),
            invocation_target: None,
            returned_callables: Vec::new(),
        }
    }

    #[test]
    fn plans_boundary_mutants_across_surfaces() {
        let code = "def first(value: int) -> int:\n    return 1 if value < 0 else 0\n\ndef second(value: int) -> int:\n    return 1 if value >= 2 else 0\n";
        let first = function("first", 1, 2);
        let second = function("second", 4, 5);
        let planned = plan_mutations(
            code,
            Language::Python,
            SourceMode::Python,
            &[&first, &second],
            2,
        )
        .unwrap();
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].surface_id, "first:1");
        assert_eq!(planned[0].replacement, "<=");
        assert_eq!(planned[1].surface_id, "second:4");
        assert_eq!(planned[1].replacement, ">");
    }

    #[test]
    fn mutation_application_checks_original_bytes() {
        let code = "export function clamp(n: number) { return n > 1 ? 1 : n }";
        let clamp = function("clamp", 1, 1);
        let candidate = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&clamp],
            1,
        )
        .unwrap()
        .remove(0);
        let mutated = apply_mutation(code, &candidate).unwrap();
        assert!(mutated.contains("n >= 1"));
    }

    #[test]
    fn same_line_callables_keep_exact_surface_attribution() {
        let code = "export const first = (value: number) => value < 0; export const second = (value: number) => value > 0;";
        let first = function("first", 1, 1);
        let second = function("second", 1, 1);
        let planned = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&first, &second],
            2,
        )
        .unwrap();
        assert_eq!(
            planned
                .iter()
                .map(|candidate| candidate.surface_id.as_str())
                .collect::<Vec<_>>(),
            ["first:1", "second:1"]
        );
    }

    #[test]
    fn nested_unselected_callable_shields_outer_surface() {
        let code =
            "export function outer() { const inner = (value: number) => value < 0; return 1; }";
        let outer = function("outer", 1, 1);
        let planned = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&outer],
            usize::MAX,
        )
        .unwrap();
        assert!(planned.is_empty(), "{planned:#?}");
    }

    #[test]
    fn same_line_class_methods_keep_class_qualified_surfaces() {
        let code = "export class First { same(value: number) { return value < 0; } } export class Second { same(value: number) { return value > 0; } }";
        let first = function("First#same", 1, 1);
        let second = function("Second#same", 1, 1);
        let planned = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&first, &second],
            2,
        )
        .unwrap();
        assert_eq!(
            planned
                .iter()
                .map(|candidate| candidate.surface_id.as_str())
                .collect::<Vec<_>>(),
            ["First#same:1", "Second#same:1"]
        );
    }

    #[test]
    fn typescript_type_level_boolean_literals_are_not_mutated() {
        let code = "export function enabled(value: true | false): true { return true; }";
        let enabled = function("enabled", 1, 1);
        let planned = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&enabled],
            usize::MAX,
        )
        .unwrap();
        assert_eq!(planned.len(), 1, "{planned:#?}");
        assert_eq!(planned[0].operator, MutationOperator::BooleanLiteral);
        assert_eq!(planned[0].original, "true");
        assert!(planned[0].column > code.find("return").unwrap());
    }

    #[test]
    fn typescript_parenthesized_condition_negation_remains_parseable() {
        let code =
            "export function normalize(code?: string) { if (!code) return 'unknown'; return code; }";
        let normalize = function("normalize", 1, 1);
        let planned = plan_mutations(
            code,
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&normalize],
            usize::MAX,
        )
        .unwrap();
        let candidate = planned
            .iter()
            .find(|candidate| candidate.operator == MutationOperator::ConditionNegation)
            .expect("condition-negation candidate");
        assert_eq!(candidate.original, "(!code)");
        assert_eq!(candidate.replacement, "(!(!code))");
        let mutated = apply_mutation(code, candidate).unwrap();
        let mut parser = parser_for_mode(SourceMode::TypeScript).unwrap();
        let tree = parser.parse(&mutated, None).unwrap();
        assert!(!tree.root_node().has_error(), "{mutated}");
    }

    #[test]
    fn parser_errors_are_attributed_without_echoing_source_bodies() {
        let public = function("public", 1, 1);
        let planning_error = plan_mutations(
            "export function public( { const password = 'source-secret';",
            Language::TypeScript,
            SourceMode::TypeScript,
            &[&public],
            1,
        )
        .unwrap_err();
        assert_eq!(
            planning_error,
            "test-quality mutation planning rejected malformed target source"
        );

        let coupling_error = analyze_coupling(
            "export const service = {};",
            "import { service from './service'; const token = 'test-secret';",
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap_err();
        assert_eq!(
            coupling_error,
            "test-quality coupling analysis rejected malformed authoritative test source"
        );
    }

    #[test]
    fn coupling_is_target_aware() {
        let source = "export const service = { run() { return 1 } }";
        let tests = "import { service } from './service'\nimport { service as externalService } from 'service'\nexpect((service as any)._cache).toBe(1)\nexpect(externalService._cache).toBe('ok')\n";
        let findings = analyze_coupling(
            source,
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "service._cache");
        assert_eq!(findings[0].test_source_file, "/repo/service.test.ts");
    }

    #[test]
    fn side_effect_import_does_not_create_a_target_binding() {
        let findings = analyze_coupling(
            "export const service = {}",
            "import './service'\nexpect((service as any)._cache).toBe(1)\n",
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn type_only_imports_do_not_create_runtime_target_bindings() {
        let tests = "import type { Service } from './service'\nimport { type Config, publicValue } from './service'\nexpect((Service as any)._cache).toBe(1)\nexpect((Config as any)._cache).toBe(1)\nexpect((publicValue as any)._cache).toBe(1)\n";
        let findings = analyze_coupling(
            "export type Service = {}; export type Config = {}; export const publicValue = {};",
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "publicValue._cache");
    }

    #[test]
    fn typescript_import_kinds_do_not_treat_private_local_aliases_as_private_imports() {
        let tests = "import _service, * as _namespace from './service'\nimport { public as _public } from './service'\nexpect(_service._cache).toBe(1)\nexpect(_namespace._cache).toBe(1)\nexpect(_public._cache).toBe(1)\n";
        let findings = analyze_coupling(
            "export default {}; export const public = {};",
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.kind == CouplingKind::PrivateTargetAccess)
                .count(),
            3,
            "{findings:#?}"
        );
        assert!(
            findings
                .iter()
                .all(|finding| finding.kind != CouplingKind::PrivateTargetImport),
            "{findings:#?}"
        );
    }

    #[test]
    fn python_plain_imports_resolve_each_module_independently() {
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            "import target as t, support as s\nassert t._cache\nassert s._cache\n",
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "t._cache");
    }

    #[test]
    fn python_dotted_import_preserves_the_resolved_module_path() {
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            "import target.submodule as implementation\nassert implementation._cache\n",
            Language::Python,
            SourceMode::Python,
            Some("/repo/target/submodule.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "implementation._cache");
    }

    #[test]
    fn unaliased_dotted_import_requires_the_qualified_target_path() {
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            "import pkg.target\nassert pkg.target._cache\nassert pkg._package_cache\n",
            Language::Python,
            SourceMode::Python,
            Some("/repo/pkg/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "pkg.target._cache");
    }

    #[test]
    fn typescript_local_bindings_shadow_target_imports_lexically() {
        let tests = "import { service } from './service'\nexpect(service._outside).toBe(1)\nfunction parameter(service: any) { return service._parameter }\nfunction local() { const service = {}; return service._local }\nexpect(service._after).toBe(1)\n";
        let findings = analyze_coupling(
            "export const service = {};",
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.symbol.as_str())
                .collect::<Vec<_>>(),
            ["service._outside", "service._after"],
            "{findings:#?}"
        );
    }

    #[test]
    fn python_local_bindings_shadow_target_imports_lexically() {
        let tests = "import target\nassert target._outside\ndef parameter(target):\n    return target._parameter\ndef local():\n    target = object()\n    return target._local\nassert target._after\n";
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.symbol.as_str())
                .collect::<Vec<_>>(),
            ["target._outside", "target._after"],
            "{findings:#?}"
        );
    }

    #[test]
    fn nearer_existing_python_module_wins_direct_script_resolution() {
        let root = tempfile::tempdir().unwrap();
        let tests_dir = root.path().join("tests");
        std::fs::create_dir(&tests_dir).unwrap();
        let target = root.path().join("target.py");
        std::fs::write(&target, "def public():\n    return 1\n").unwrap();
        std::fs::write(
            tests_dir.join("target.py"),
            "def unrelated():\n    return 2\n",
        )
        .unwrap();
        let test_file = tests_dir.join("test_target.py");
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            "import target\nassert target._cache\n",
            Language::Python,
            SourceMode::Python,
            target.to_str(),
            test_file.to_str(),
        )
        .unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn python_comprehension_binding_shadows_body_and_filters_not_iterable() {
        let tests = "import target\nvalues = [target._body for target in target._items if target._filter]\nassert target._outside\n";
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.symbol.as_str())
                .collect::<Vec<_>>(),
            ["target._items", "target._outside"],
            "{findings:#?}"
        );
    }

    #[test]
    fn typescript_var_shadows_at_function_scope() {
        let tests = "import { service } from './service'\nfunction local() { if (true) { var service = {}; } return service._local }\nexpect(service._outside).toBe(1)\n";
        let findings = analyze_coupling(
            "export const service = {};",
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "service._outside");
    }

    #[test]
    fn arbitrary_wrapper_descendants_do_not_prove_target_roots() {
        let tests = "import inspect\nimport target\ninspect.getsource(lambda: target.public)\nwrapper(target)._cache\n";
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn qualified_spies_require_target_root_and_private_second_literal() {
        let tests = "import * as service from './service'\nconst publicName = '_dynamic'\njest.spyOn(service, '_jest')\nvi.spyOn(service.member, '_vi')\njest.spyOn(service, publicName, '_third')\njest.spyOn(wrapper(service), '_wrapped')\n";
        let findings = analyze_coupling(
            "export const member = {};",
            tests,
            Language::TypeScript,
            SourceMode::TypeScript,
            Some("/repo/service.ts"),
            Some("/repo/service.test.ts"),
        )
        .unwrap();
        let spies = findings
            .iter()
            .filter(|finding| finding.kind == CouplingKind::PrivateTargetSpy)
            .collect::<Vec<_>>();
        assert_eq!(spies.len(), 2, "{findings:#?}");
        assert_eq!(spies[0].symbol, "service._jest");
        assert_eq!(spies[1].symbol, "service.member._vi");
    }

    #[test]
    fn every_coupling_kind_retains_normalized_test_provenance() {
        let tests = "import inspect\nfrom unittest import mock\nfrom .target import _secret, public\nassert public._cache\nassert inspect.getsource(public.member)\nmock.patch.object(public, '_hidden')\n";
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/tests/target.py"),
            Some("/repo/tests/../tests/./test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 4, "{findings:#?}");
        for kind in [
            CouplingKind::PrivateTargetAccess,
            CouplingKind::PrivateTargetImport,
            CouplingKind::PrivateTargetSpy,
            CouplingKind::TargetSourceIntrospection,
        ] {
            assert!(
                findings.iter().any(|finding| finding.kind == kind),
                "missing {kind:?}: {findings:#?}"
            );
        }
        assert!(findings
            .iter()
            .all(|finding| { finding.test_source_file == "/repo/tests/test_target.py" }));
    }

    #[test]
    fn unrelated_source_introspection_is_ignored() {
        let tests =
            "import inspect\nimport target\nimport other\nassert inspect.getsource(other.public)\n";
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn python_relative_module_import_creates_target_binding() {
        let findings = analyze_coupling(
            "def public():\n    return 1\n",
            "from . import target\nassert target._cache\n",
            Language::Python,
            SourceMode::Python,
            Some("/repo/package/target.py"),
            Some("/repo/package/test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "target._cache");
    }

    #[test]
    fn coupling_uses_tsx_test_parser_mode() {
        let tests = "import { service } from './service'\nconst view = <div />\nexpect((service as any)._cache).toBe(view)\n";
        let findings = analyze_coupling(
            "export const service = {}",
            tests,
            Language::TypeScript,
            SourceMode::Tsx,
            Some("/repo/service.ts"),
            Some("/repo/service.test.tsx"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].symbol, "service._cache");
    }

    #[test]
    fn source_introspection_is_advisory_evidence() {
        let source = "def public(value: int) -> int:\n    return value\n";
        let tests =
            "import inspect\nimport target\nassert 'return' in inspect.getsource(target.public)\n";
        let findings = analyze_coupling(
            source,
            tests,
            Language::Python,
            SourceMode::Python,
            Some("/repo/target.py"),
            Some("/repo/test_target.py"),
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].kind, CouplingKind::TargetSourceIntrospection);
    }
}
