//! Redaction and bounded diagnostic text for persisted and repair reports.

use crate::types::ExecutionResult;

const MAX_REPORT_OUTPUT_CHARS: usize = 64 * 1024;
pub(super) const MAX_REPORT_MESSAGE_CHARS: usize = 4 * 1024;

fn report_secret_value_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len()
        && !bytes[index].is_ascii_whitespace()
        && !matches!(
            bytes[index],
            b',' | b';' | b')' | b']' | b'}' | b'\'' | b'"' | b'&'
        )
    {
        index += 1;
    }
    index
}

fn report_key_tokens(key: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous_was_lowercase = false;
    for character in key.chars() {
        if !character.is_ascii_alphanumeric() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            previous_was_lowercase = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_was_lowercase && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        current.push(character.to_ascii_lowercase());
        previous_was_lowercase = character.is_ascii_lowercase();
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_sensitive_report_key(key: &str) -> bool {
    let tokens = report_key_tokens(key);
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "apikey" | "token" | "secret" | "password" | "authorization" | "cookie"
        )
    }) || tokens
        .windows(2)
        .any(|pair| pair[0] == "api" && pair[1] == "key")
}

fn report_text_secret_ranges(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-') {
            let key_start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
            {
                index += 1;
            }
            let key = &text[key_start..index];
            if is_sensitive_report_key(key) {
                let mut separator = index;
                if separator < bytes.len() && matches!(bytes[separator], b'\'' | b'"') {
                    separator += 1;
                }
                while separator < bytes.len() && bytes[separator].is_ascii_whitespace() {
                    separator += 1;
                }
                if separator < bytes.len() && matches!(bytes[separator], b'=' | b':') {
                    let mut value_start = separator + 1;
                    while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                        value_start += 1;
                    }
                    let (redact_start, redact_end) = if value_start < bytes.len()
                        && matches!(bytes[value_start], b'\'' | b'"')
                    {
                        let quote = bytes[value_start];
                        let content_start = value_start + 1;
                        let content_end = bytes[content_start..]
                            .iter()
                            .position(|byte| *byte == quote)
                            .map(|offset| content_start + offset)
                            .unwrap_or(bytes.len());
                        (content_start, content_end)
                    } else if key.eq_ignore_ascii_case("authorization")
                        && lower[value_start..].starts_with("bearer")
                    {
                        let mut credential_start = value_start + "bearer".len();
                        while credential_start < bytes.len()
                            && bytes[credential_start].is_ascii_whitespace()
                        {
                            credential_start += 1;
                        }
                        (
                            credential_start,
                            report_secret_value_end(bytes, credential_start),
                        )
                    } else {
                        (value_start, report_secret_value_end(bytes, value_start))
                    };
                    if redact_start < redact_end {
                        ranges.push((redact_start, redact_end));
                    }
                }
            }
            continue;
        }
        index += 1;
    }

    for (pattern, preserve_prefix) in [
        ("bearer ", true),
        ("ghp_", false),
        ("github_pat_", false),
        ("sk-", false),
    ] {
        let mut offset = 0;
        while let Some(relative) = lower[offset..].find(pattern) {
            let start = offset + relative;
            if start > 0
                && (bytes[start - 1].is_ascii_alphanumeric()
                    || matches!(bytes[start - 1], b'_' | b'-'))
            {
                offset = start + pattern.len();
                continue;
            }
            let value_start = if preserve_prefix {
                let mut value_start = start + pattern.len();
                while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                    value_start += 1;
                }
                value_start
            } else {
                start
            };
            let value_end = report_secret_value_end(bytes, value_start);
            if value_start < value_end {
                ranges.push((value_start, value_end));
            }
            offset = (start + pattern.len()).min(lower.len());
            if offset == lower.len() {
                break;
            }
        }
    }
    ranges.sort_unstable();
    ranges
}

pub(super) fn sanitize_report_text(text: &str) -> String {
    let mut ranges = report_text_secret_ranges(text).into_iter();
    let Some((mut range_start, mut range_end)) = ranges.next() else {
        return text.to_string();
    };
    let mut merged = Vec::new();
    for (start, end) in ranges {
        if start <= range_end {
            range_end = range_end.max(end);
        } else {
            merged.push((range_start, range_end));
            range_start = start;
            range_end = end;
        }
    }
    merged.push((range_start, range_end));

    let mut sanitized = String::with_capacity(text.len());
    let mut copied_until = 0;
    for (start, end) in merged {
        sanitized.push_str(&text[copied_until..start]);
        sanitized.push_str("[REDACTED]");
        copied_until = end;
    }
    sanitized.push_str(&text[copied_until..]);
    sanitized
}

