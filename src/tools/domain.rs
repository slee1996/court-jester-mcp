//! Pure repository-derived domains and verification planning.
//!
//! This module deliberately does not read files or execute code.  Analysis and
//! verification feed it syntax metadata and observed examples; synthesis only
//! consumes the resulting plan.

use std::collections::{BTreeMap, HashMap};

use crate::types::*;

const ALIAS_DEPTH_LIMIT: usize = 8;

fn source(kind: DomainSourceKind, symbol: Option<&str>, line: Option<usize>) -> DomainSource {
    DomainSource {
        kind,
        symbol: symbol.map(str::to_string),
        source_file: None,
        line,
    }
}

fn split_top_level(raw: &str, delimiter: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut quote = None;
    for (index, ch) in raw.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '[' | '(' | '{' | '<' => depth += 1,
            ']' | ')' | '}' | '>' => depth -= 1,
            _ if ch == delimiter && depth == 0 => {
                let item = raw[start..index].trim();
                if !item.is_empty() {
                    out.push(item.to_string());
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = raw[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn literal(raw: &str, language: &Language) -> Option<DomainLiteral> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let bytes_literal = matches!(language, Language::Python) && text.starts_with('b');
    let json_value = match text {
        "true" | "True" => Some(serde_json::Value::Bool(true)),
        "false" | "False" => Some(serde_json::Value::Bool(false)),
        "null" | "None" => Some(serde_json::Value::Null),
        _ if bytes_literal => None,
        _ if (text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\''))
            || (text.starts_with('`') && text.ends_with('`')) =>
        {
            let inner = &text[1..text.len().saturating_sub(1)];
            Some(serde_json::Value::String(
                inner.replace("\\'", "'").replace("\\\"", "\""),
            ))
        }
        _ => text
            .parse::<i64>()
            .ok()
            .map(serde_json::Value::from)
            .or_else(|| {
                text.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
            }),
    };
    if json_value.is_none() && !bytes_literal {
        return None;
    }
    let expression = if json_value.as_ref().is_some_and(serde_json::Value::is_null) {
        match language {
            Language::Python => "None".to_string(),
            Language::TypeScript => "null".to_string(),
        }
    } else {
        text.to_string()
    };
    Some(DomainLiteral {
        expression,
        json_value,
    })
}

fn literal_domain(parts: &[String], language: &Language) -> DomainNode {
    DomainNode::Literal(
        parts
            .iter()
            .filter_map(|part| literal(part, language))
            .collect(),
    )
}

fn nullable_or_union(
    parts: Vec<String>,
    aliases: &[TypeAliasInfo],
    classes: &[ClassInfo],
    language: &Language,
    stack: &mut Vec<String>,
    depth: usize,
) -> DomainNode {
    let has_none = parts
        .iter()
        .any(|part| matches!(part.trim(), "None" | "null" | "undefined"));
    let non_null = parts
        .into_iter()
        .filter(|part| !matches!(part.trim(), "None" | "null" | "undefined"))
        .collect::<Vec<_>>();
    let mut nodes = non_null
        .iter()
        .map(|part| domain_inner(part, aliases, classes, language, stack, depth))
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return DomainNode::Opaque("null_only".into());
    }
    let base = if nodes.len() == 1 {
        nodes.remove(0)
    } else {
        DomainNode::Union(nodes)
    };
    if has_none {
        DomainNode::Nullable(Box::new(base))
    } else {
        base
    }
}

fn domain_inner(
    raw: &str,
    aliases: &[TypeAliasInfo],
    classes: &[ClassInfo],
    language: &Language,
    stack: &mut Vec<String>,
    depth: usize,
) -> DomainNode {
    let text = raw.trim();
    if text.is_empty() {
        return DomainNode::Any;
    }
    if depth > ALIAS_DEPTH_LIMIT {
        return DomainNode::Opaque("recursive_or_depth_limit".into());
    }
    let alias = aliases.iter().find(|item| item.name == text);
    if let Some(alias) = alias {
        if stack.iter().any(|name| name == text) {
            return DomainNode::Opaque("recursive_or_depth_limit".into());
        }
        stack.push(text.to_string());
        let result = domain_inner(
            &alias.type_annotation,
            aliases,
            classes,
            language,
            stack,
            depth + 1,
        );
        stack.pop();
        return result;
    }
    if text.starts_with("Literal[") {
        let inner = text
            .strip_prefix("Literal[")
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or("");
        return literal_domain(&split_top_level(inner, ','), language);
    }
    let union_parts = split_top_level(text, '|');
    if union_parts.len() > 1 {
        return nullable_or_union(union_parts, aliases, classes, language, stack, depth);
    }
    if text.starts_with("Optional[") {
        let inner = text
            .strip_prefix("Optional[")
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or("");
        let domain = domain_inner(inner, aliases, classes, language, stack, depth);
        return match domain {
            DomainNode::Nullable(_) => domain,
            _ => DomainNode::Nullable(Box::new(domain)),
        };
    }
    if text.starts_with("Union[") {
        let inner = text
            .strip_prefix("Union[")
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or("");
        return nullable_or_union(
            split_top_level(inner, ','),
            aliases,
            classes,
            language,
            stack,
            depth,
        );
    }
    if let Some(inner) = text.strip_suffix("[]") {
        return DomainNode::Array(Box::new(domain_inner(
            inner, aliases, classes, language, stack, depth,
        )));
    }
    for (prefix, ctor) in [
        ("list[", 0u8),
        ("List[", 0),
        ("Array<", 0),
        ("Set[", 1),
        ("set[", 1),
        ("Tuple[", 2),
        ("tuple[", 2),
    ] {
        if let Some(stripped) = text.strip_prefix(prefix) {
            let inner = stripped
                .strip_suffix(if prefix.ends_with('<') { '>' } else { ']' })
                .unwrap_or("");
            let items = split_top_level(inner, ',');
            return match ctor {
                0 => DomainNode::Array(Box::new(domain_inner(
                    items.first().map(String::as_str).unwrap_or("Any"),
                    aliases,
                    classes,
                    language,
                    stack,
                    depth,
                ))),
                1 => DomainNode::Set(Box::new(domain_inner(
                    items.first().map(String::as_str).unwrap_or("Any"),
                    aliases,
                    classes,
                    language,
                    stack,
                    depth,
                ))),
                _ => DomainNode::Tuple(
                    items
                        .into_iter()
                        .map(|item| domain_inner(&item, aliases, classes, language, stack, depth))
                        .collect(),
                ),
            };
        }
    }
    if text.starts_with('[') && text.ends_with(']') {
        return DomainNode::Tuple(
            split_top_level(&text[1..text.len() - 1], ',')
                .into_iter()
                .map(|item| domain_inner(&item, aliases, classes, language, stack, depth))
                .collect(),
        );
    }
    if text.starts_with("dict[")
        || text.starts_with("Dict[")
        || text.starts_with("Record<")
        || text.starts_with("Map<")
    {
        let inner = text
            .split_once('[')
            .or_else(|| text.split_once('<'))
            .and_then(|(_, rest)| rest.strip_suffix(if text.contains('[') { ']' } else { '>' }))
            .unwrap_or("");
        let items = split_top_level(inner, ',');
        return DomainNode::Map(
            Box::new(domain_inner(
                items.first().map(String::as_str).unwrap_or("String"),
                aliases,
                classes,
                language,
                stack,
                depth,
            )),
            Box::new(domain_inner(
                items.get(1).map(String::as_str).unwrap_or("Any"),
                aliases,
                classes,
                language,
                stack,
                depth,
            )),
        );
    }
    if text.starts_with('{') && text.ends_with('}') {
        let body = &text[1..text.len() - 1];
        let mut items = split_top_level(body, ',');
        if items.len() == 1 && items[0].trim() == body.trim() {
            items = split_top_level(body, ';');
        }
        if items.len() == 1 && items[0].trim() == body.trim() {
            items = split_top_level(body, '\n');
        }
        let mut fields = Vec::new();
        for item in items {
            let Some((raw_name, annotation)) = item.split_once(':') else {
                continue;
            };
            let raw_name = raw_name
                .trim()
                .strip_prefix("readonly ")
                .unwrap_or(raw_name.trim());
            let optional = raw_name.ends_with('?');
            let name = raw_name
                .trim_end_matches('?')
                .trim_matches(['"', '\'', '`']);
            fields.push(DomainField {
                name: name.to_string(),
                domain: domain_inner(annotation, aliases, classes, language, stack, depth),
                optional,
            });
        }
        return DomainNode::Object(fields);
    }
    if let Some(class) = classes.iter().find(|class| class.name == text) {
        if stack.iter().any(|name| name == text) {
            return DomainNode::Opaque("recursive_or_depth_limit".into());
        }
        stack.push(text.to_string());
        let fields = class
            .fields
            .iter()
            .map(|field| DomainField {
                name: field.name.clone(),
                domain: domain_inner(
                    field.type_annotation.as_deref().unwrap_or("Any"),
                    aliases,
                    classes,
                    language,
                    stack,
                    depth + 1,
                ),
                optional: field.optional || field.has_default,
            })
            .collect();
        stack.pop();
        return DomainNode::Object(fields);
    }
    if let Some(values) = split_top_level(text, ',')
        .into_iter()
        .filter_map(|item| literal(&item, language))
        .collect::<Vec<_>>()
        .into_iter()
        .next()
    {
        // A single literal is a closed domain; this branch mostly handles aliases
        // normalized by the analyzer to one literal.
        return DomainNode::Literal(vec![values]);
    }
    match text.trim_matches(&[' ', '"', '\'', '`'][..]) {
        "Any" | "unknown" => DomainNode::Any,
        "bool" | "boolean" => DomainNode::Boolean,
        "int" | "number" | "bigint" => {
            if text == "number" || text == "bigint" {
                DomainNode::Float
            } else {
                DomainNode::Integer
            }
        }
        "float" => DomainNode::Float,
        "str" | "string" => DomainNode::String,
        "bytes" | "Buffer" | "Uint8Array" => DomainNode::Bytes,
        "None" | "null" | "undefined" => DomainNode::Opaque("null_only".into()),
        _ => DomainNode::Opaque(format!("unresolved:{text}")),
    }
}

pub fn domain_for_annotation(
    annotation: Option<&str>,
    aliases: &[TypeAliasInfo],
    classes: &[ClassInfo],
    language: &Language,
) -> DomainNode {
    domain_inner(
        annotation.unwrap_or("Any"),
        aliases,
        classes,
        language,
        &mut Vec::new(),
        0,
    )
}
/// Returns true when a parameter represents an injectable dependency rather
/// than a value domain.  This intentionally examines resolved type structure
/// and default expressions; parameter names are never used as a signal.
pub fn is_dependency_shaped(
    param: &ParamInfo,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
) -> bool {
    param.type_annotation.as_deref().is_some_and(|annotation| {
        annotation_is_dependency(annotation, classes, aliases, &mut Vec::new())
    }) || param
        .default_value
        .as_deref()
        .is_some_and(is_nonliteral_dependency_default)
}

fn annotation_is_dependency(
    annotation: &str,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    stack: &mut Vec<String>,
) -> bool {
    let text = annotation.trim();
    if text.is_empty() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if lower == "callable"
        || lower.starts_with("callable[")
        || lower == "function"
        || lower == "functiontype"
        || lower.contains("=>")
        || lower.starts_with("(...")
    {
        return true;
    }
    if let Some(alias) = aliases.iter().find(|alias| alias.name == text) {
        if stack.iter().any(|name| name == text) {
            return false;
        }
        stack.push(text.to_string());
        let result = annotation_is_dependency(&alias.type_annotation, classes, aliases, stack);
        stack.pop();
        return result;
    }
    if text.starts_with('{') && text.ends_with('}') {
        return split_top_level(text[1..text.len().saturating_sub(1)].trim(), ',')
            .into_iter()
            .filter_map(|field| field.split_once(':').map(|(_, value)| value.to_string()))
            .any(|field| annotation_is_dependency(&field, classes, aliases, stack));
    }
    if let Some(class) = classes.iter().find(|class| class.name == text) {
        if stack.iter().any(|name| name == text) {
            return false;
        }
        stack.push(text.to_string());
        let result = class.fields.iter().any(|field| {
            field.type_annotation.as_deref().is_some_and(|annotation| {
                annotation_is_dependency(annotation, classes, aliases, stack)
            })
        });
        stack.pop();
        return result;
    }
    false
}

fn is_nonliteral_dependency_default(default_value: &str) -> bool {
    let value = default_value.trim();
    if value.is_empty() || literal(value, &Language::TypeScript).is_some() {
        return false;
    }
    let mut body = value;
    if let Some(stripped) = body.strip_prefix("new ") {
        body = stripped.trim();
    }
    let Some(first) = body.chars().next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    if body.ends_with(')') {
        return body.contains('(');
    }
    body.chars()
        .all(|ch| ch == '_' || ch == '.' || ch.is_ascii_alphanumeric())
}

fn resolve_alias_annotation(
    annotation: &str,
    aliases: &[TypeAliasInfo],
    stack: &mut Vec<String>,
) -> Option<String> {
    let text = annotation.trim();
    let alias = aliases.iter().find(|alias| alias.name == text)?;
    if stack.iter().any(|name| name == text) {
        return None;
    }
    stack.push(text.to_string());
    let result = resolve_alias_annotation(&alias.type_annotation, aliases, stack)
        .or_else(|| Some(alias.type_annotation.trim().to_string()));
    stack.pop();
    result
}

fn callable_return_annotation(
    annotation: &str,
    aliases: &[TypeAliasInfo],
) -> Option<(String, bool)> {
    let mut resolved = annotation.trim().to_string();
    let mut stack = Vec::new();
    if let Some(alias) = resolve_alias_annotation(&resolved, aliases, &mut stack) {
        resolved = alias;
    }
    let lower = resolved.to_ascii_lowercase();
    if lower == "callable" || lower == "function" || lower == "functiontype" {
        return None;
    }
    if lower.starts_with("callable[") {
        let inner = resolved
            .split_once('[')
            .and_then(|(_, value)| value.strip_suffix(']'))
            .unwrap_or("");
        let parts = split_top_level(inner, ',');
        let result = parts.last()?.trim();
        if result == "..." || result.is_empty() {
            return None;
        }
        return Some((result.to_string(), false));
    }
    let arrow = resolved.find("=>")?;
    let result = resolved[arrow + 2..].trim();
    if result.is_empty() {
        return None;
    }
    let (is_async, result) = if result.starts_with("Promise<") && result.ends_with('>') {
        (true, result[8..result.len() - 1].trim())
    } else {
        (false, result)
    };
    Some((result.to_string(), is_async))
}

fn strip_outer_parentheses(mut text: &str) -> &str {
    loop {
        let bytes = text.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'(' || bytes[bytes.len() - 1] != b')' {
            return text;
        }
        let mut depth = 0usize;
        let mut encloses = true;
        for (index, character) in text.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && index + character.len_utf8() < text.len() {
                        encloses = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !encloses || depth != 0 {
            return text;
        }
        text = text[1..text.len() - 1].trim();
    }
}

fn dependency_object_fields(
    annotation: &str,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    stack: &mut Vec<String>,
) -> Option<Vec<(String, String, bool)>> {
    let text = strip_outer_parentheses(annotation.trim());
    if let Some(alias) = aliases.iter().find(|alias| alias.name == text) {
        if stack.iter().any(|name| name == text) {
            return None;
        }
        stack.push(text.to_string());
        let result = dependency_object_fields(&alias.type_annotation, classes, aliases, stack);
        stack.pop();
        return result;
    }
    if let Some(class) = classes.iter().find(|class| class.name == text) {
        if class.fields.is_empty() || stack.iter().any(|name| name == text) {
            return None;
        }
        stack.push(text.to_string());
        let fields = class
            .fields
            .iter()
            .filter_map(|field| {
                field
                    .type_annotation
                    .as_ref()
                    .map(|annotation| (field.name.clone(), annotation.clone(), field.optional))
            })
            .collect();
        stack.pop();
        return Some(fields);
    }
    if !(text.starts_with('{') && text.ends_with('}')) {
        return None;
    }
    let body = &text[1..text.len() - 1];
    let mut members = split_top_level(body, ',');
    if members.len() == 1 && members[0].trim() == body.trim() {
        members = split_top_level(body, ';');
    }
    let mut fields = Vec::new();
    for member in members {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        let (raw_name, annotation) = member.split_once(':')?;
        let optional = raw_name.trim().ends_with('?');
        let name = raw_name
            .trim()
            .trim_end_matches('?')
            .trim_matches(['"', '\'', '`'])
            .to_string();
        if name.is_empty() {
            return None;
        }
        fields.push((name, annotation.trim().to_string(), optional));
    }
    (!fields.is_empty()).then_some(fields)
}

fn callback_substitute(
    return_type: &str,
    is_async: bool,
    language: &Language,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
) -> Option<DomainLiteral> {
    let value = deterministic_domain_literal(return_type, aliases, classes, language)?;
    let expression = match language {
        Language::Python => format!("(lambda *args, **kwargs: {})", value.expression),
        Language::TypeScript if is_async => {
            format!(
                "(async (..._args: unknown[]) => Promise.resolve({}))",
                value.expression
            )
        }
        Language::TypeScript => format!("((..._args: unknown[]) => {})", value.expression),
    };
    Some(DomainLiteral {
        expression,
        json_value: None,
    })
}

fn deterministic_service_literal(
    annotation: &str,
    aliases: &[TypeAliasInfo],
    classes: &[ClassInfo],
    language: &Language,
    stack: &mut Vec<String>,
) -> Option<DomainLiteral> {
    let fields = dependency_object_fields(annotation, classes, aliases, stack)?;
    let mut rendered = Vec::new();
    for (name, annotation, optional) in fields {
        let value = if let Some((return_type, is_async)) =
            callable_return_annotation(&annotation, aliases)
        {
            callback_substitute(&return_type, is_async, language, classes, aliases)
        } else if dependency_object_fields(&annotation, classes, aliases, stack).is_some() {
            deterministic_service_literal(&annotation, aliases, classes, language, stack)
        } else {
            deterministic_domain_literal(&annotation, aliases, classes, language)
        };
        let Some(value) = value else {
            if optional {
                continue;
            }
            return None;
        };
        let key = serde_json::to_string(&name).ok()?;
        rendered.push(format!("{key}: {}", value.expression));
    }
    if rendered.is_empty() {
        return None;
    }
    let expression = match language {
        Language::TypeScript => format!("{{{}}}", rendered.join(", ")),
        Language::Python => format!(
            "type(\"CourtJesterSafeDependency\", (), {{{}}})()",
            rendered.join(", ")
        ),
    };
    Some(DomainLiteral {
        expression,
        json_value: None,
    })
}

fn deterministic_domain_literal(
    annotation: &str,
    aliases: &[TypeAliasInfo],
    classes: &[ClassInfo],
    language: &Language,
) -> Option<DomainLiteral> {
    let normalized = annotation.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "void" | "none" | "undefined") {
        return Some(DomainLiteral {
            expression: match language {
                Language::Python => "None".into(),
                Language::TypeScript => "undefined".into(),
            },
            json_value: None,
        });
    }
    let domain = domain_for_annotation(Some(annotation), aliases, classes, language);
    domain_literals(&domain, language)
        .into_iter()
        .next()
        .or_else(|| {
            let (expression, json_value) = match domain {
                DomainNode::Boolean => (
                    render_json_literal(&serde_json::Value::Bool(false), language),
                    Some(serde_json::Value::Bool(false)),
                ),
                DomainNode::Integer | DomainNode::Float => {
                    ("0".to_string(), Some(serde_json::Value::from(0)))
                }
                DomainNode::String => (
                    render_json_literal(&serde_json::Value::String(String::new()), language),
                    Some(serde_json::Value::String(String::new())),
                ),
                DomainNode::Array(_) | DomainNode::Tuple(_) | DomainNode::Set(_) => {
                    ("[]".to_string(), Some(serde_json::Value::Array(Vec::new())))
                }
                DomainNode::Object(_) => ("{}".to_string(), Some(serde_json::json!({}))),
                _ => return None,
            };
            Some(DomainLiteral {
                expression,
                json_value,
            })
        })
}

/// Build a deterministic, no-I/O replacement for an omitted dependency
/// argument.  Callable values retain their callable shape and async
/// TypeScript callbacks resolve a generated return-domain value.
pub fn safe_dependency_substitute(
    param: &ParamInfo,
    language: &Language,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
) -> Result<DomainLiteral, UnsafeDefaultReason> {
    let annotation = param
        .type_annotation
        .as_deref()
        .ok_or(UnsafeDefaultReason::Untyped)?;
    if !is_dependency_shaped(param, classes, aliases) {
        return Err(UnsafeDefaultReason::Unsynthesizable);
    }
    if let Some((return_type, is_async)) = callable_return_annotation(annotation, aliases) {
        return callback_substitute(&return_type, is_async, language, classes, aliases)
            .ok_or(UnsafeDefaultReason::SubstituteUnavailable);
    }
    let mut stack = Vec::new();
    if dependency_object_fields(annotation, classes, aliases, &mut stack).is_some() {
        return deterministic_service_literal(
            annotation,
            aliases,
            classes,
            language,
            &mut Vec::new(),
        )
        .ok_or(UnsafeDefaultReason::SubstituteUnavailable);
    }
    deterministic_domain_literal(annotation, aliases, classes, language)
        .map(|mut value| {
            if matches!(language, Language::Python)
                && annotation.to_ascii_lowercase().contains("callable")
            {
                value.expression = format!("(lambda *args, **kwargs: {})", value.expression);
                value.json_value = None;
            }
            value
        })
        .ok_or(UnsafeDefaultReason::SubstituteUnavailable)
}

/// Normalize omitted/default-activating dependency slots before input
/// classification.  The returned sources identify substitutions so callers
/// can preserve provenance in the plan.
pub fn normalize_dependency_arguments(
    params: &[ParamInfo],
    arguments: &mut PlannedArguments,
    language: &Language,
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
) -> Result<Vec<(String, DomainSource)>, UnsafeDefaultReason> {
    let mut sources = Vec::new();
    let mut positional_index = 0usize;
    for param in params {
        let value = if param.keyword_only {
            arguments.named.get(&param.name)
        } else {
            let current = arguments.positional.get(positional_index);
            positional_index += 1;
            current
        };
        let default_activating = value.is_none()
            || value.is_some_and(|value| {
                language == &Language::TypeScript
                    && matches!(value.expression.trim(), "undefined" | "void 0")
            });
        if !default_activating || !is_dependency_shaped(param, classes, aliases) {
            continue;
        }
        let substitute = safe_dependency_substitute(param, language, classes, aliases)?;
        if param.keyword_only {
            arguments
                .named
                .insert(param.name.clone(), substitute.clone());
        } else if positional_index > 0 {
            let index = positional_index - 1;
            match index.cmp(&arguments.positional.len()) {
                std::cmp::Ordering::Equal => arguments.positional.push(substitute.clone()),
                std::cmp::Ordering::Less => {
                    arguments.positional[index] = substitute.clone();
                }
                std::cmp::Ordering::Greater => {
                    arguments.positional.resize_with(index, || DomainLiteral {
                        expression: match language {
                            Language::Python => "None".into(),
                            Language::TypeScript => "undefined".into(),
                        },
                        json_value: None,
                    });
                    arguments.positional.push(substitute.clone());
                }
            }
        }
        sources.push((
            param.name.clone(),
            source(
                DomainSourceKind::SafeDependencySubstitute,
                Some(&param.name),
                None,
            ),
        ));
    }
    Ok(sources)
}

fn domain_is_closed(domain: &DomainNode) -> bool {
    match domain {
        DomainNode::Boolean => true,
        DomainNode::Literal(values) => !values.is_empty(),
        DomainNode::Nullable(inner) => domain_is_closed(inner),
        DomainNode::Union(items) => !items.is_empty() && items.iter().all(domain_is_closed),
        DomainNode::Tuple(items) => items.iter().all(domain_is_closed),
        DomainNode::Object(fields) => fields.iter().all(|field| domain_is_closed(&field.domain)),
        _ => false,
    }
}

fn render_json_literal(value: &serde_json::Value, language: &Language) -> String {
    match value {
        serde_json::Value::Null => match language {
            Language::Python => "None".into(),
            Language::TypeScript => "null".into(),
        },
        serde_json::Value::Bool(value) => match language {
            Language::Python if *value => "True".into(),
            Language::Python => "False".into(),
            Language::TypeScript if *value => "true".into(),
            Language::TypeScript => "false".into(),
        },
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|item| render_json_literal(item, language))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!(
                    "{}: {}",
                    serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into()),
                    render_json_literal(value, language)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub(crate) fn literal_from_json_value(
    value: serde_json::Value,
    language: &Language,
) -> DomainLiteral {
    DomainLiteral {
        expression: render_json_literal(&value, language),
        json_value: Some(value),
    }
}

fn domain_literals(domain: &DomainNode, language: &Language) -> Vec<DomainLiteral> {
    match domain {
        DomainNode::Boolean => [false, true]
            .into_iter()
            .map(|value| {
                let json_value = serde_json::Value::Bool(value);
                DomainLiteral {
                    expression: render_json_literal(&json_value, language),
                    json_value: Some(json_value),
                }
            })
            .collect(),
        DomainNode::Literal(values) => values.clone(),
        DomainNode::Nullable(inner) => {
            let json_value = serde_json::Value::Null;
            let mut values = vec![DomainLiteral {
                expression: render_json_literal(&json_value, language),
                json_value: Some(json_value),
            }];
            values.extend(domain_literals(inner, language));
            values
        }
        DomainNode::Union(items) => items
            .iter()
            .flat_map(|item| domain_literals(item, language))
            .collect(),
        DomainNode::Tuple(items) => {
            let mut rows = vec![Vec::new()];
            for item in items {
                let values = domain_literals(item, language);
                if values.is_empty() {
                    return Vec::new();
                }
                rows = rows
                    .into_iter()
                    .flat_map(|row| {
                        values.iter().filter_map(move |value| {
                            let json = value.json_value.clone()?;
                            let mut next = row.clone();
                            next.push(json);
                            Some(next)
                        })
                    })
                    .take(65)
                    .collect();
            }
            rows.into_iter()
                .map(|row| {
                    let json_value = serde_json::Value::Array(row);
                    DomainLiteral {
                        expression: render_json_literal(&json_value, language),
                        json_value: Some(json_value),
                    }
                })
                .collect()
        }
        DomainNode::Object(fields) => {
            let mut rows = vec![serde_json::Map::new()];
            for field in fields {
                let values = domain_literals(&field.domain, language);
                if values.is_empty() && !field.optional {
                    return Vec::new();
                }
                rows = rows
                    .into_iter()
                    .flat_map(|row| {
                        let omitted = field.optional.then(|| row.clone()).into_iter();
                        let included = values.iter().filter_map(move |value| {
                            let json = value.json_value.clone()?;
                            let mut next = row.clone();
                            next.insert(field.name.clone(), json);
                            Some(next)
                        });
                        omitted.chain(included)
                    })
                    .take(65)
                    .collect();
            }
            rows.into_iter()
                .map(|row| {
                    let json_value = serde_json::Value::Object(row);
                    DomainLiteral {
                        expression: render_json_literal(&json_value, language),
                        json_value: Some(json_value),
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn representative_domain_literal(
    domain: &DomainNode,
    language: &Language,
) -> Option<DomainLiteral> {
    if let Some(value) = domain_literals(domain, language).into_iter().next() {
        return Some(value);
    }
    let json = match domain {
        DomainNode::Any | DomainNode::Nullable(_) => serde_json::Value::Null,
        DomainNode::Boolean => serde_json::Value::Bool(false),
        DomainNode::Integer => serde_json::json!(0),
        DomainNode::Float => serde_json::json!(0.0),
        DomainNode::String | DomainNode::Bytes => serde_json::Value::String(String::new()),
        DomainNode::Array(_) | DomainNode::Tuple(_) | DomainNode::Set(_) => serde_json::json!([]),
        DomainNode::Map(_, _) | DomainNode::Object(_) => serde_json::json!({}),
        DomainNode::Union(items) => {
            return items
                .iter()
                .find_map(|item| representative_domain_literal(item, language));
        }
        DomainNode::Literal(_) | DomainNode::Opaque(_) => return None,
    };
    Some(DomainLiteral {
        expression: render_json_literal(&json, language),
        json_value: Some(json),
    })
}

fn value_matches_domain(value: &DomainLiteral, domain: &DomainNode) -> Option<bool> {
    let Some(json) = &value.json_value else {
        return None;
    };
    Some(match domain {
        DomainNode::Any | DomainNode::Opaque(_) => return None,
        DomainNode::Boolean => json.is_boolean(),
        DomainNode::Integer => json.as_i64().is_some(),

        DomainNode::Float => json.is_number(),
        DomainNode::String => json.is_string(),
        DomainNode::Bytes => false,
        DomainNode::Literal(values) => values.iter().any(|candidate| same_value(candidate, value)),
        DomainNode::Nullable(inner) => {
            json.is_null() || value_matches_domain(value, inner).unwrap_or(false)
        }
        DomainNode::Union(items) => items
            .iter()
            .any(|item| value_matches_domain(value, item).unwrap_or(false)),
        DomainNode::Array(_) | DomainNode::Tuple(_) | DomainNode::Set(_) => json.is_array(),
        DomainNode::Map(_, _) | DomainNode::Object(_) => json.is_object(),
    })
}

fn normalized_expression(expression: &str) -> String {
    let mut normalized = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in expression.trim().chars() {
        if let Some(active_quote) = quote {
            normalized.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
        } else if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            normalized.push(character);
        } else if !character.is_whitespace() {
            normalized.push(character);
        }
    }
    normalized
}

fn same_value(a: &DomainLiteral, b: &DomainLiteral) -> bool {
    a.json_value.is_some() && a.json_value == b.json_value
        || normalized_expression(&a.expression) == normalized_expression(&b.expression)
}

/// Bind tagged slots into the canonical flat invocation representation.
/// Variadic slot boundaries exist only during generation/normalization.
pub(crate) fn bind_argument_slots(
    signature: &[ParameterDomain],
    slots: PlannedArgumentSlots,
) -> Result<PlannedArguments, BindingError> {
    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    let mut slot_index = 0usize;
    for parameter in signature {
        let slot = slots.slots.get(slot_index);
        match parameter.variadic {
            Some(VariadicKind::Positional) => {
                let Some(PlannedArgumentSlot::PositionalVariadic(values)) = slot else {
                    return Err(BindingError::InvalidSlot {
                        parameter: parameter.parameter.clone(),
                        message: "expected positional variadic slot".into(),
                    });
                };
                positional.extend(values.iter().cloned());
                slot_index += 1;
            }
            Some(VariadicKind::Keyword) => {
                let Some(PlannedArgumentSlot::KeywordVariadic(values)) = slot else {
                    return Err(BindingError::InvalidSlot {
                        parameter: parameter.parameter.clone(),
                        message: "expected keyword variadic slot".into(),
                    });
                };
                named.extend(
                    values
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone())),
                );
                slot_index += 1;
            }
            None => {
                let Some(slot) = slot else {
                    if parameter.optional {
                        continue;
                    }
                    return Err(BindingError::MissingSlot {
                        parameter: parameter.parameter.clone(),
                    });
                };
                let PlannedArgumentSlot::Single(value) = slot else {
                    return Err(BindingError::InvalidSlot {
                        parameter: parameter.parameter.clone(),
                        message: "expected single argument slot".into(),
                    });
                };
                if parameter.keyword_only {
                    named.insert(parameter.parameter.clone(), value.clone());
                } else {
                    positional.push(value.clone());
                }
                slot_index += 1;
            }
        }
    }
    if slot_index != slots.slots.len() {
        return Err(BindingError::TooManySlots {
            expected: slot_index,
            actual: slots.slots.len(),
        });
    }
    Ok(PlannedArguments { positional, named })
}

pub fn classify_input(
    arguments: &PlannedArguments,
    domains: &[ParameterDomain],
) -> InputClassification {
    if arguments.positional.is_empty() && arguments.named.is_empty() {
        return InputClassification::Unknown;
    }
    let mut saw_unknown = false;
    let mut positional_index = 0usize;
    let mut saw_positional_variadic = false;
    for parameter in domains {
        match parameter.variadic {
            Some(VariadicKind::Positional) => {
                saw_positional_variadic = true;
                for value in &arguments.positional[positional_index..] {
                    match value_matches_domain(value, &parameter.domain) {
                        Some(true) => {}
                        Some(false) => return InputClassification::Invalid,
                        None => saw_unknown = true,
                    }
                }
                positional_index = arguments.positional.len();
            }
            Some(VariadicKind::Keyword) => {
                let known_names = domains
                    .iter()
                    .filter(|candidate| candidate.variadic.is_none() && candidate.keyword_only)
                    .map(|candidate| candidate.parameter.as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                for (_, value) in arguments
                    .named
                    .iter()
                    .filter(|(name, _)| !known_names.contains(name.as_str()))
                {
                    match value_matches_domain(value, &parameter.domain) {
                        Some(true) => {}
                        Some(false) => return InputClassification::Invalid,
                        None => saw_unknown = true,
                    }
                }
            }
            None => {
                let value = if parameter.keyword_only {
                    arguments.named.get(&parameter.parameter)
                } else {
                    let value = arguments.positional.get(positional_index);
                    if value.is_some() {
                        positional_index += 1;
                    }
                    value.or_else(|| arguments.named.get(&parameter.parameter))
                };
                let Some(value) = value else {
                    if !parameter.optional {
                        saw_unknown = true;
                    }
                    continue;
                };
                match value_matches_domain(value, &parameter.domain) {
                    Some(true) => {}
                    Some(false) => return InputClassification::Invalid,
                    None => saw_unknown = true,
                }
            }
        }
    }
    if positional_index < arguments.positional.len() && !saw_positional_variadic {
        return InputClassification::Invalid;
    }
    if saw_unknown {
        InputClassification::Unknown
    } else {
        InputClassification::Valid
    }
}

fn same_arguments(left: &PlannedArguments, right: &PlannedArguments) -> bool {
    left == right
}

fn add_planned_input(inputs: &mut Vec<PlannedInput>, mut candidate: PlannedInput) {
    if let Some(existing) = inputs.iter_mut().find(|input| {
        input.surface_id == candidate.surface_id
            && same_arguments(&input.arguments, &candidate.arguments)
    }) {
        for candidate_source in candidate.sources.drain(..) {
            if !existing.sources.contains(&candidate_source) {
                existing.sources.push(candidate_source);
            }
        }
        if candidate.classification == InputClassification::Valid {
            existing.classification = InputClassification::Valid;
        }
        return;
    }
    inputs.push(candidate);
}

pub fn build_verification_plan(
    functions: &[FunctionInfo],
    classes: &[ClassInfo],
    aliases: &[TypeAliasInfo],
    language: &Language,
    caller_examples: &[CallerExample],
    fixture_examples: &[FixtureExample],
    inferred_properties: &[InferredProperty],
) -> VerificationPlan {
    let source_file = "<source>".to_string();
    let surfaces = functions
        .iter()
        .filter(|func| !func.is_nested)
        .map(|func| SurfaceSpec {
            id: format!("{}:{}", func.name, func.line),
            symbol: func.name.clone(),
            source_file: source_file.clone(),
            line: func.line,
            exported: func.is_exported,
            invocable: !func.is_method || func.invocation_target.is_some(),
            parameter_names: func
                .params
                .iter()
                .filter(|param| param.variadic.is_none())
                .map(|param| param.name.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut parameter_domains = Vec::new();
    for func in functions.iter().filter(|func| !func.is_nested) {
        let surface_id = format!("{}:{}", func.name, func.line);
        for (index, param) in func.params.iter().enumerate() {
            let mut sources = vec![source(
                DomainSourceKind::TypeAnnotation,
                param.type_annotation.as_deref(),
                None,
            )];
            if param.default_value.is_some() {
                sources.push(source(
                    DomainSourceKind::DefaultValue,
                    Some(&param.name),
                    None,
                ));
            }
            for seed in func
                .predicate_seeds
                .iter()
                .filter(|seed| seed.parameter == param.name)
            {
                sources.push(source(
                    DomainSourceKind::ValidationGuard,
                    Some(&param.name),
                    Some(seed.line),
                ));
            }
            let mut domain =
                domain_for_annotation(param.type_annotation.as_deref(), aliases, classes, language);
            if matches!(param.variadic, Some(VariadicKind::Positional))
                && matches!(language, Language::TypeScript)
            {
                // TypeScript rest annotations describe the collected array;
                // strip exactly one outer array so T[][] yields T[] items.
                if let DomainNode::Array(inner) = domain {
                    domain = *inner;
                }
            }
            parameter_domains.push(ParameterDomain {
                surface_id: surface_id.clone(),
                parameter: param.name.clone(),
                index,
                closed: domain_is_closed(&domain),
                domain,
                sources,
                keyword_only: param.keyword_only,
                optional: param.optional,
                variadic: param.variadic,
            });
        }
    }
    let mut contracts: Vec<ContractSpec> = Vec::new();
    for property in inferred_properties {
        let authoritative = matches!(property.evidence, CallerEvidence::AuthoritativeFixture);
        let candidate = ContractSpec {
            id: property.contract_id.clone(),
            target_surface_id: property.target_surface_id.clone(),
            oracle_kind: if authoritative {
                OracleKind::DeclaredProperty
            } else {
                OracleKind::InferredSemantic
            },
            provenance: if authoritative {
                OracleProvenance::JsonFixture
            } else {
                OracleProvenance::NameHeuristic
            },
            confidence: if authoritative {
                FindingConfidence::Authoritative
            } else {
                FindingConfidence::Low
            },
            source_file: property.source_file.clone(),
            line: property.line,
        };
        if let Some(index) = contracts.iter().position(|contract| {
            contract.id == candidate.id && contract.target_surface_id == candidate.target_surface_id
        }) {
            if authoritative && contracts[index].confidence != FindingConfidence::Authoritative {
                contracts[index] = candidate;
            }
        } else {
            contracts.push(candidate);
        }
    }
    let mut inputs = Vec::new();
    let mut by_surface = HashMap::<&str, Vec<ParameterDomain>>::new();
    for domain in &parameter_domains {
        by_surface
            .entry(domain.surface_id.as_str())
            .or_default()
            .push(domain.clone());
    }
    for surface in &surfaces {
        let domains = by_surface
            .get(surface.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let choices = domains
            .iter()
            .map(|parameter| {
                let values = domain_literals(&parameter.domain, language);
                match parameter.variadic {
                    Some(VariadicKind::Positional) => {
                        let mut slots = vec![PlannedArgumentSlot::PositionalVariadic(vec![])];
                        for value in values.iter().take(8) {
                            slots
                                .push(PlannedArgumentSlot::PositionalVariadic(vec![value.clone()]));
                        }
                        // Keep two scalar rest arguments as separate flat
                        // slots.  This is distinct from one array-valued
                        // argument, whose domain literal is itself an array.
                        for left in values.iter().take(4) {
                            for right in values.iter().take(4) {
                                slots.push(PlannedArgumentSlot::PositionalVariadic(vec![
                                    left.clone(),
                                    right.clone(),
                                ]));
                            }
                        }
                        slots
                    }
                    Some(VariadicKind::Keyword) => {
                        let mut slots = vec![PlannedArgumentSlot::KeywordVariadic(BTreeMap::new())];
                        if let Some(value) = values.first() {
                            let mut one = BTreeMap::new();
                            one.insert("kw0".to_string(), value.clone());
                            slots.push(PlannedArgumentSlot::KeywordVariadic(one));
                        }
                        if values.len() >= 2 {
                            let mut two = BTreeMap::new();
                            two.insert("kw0".to_string(), values[0].clone());
                            two.insert("kw1".to_string(), values[1].clone());
                            slots.push(PlannedArgumentSlot::KeywordVariadic(two));
                        }
                        slots
                    }
                    None => values
                        .into_iter()
                        .map(PlannedArgumentSlot::Single)
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        if !choices.is_empty()
            && choices.iter().all(|values| !values.is_empty())
            && choices.iter().map(Vec::len).product::<usize>() <= 128
        {
            let mut rows = vec![PlannedArgumentSlots { slots: Vec::new() }];
            for values in choices {
                rows = rows
                    .into_iter()
                    .flat_map(|row| {
                        values.iter().cloned().map(move |value| {
                            let mut next = row.clone();
                            next.slots.push(value);
                            next
                        })
                    })
                    .collect();
            }
            for slots in rows {
                let Ok(mut arguments) = bind_argument_slots(domains, slots) else {
                    continue;
                };
                let mut sources = vec![source(DomainSourceKind::TypeAnnotation, None, None)];
                if let Some(function) = functions.iter().find(|function| {
                    !function.is_nested
                        && format!("{}:{}", function.name, function.line) == surface.id
                }) {
                    let dependency_sources = match normalize_dependency_arguments(
                        &function.params,
                        &mut arguments,
                        language,
                        classes,
                        aliases,
                    ) {
                        Ok(sources) => sources,
                        Err(_) => continue,
                    };
                    sources.extend(dependency_sources.into_iter().map(|(_, source)| source));
                }
                let classification = classify_input(&arguments, domains);
                add_planned_input(
                    &mut inputs,
                    PlannedInput {
                        surface_id: surface.id.clone(),
                        classification,
                        arguments,
                        sources,
                    },
                );
            }
        }
    }
    for function in functions.iter().filter(|function| !function.is_nested) {
        if function.predicate_seeds.is_empty() {
            continue;
        }
        let surface_id = format!("{}:{}", function.name, function.line);
        let domains = by_surface
            .get(surface_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for seed in &function.predicate_seeds {
            let mut slots = Vec::with_capacity(domains.len());
            let mut complete = true;
            for parameter in domains {
                let value = if parameter.parameter == seed.parameter {
                    DomainLiteral {
                        expression: render_json_literal(&seed.value, language),
                        json_value: Some(seed.value.clone()),
                    }
                } else {
                    let Some(value) = representative_domain_literal(&parameter.domain, language)
                    else {
                        complete = false;
                        break;
                    };
                    value
                };
                slots.push(match parameter.variadic {
                    Some(VariadicKind::Positional) => {
                        PlannedArgumentSlot::PositionalVariadic(vec![value])
                    }
                    Some(VariadicKind::Keyword) => {
                        let mut values = BTreeMap::new();
                        values.insert("kw0".to_string(), value);
                        PlannedArgumentSlot::KeywordVariadic(values)
                    }
                    None => PlannedArgumentSlot::Single(value),
                });
            }
            if !complete {
                continue;
            }
            let Ok(mut arguments) = bind_argument_slots(domains, PlannedArgumentSlots { slots })
            else {
                continue;
            };
            let dependency_sources = match normalize_dependency_arguments(
                &function.params,
                &mut arguments,
                language,
                classes,
                aliases,
            ) {
                Ok(sources) => sources,
                Err(_) => continue,
            };
            let mut sources = vec![source(
                DomainSourceKind::ValidationGuard,
                Some(&seed.parameter),
                Some(seed.line),
            )];
            sources.extend(dependency_sources.into_iter().map(|(_, source)| source));
            add_planned_input(
                &mut inputs,
                PlannedInput {
                    surface_id: surface_id.clone(),
                    classification: classify_input(&arguments, domains),
                    arguments,
                    sources,
                },
            );
        }
    }
    for caller in caller_examples {
        let Some(function) = functions.iter().find(|function| {
            !function.is_nested
                && format!("{}:{}", function.name, function.line) == caller.target_surface_id
        }) else {
            continue;
        };
        let mut arguments = caller.arguments.clone();
        let dependency_sources = match normalize_dependency_arguments(
            &function.params,
            &mut arguments,
            language,
            classes,
            aliases,
        ) {
            Ok(sources) => sources,
            Err(_) => continue,
        };
        let classification = if matches!(caller.evidence, CallerEvidence::StaticSyntax) {
            InputClassification::Unknown
        } else {
            let domains = parameter_domains
                .iter()
                .filter(|domain| domain.surface_id == caller.target_surface_id)
                .cloned()
                .collect::<Vec<_>>();
            classify_input(&arguments, &domains)
        };
        let mut observed_source = source(
            DomainSourceKind::ObservedCall,
            Some(&caller.caller),
            Some(caller.line),
        );
        observed_source.source_file = Some(caller.source_file.clone());
        let mut sources = vec![observed_source];
        sources.extend(dependency_sources.into_iter().map(|(_, source)| source));
        add_planned_input(
            &mut inputs,
            PlannedInput {
                surface_id: caller.target_surface_id.clone(),
                arguments,
                classification,
                sources,
            },
        );
    }
    for fixture in fixture_examples {
        let Some(function) = functions.iter().find(|function| {
            !function.is_nested
                && format!("{}:{}", function.name, function.line) == fixture.target_surface_id
        }) else {
            continue;
        };
        let mut arguments = fixture.arguments.clone();
        let dependency_sources = match normalize_dependency_arguments(
            &function.params,
            &mut arguments,
            language,
            classes,
            aliases,
        ) {
            Ok(sources) => sources,
            Err(_) => continue,
        };
        let mut fixture_source = source(DomainSourceKind::JsonFixture, None, Some(fixture.line));
        fixture_source.source_file = Some(fixture.source_file.clone());
        let mut sources = vec![fixture_source];
        sources.extend(dependency_sources.into_iter().map(|(_, source)| source));
        let domains = parameter_domains
            .iter()
            .filter(|domain| domain.surface_id == fixture.target_surface_id)
            .cloned()
            .collect::<Vec<_>>();
        add_planned_input(
            &mut inputs,
            PlannedInput {
                surface_id: fixture.target_surface_id.clone(),
                classification: classify_input(&arguments, &domains),
                arguments,
                sources,
            },
        );
    }
    let mut execution_units = Vec::new();
    for surface in surfaces.iter().filter(|surface| surface.invocable) {
        let mut invocation = InvocationPath::Direct;
        let mut target = InvocationTarget::Direct;
        if let Some(caller) = caller_examples.iter().find(|caller| {
            caller.target_surface_id == surface.id
                && matches!(
                    caller.evidence,
                    CallerEvidence::RuntimeConfirmed | CallerEvidence::AuthoritativeFixture
                )
        }) {
            invocation = InvocationPath::Caller {
                source_file: caller.source_file.clone(),
                symbol: caller.caller.clone(),
                line: caller.line,
            };
            target = InvocationTarget::ExportedCaller {
                caller_surface_id: caller.caller.clone(),
                source_file: caller.source_file.clone(),
                line: caller.line,
            };
        }
        execution_units.push(ExecutionUnit {
            surface_id: surface.id.clone(),
            invocation,
            target,
            source_file: surface.source_file.clone(),
            inputs: inputs
                .iter()
                .filter(|input| input.surface_id == surface.id)
                .cloned()
                .collect(),
            contracts: contracts
                .iter()
                .filter(|contract| contract.target_surface_id == surface.id)
                .cloned()
                .collect(),
        });
    }
    VerificationPlan {
        surfaces,
        parameter_domains,
        contracts,
        inputs,
        execution_units,
    }
}
