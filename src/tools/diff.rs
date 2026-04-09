#[derive(Debug, Clone)]
pub struct ChangedRange {
    pub start_line: usize,
    pub end_line: usize,
}

/// Parse unified diff format to extract changed line ranges (new-file side).
pub fn parse_changed_lines(diff: &str) -> Vec<ChangedRange> {
    let mut ranges = vec![];
    let mut current_line: usize = 0;
    let mut range_start: Option<usize> = None;

    for line in diff.lines() {
        if line.starts_with("@@") {
            // Flush any open range
            if let Some(start) = range_start.take() {
                ranges.push(ChangedRange {
                    start_line: start,
                    end_line: current_line.saturating_sub(1).max(start),
                });
            }
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
            if let Some(start) = range_start.take() {
                ranges.push(ChangedRange {
                    start_line: start,
                    end_line: current_line.saturating_sub(1).max(start),
                });
            }
            current_line += 1;
        }
    }

    // Flush final range
    if let Some(start) = range_start.take() {
        ranges.push(ChangedRange {
            start_line: start,
            end_line: current_line.saturating_sub(1).max(start),
        });
    }

    ranges
}
