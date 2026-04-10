use court_jester_mcp::tools::diff::{parse_changed_lines, parse_changed_lines_for_file};

#[test]
fn parse_simple_hunk() {
    let diff = "\
--- a/foo.py
+++ b/foo.py
@@ -1,3 +1,4 @@
 unchanged
+added line
 unchanged
 unchanged
";
    let ranges = parse_changed_lines(diff);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start_line, 2);
    assert_eq!(ranges[0].end_line, 2);
}

#[test]
fn parse_multiple_hunks() {
    let diff = "\
@@ -1,3 +1,5 @@
 context
+line a
+line b
 context
@@ -10,3 +12,4 @@
 context
+line c
 context
";
    let ranges = parse_changed_lines(diff);
    assert_eq!(ranges.len(), 2, "should have 2 ranges, got {:?}", ranges);
}

#[test]
fn empty_diff_returns_no_ranges() {
    let ranges = parse_changed_lines("");
    assert!(ranges.is_empty());
}

#[test]
fn only_deletions_still_produce_range() {
    let diff = "\
@@ -1,3 +1,2 @@
 context
-deleted line
 context
";
    let ranges = parse_changed_lines(diff);
    // Deletion at line 2 should produce a range
    assert!(!ranges.is_empty(), "deletion should produce a range");
}

#[test]
fn multi_file_diff_only_returns_ranges_for_requested_file() {
    let diff = "\
diff --git a/src/alpha.ts b/src/alpha.ts
--- a/src/alpha.ts
+++ b/src/alpha.ts
@@ -1,2 +1,2 @@
-old
+new
 unchanged
diff --git a/src/beta.ts b/src/beta.ts
--- a/src/beta.ts
+++ b/src/beta.ts
@@ -10,2 +10,3 @@
 context
+extra
 context
";
    let target = "/tmp/workspace/src/beta.ts";
    let ranges = parse_changed_lines_for_file(diff, target);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start_line, 11);
    assert_eq!(ranges[0].end_line, 11);
}