pub(super) fn clip_report_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let retained = limit.saturating_sub(64);
    let prefix_len = retained / 2;
    let suffix_len = retained.saturating_sub(prefix_len);
    let prefix = text.chars().take(prefix_len).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!(
        "{prefix}\n...[court-jester truncated {} chars]...\n{suffix}",
        text.chars().count() - retained
    )
}

pub(super) fn sanitize_report_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = sanitize_report_text(text),
        serde_json::Value::Array(items) => {
            for item in items {
                sanitize_report_value(item);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if is_sensitive_report_key(key) && value.is_string() {
                    *value = serde_json::Value::String("[REDACTED]".into());
                    continue;
                }
                sanitize_report_value(value);
                if let Some(text) = value.as_str() {
                    let limit = match key.as_str() {
                        "stdout" | "stderr" => Some(MAX_REPORT_OUTPUT_CHARS),
                        "message" | "actual" | "expected" | "failure" => {
                            Some(MAX_REPORT_MESSAGE_CHARS)
                        }
                        _ => None,
                    };
                    if let Some(limit) = limit {
                        *value = serde_json::Value::String(clip_report_text(text, limit));
                    }
                }
            }
        }
        _ => {}
    }
}

pub(super) fn clipped_test_failure(result: &ExecutionResult) -> String {
    let diagnostic = result
        .stderr
        .lines()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|value| {
                    value
                        .get("event")
                        .and_then(|event| event.as_str())
                        .map(str::to_owned)
                })
                .as_deref()
                != Some("target_entered")
        })
        .collect::<Vec<_>>()
        .join("\n");
    sanitize_report_text(diagnostic.trim())
        .chars()
        .take(1_000)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_text_sanitizer_redacts_credentials_without_erasing_diagnostics() {
        let input = "request failed: API_KEY=alpha123 password: hunter2\nDATABASE_PASSWORD=dbopaque OPENAI_API_KEY=aiopaque ACCESS_TOKEN=accessopaque CLIENT_SECRET=clientopaque GITHUB_TOKEN=githubopaque monkey=visible\n{\"token\":\"json-token\",\"cookie\":\"session=abc\"}\nAuthorization: Bearer bearer-token\nghp_deadbeef github_pat_123 sk-live-key";
        let sanitized = sanitize_report_text(input);
        assert!(sanitized.contains("request failed:"), "{sanitized}");
        assert!(
            sanitized.contains("\"token\":\"[REDACTED]\""),
            "{sanitized}"
        );
        assert!(sanitized.contains("Bearer [REDACTED]"), "{sanitized}");
        assert!(sanitized.contains("monkey=visible"), "{sanitized}");
        for secret in [
            "alpha123",
            "hunter2",
            "json-token",
            "session=abc",
            "bearer-token",
            "ghp_deadbeef",
            "github_pat_123",
            "sk-live-key",
            "dbopaque",
            "aiopaque",
            "accessopaque",
            "clientopaque",
            "githubopaque",
        ] {
            assert!(
                !sanitized.contains(secret),
                "{secret} leaked in {sanitized}"
            );
        }
    }

    #[test]
    fn structured_sensitive_report_fields_are_redacted() {
        let mut value = serde_json::json!({
            "diagnostic": "request failed",
            "api_key": "structured-secret",
            "DATABASE_PASSWORD": "database-secret",
            "openaiApiKey": "openai-secret",
            "ACCESS_TOKEN": "access-secret",
            "CLIENT_SECRET": "client-secret",
            "GITHUB_TOKEN": "github-secret",
            "monkey": "visible",
            "nested": { "Authorization": "Bearer nested-secret" },
        });
        sanitize_report_value(&mut value);
        assert_eq!(value["diagnostic"], "request failed");
        assert_eq!(value["api_key"], "[REDACTED]");
        assert_eq!(value["nested"]["Authorization"], "[REDACTED]");
        for key in [
            "DATABASE_PASSWORD",
            "openaiApiKey",
            "ACCESS_TOKEN",
            "CLIENT_SECRET",
            "GITHUB_TOKEN",
        ] {
            assert_eq!(value[key], "[REDACTED]", "{key}");
        }
        assert_eq!(value["monkey"], "visible");
    }

    #[test]
    fn report_failure_bound_is_applied_after_secret_redaction() {
        let result = ExecutionResult {
            stdout: String::new(),
            stderr: format!("{} token=secret-beyond-bound", "x".repeat(990)),
            exit_code: Some(1),
            duration_ms: 1,
            timed_out: false,
            memory_error: false,
            termination: None,
            diagnostics: Vec::new(),
        };
        let clipped = clipped_test_failure(&result);
        assert!(!clipped.contains("secret"), "{clipped}");
        assert!(clipped.contains("[RE"), "{clipped}");
        assert!(clipped.chars().count() <= 1_000);
    }
}
