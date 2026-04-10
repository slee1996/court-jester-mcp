#[derive(Debug, Clone)]
pub struct ChangedRange {
    pub start_line: usize,
    pub end_line: usize,
}

fn flush_range(
    ranges: &mut Vec<ChangedRange>,
    range_start: &mut Option<usize>,
    current_line: usize,
) {
    if let Some(start) = range_start.take() {
        ranges.push(ChangedRange {
            start_line: start,
            end_line: current_line.saturating_sub(1).max(start),
        });
    }
}

fn normalize_diff_path(path: &str) -> Option<String> {
    let raw = path.split('\t').next().unwrap_or(path).trim();
    if raw.is_empty() || raw == "/dev/null" {
        return None;
    }
    let stripped = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    Some(stripped.to_string())
}

fn path_matches(target_file: &str, diff_path: &str) -> bool {
    let target_parts: Vec<String> = std::path::Path::new(target_file)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    let diff_parts: Vec<String> = std::path::Path::new(diff_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();

    if diff_parts.is_empty() || diff_parts.len() > target_parts.len() {
        return false;
    }

    target_parts[target_parts.len() - diff_parts.len()..] == diff_parts[..]
}

/// Parse unified diff format to extract changed line ranges (new-file side).
pub fn parse_changed_lines(diff: &str) -> Vec<ChangedRange> {
    let mut ranges = vec![];
    let mut current_line: usize = 0;
    let mut range_start: Option<usize> = None;

    for line in diff.lines() {
        if line.starts_with("@@") {
            flush_range(&mut ranges, &mut range_start, current_line);
            // Parse @@ -old,count +new,count @@ header
            if let Some(plus_pos) = line.find('+') {
                let rest = &line[plus_pos + 1..];
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != ',')
                    .unwrap_or(rest.len());
                let nums = &rest[..end];
                let new_start: usize = nums
                    .split(',')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                current_line = new_start;
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            // Added line
            if range_start.is_none() {
                range_start = Some(current_line);
            }
            current_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            // Deleted line — doesn't advance new-file line counter
            // but is part of a change
            if range_start.is_none() {
                range_start = Some(current_line);
            }
        } else {
            // Context line
            flush_range(&mut ranges, &mut range_start, current_line);
            current_line += 1;
        }
    }

    // Flush final range
    flush_range(&mut ranges, &mut range_start, current_line);

    ranges
}

/// Parse unified diff format to extract changed line ranges for a single file.
/// Handles full repo diffs by selecting only hunks whose `+++` path matches the
/// requested file path.
pub fn parse_changed_lines_for_file(diff: &str, target_file: &str) -> Vec<ChangedRange> {
    let mut ranges = vec![];
    let mut current_line: usize = 0;
    let mut range_start: Option<usize> = None;
    let mut saw_file_header = false;
    let mut current_file_matches = true;

    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            flush_range(&mut ranges, &mut range_start, current_line);
            saw_file_header = true;
            current_file_matches = normalize_diff_path(path)
                .map(|diff_path| path_matches(target_file, &diff_path))
                .unwrap_or(false);
            continue;
        }

        if line.starts_with("--- ") {
            flush_range(&mut ranges, &mut range_start, current_line);
            continue;
        }

        if saw_file_header && !current_file_matches {
            if line.starts_with("@@") {
                range_start = None;
            }
            continue;
        }

        if line.starts_with("@@") {
            flush_range(&mut ranges, &mut range_start, current_line);
            if let Some(plus_pos) = line.find('+') {
                let rest = &line[plus_pos + 1..];
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != ',')
                    .unwrap_or(rest.len());
                let nums = &rest[..end];
                let new_start: usize = nums
                    .split(',')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                current_line = new_start;
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            if range_start.is_none() {
                range_start = Some(current_line);
            }
            current_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            if range_start.is_none() {
                range_start = Some(current_line);
            }
        } else {
            flush_range(&mut ranges, &mut range_start, current_line);
            current_line += 1;
        }
    }

    flush_range(&mut ranges, &mut range_start, current_line);
    ranges
}
