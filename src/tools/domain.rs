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

fn same_arguments(a: &PlannedArguments, b: &PlannedArguments) -> bool {
    a.positional.len() == b.positional.len()
        && a.positional
            .iter()
            .zip(&b.positional)
            .all(|(left, right)| same_value(left, right))
        && a.named.len() == b.named.len()
        && a.named.iter().all(|(name, left)| {
            b.named
                .get(name)
                .is_some_and(|right| same_value(left, right))
        })
}

pub fn classify_input(
    arguments: &PlannedArguments,
    domains: &[ParameterDomain],
) -> InputClassification {
    if arguments.positional.is_empty() && arguments.named.is_empty() {
        return InputClassification::Unknown;
    }
    let mut saw_unknown = false;
    for (index, domain) in domains.iter().enumerate() {
        let value = arguments
            .positional
            .get(index)
            .or_else(|| arguments.named.get(&domain.parameter));
        let Some(value) = value else {
            saw_unknown = true;
            continue;
        };
        match value_matches_domain(value, &domain.domain) {
            Some(true) => {}
            Some(false) => return InputClassification::Invalid,
            None => saw_unknown = true,
        }
    }
    if saw_unknown {
        InputClassification::Unknown
    } else {
        InputClassification::Valid
    }
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
                .filter(|param| !param.name.starts_with('*'))
                .map(|param| param.name.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut parameter_domains = Vec::new();
    for func in functions.iter().filter(|func| !func.is_nested) {
        let surface_id = format!("{}:{}", func.name, func.line);
        for (index, param) in func
            .params
            .iter()
            .filter(|param| !param.name.starts_with('*'))
            .enumerate()
        {
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
            let domain =
                domain_for_annotation(param.type_annotation.as_deref(), aliases, classes, language);
            parameter_domains.push(ParameterDomain {
                surface_id: surface_id.clone(),
                parameter: param.name.clone(),
                index,
                closed: domain_is_closed(&domain),
                domain,
                sources,
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
            .map(|domain| domain_literals(&domain.domain, language))
            .collect::<Vec<_>>();
        if !choices.is_empty()
            && choices.iter().all(|values| !values.is_empty())
            && choices.iter().map(Vec::len).product::<usize>() <= 64
        {
            let mut rows = vec![Vec::new()];
            for values in choices {
                rows = rows
                    .into_iter()
                    .flat_map(|row| {
                        values.iter().cloned().map(move |value| {
                            let mut next = row.clone();
                            next.push(value);
                            next
                        })
                    })
                    .collect();
            }
            for positional in rows {
                add_planned_input(
                    &mut inputs,
                    PlannedInput {
                        surface_id: surface.id.clone(),
                        arguments: PlannedArguments {
                            positional,
                            named: BTreeMap::new(),
                        },
                        classification: InputClassification::Valid,
                        sources: vec![source(DomainSourceKind::TypeAnnotation, None, None)],
                    },
                );
            }
        }
    }
    for caller in caller_examples {
        let classification = if matches!(caller.evidence, CallerEvidence::StaticSyntax) {
            InputClassification::Unknown
        } else {
            InputClassification::Valid
        };
        let mut observed_source = source(
            DomainSourceKind::ObservedCall,
            Some(&caller.caller),
            Some(caller.line),
        );
        observed_source.source_file = Some(caller.source_file.clone());
        add_planned_input(
            &mut inputs,
            PlannedInput {
                surface_id: caller.target_surface_id.clone(),
                arguments: caller.arguments.clone(),
                classification,
                sources: vec![observed_source],
            },
        );
    }
    for fixture in fixture_examples {
        let mut fixture_source = source(DomainSourceKind::JsonFixture, None, Some(fixture.line));
        fixture_source.source_file = Some(fixture.source_file.clone());
        add_planned_input(
            &mut inputs,
            PlannedInput {
                surface_id: fixture.target_surface_id.clone(),
                arguments: fixture.arguments.clone(),
                classification: InputClassification::Valid,
                sources: vec![fixture_source],
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
