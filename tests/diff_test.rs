use court_jester_mcp::tools::diff::parse_changed_lines;

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
