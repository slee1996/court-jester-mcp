use court_jester_mcp::tools::analyze;
use court_jester_mcp::tools::analyze::{
    analyze, check_complexity_threshold, filter_changed_functions,
};
use court_jester_mcp::tools::diff::parse_changed_lines;
use court_jester_mcp::types::Language;

#[test]
fn python_function_with_types() {
    let code = "def greet(name: str, times: int = 1) -> str:\n    return name * times";
    let r = analyze(code, &Language::Python);

    assert!(!r.parse_error);
    assert_eq!(r.functions.len(), 1);
    assert_eq!(r.functions[0].name, "greet");
    assert_eq!(r.functions[0].params.len(), 2);
    assert_eq!(
        r.functions[0].params[0].type_annotation.as_deref(),
        Some("str")
    );
    assert_eq!(r.functions[0].return_type.as_deref(), Some("str"));
    assert!(r.functions[0].is_exported);
}

#[test]
fn python_class_with_bases() {
    let code = "class Dog(Animal):\n    def bark(self):\n        pass";
    let r = analyze(code, &Language::Python);

    assert_eq!(r.classes.len(), 1);
    assert_eq!(r.classes[0].name, "Dog");
    assert_eq!(r.classes[0].bases, vec!["Animal"]);
    // bark's `self` param should be filtered out
    assert_eq!(r.functions.len(), 1);
    assert!(r.functions[0].params.is_empty());
}

#[test]
fn python_imports() {
    let code = "import os\nfrom pathlib import Path\n\ndef f(): pass";
    let r = analyze(code, &Language::Python);

    assert_eq!(r.imports.len(), 2);
    assert!(r.imports[0].statement.contains("os"));
    assert!(r.imports[1].statement.contains("Path"));
}

#[test]
fn python_complexity() {
    let code = "\
def foo(x):
    if x > 0:
        for i in range(x):
            while True:
                break
";
    let r = analyze(code, &Language::Python);
    // base(1) + if(1) + for(1) + while(1) = 4
    assert!(r.complexity >= 4, "complexity was {}", r.complexity);
}

#[test]
fn python_parse_error() {
    let code = "def foo(:\n    pass";
    let r = analyze(code, &Language::Python);
    assert!(r.parse_error);
}

#[test]
fn typescript_function() {
    let code = "function add(a: number, b: number): number { return a + b; }";
    let r = analyze(code, &Language::TypeScript);

    assert!(!r.parse_error);
    assert_eq!(r.functions.len(), 1);
    assert_eq!(r.functions[0].name, "add");
    assert_eq!(r.functions[0].params.len(), 2);
    assert_eq!(
        r.functions[0].params[0].type_annotation.as_deref(),
        Some("number")
    );
    assert_eq!(r.functions[0].return_type.as_deref(), Some("number"));
    assert!(!r.functions[0].is_exported);
}

#[test]
fn typescript_class_and_interface() {
    let code = "class Foo {}\ninterface Bar {}";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(r.classes.len(), 2);
    assert_eq!(r.classes[0].name, "Foo");
    assert_eq!(r.classes[1].name, "Bar");
}

#[test]
fn typescript_imports() {
    let code = "import { readFile } from 'fs';\nfunction f() {}";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(r.imports.len(), 1);
    assert!(r.imports[0].statement.contains("fs"));
}

// ── Arrow function detection ────────────────────────────────────────────────

#[test]
fn typescript_arrow_function_detected() {
    let code = "const greet = (name: string): string => name.toUpperCase();";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(r.functions.len(), 1, "should detect arrow function");
    assert_eq!(r.functions[0].name, "greet");
    assert_eq!(r.functions[0].params.len(), 1);
    assert_eq!(r.functions[0].params[0].name, "name");
    assert_eq!(
        r.functions[0].params[0].type_annotation.as_deref(),
        Some("string")
    );
    assert_eq!(r.functions[0].return_type.as_deref(), Some("string"));
    assert!(!r.functions[0].is_method);
    assert!(!r.functions[0].is_exported);
}

#[test]
fn typescript_export_arrow_function() {
    let code = "export const add = (a: number, b: number): number => a + b;";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(
        r.functions.len(),
        1,
        "should detect exported arrow function"
    );
    assert_eq!(r.functions[0].name, "add");
    assert_eq!(r.functions[0].params.len(), 2);
    assert!(r.functions[0].is_exported);
}

#[test]
fn typescript_arrow_block_body() {
    let code = "const process = (x: string): string => {\n  if (x.length > 10) return x.slice(0, 10);\n  return x;\n};";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(r.functions.len(), 1);
    assert_eq!(r.functions[0].name, "process");
    assert!(
        r.functions[0].complexity >= 2,
        "arrow with if should have complexity >= 2, got {}",
        r.functions[0].complexity
    );
}

#[test]
fn typescript_arrow_and_function_mixed() {
    let code = "\
const foo = (x: string): string => x.trim();
function bar(x: number): number { return x + 1; }
";
    let r = analyze(code, &Language::TypeScript);
    assert_eq!(
        r.functions.len(),
        2,
        "should detect both arrow and function declaration"
    );
    let names: Vec<&str> = r.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"bar"));
}

#[test]
fn typescript_non_arrow_const_ignored() {
    let code = "const x = 42;\nconst y = \"hello\";";
    let r = analyze(code, &Language::TypeScript);
    assert_eq!(
        r.functions.len(),
        0,
        "plain const should not create functions"
    );
}

// ── Type alias extraction ───────────────────────────────────────────────────

#[test]
fn typescript_type_alias_extracted_as_class() {
    let code = "export type Foo = {\n  id: number;\n  name: string;\n  email?: string;\n};";
    let r = analyze(code, &Language::TypeScript);

    assert_eq!(
        r.classes.len(),
        1,
        "type alias with object body should be extracted"
    );
    assert_eq!(r.classes[0].name, "Foo");
    assert_eq!(r.classes[0].fields.len(), 3);
    assert_eq!(r.classes[0].fields[0].name, "id");
    assert_eq!(
        r.classes[0].fields[0].type_annotation.as_deref(),
        Some("number")
    );
    assert!(r.classes[0].fields[2].optional, "email should be optional");
}

#[test]
fn typescript_type_alias_non_object_recorded_for_resolution() {
    let code = "export type ID = string;\nexport type Pair = [string, number];";
    let r = analyze(code, &Language::TypeScript);
    assert_eq!(
        r.classes.len(),
        0,
        "non-object type aliases should not create classes"
    );
    assert_eq!(
        r.aliases.len(),
        2,
        "non-object aliases should still be recorded"
    );
    assert_eq!(r.aliases[0].name, "ID");
    assert_eq!(r.aliases[0].type_annotation, "string");
    assert_eq!(r.aliases[1].name, "Pair");
    assert_eq!(r.aliases[1].type_annotation, "[string, number]");
}

// ── Import resolution ───────────────────────────────────────────────────────

#[test]
fn resolve_imported_types_from_sibling() {
    let dir = tempfile::tempdir().unwrap();

    // Create a types file with a type alias
    std::fs::write(
        dir.path().join("types.ts"),
        "export type Foo = { id: number; name: string; };",
    )
    .unwrap();

    // Create a main file that imports it
    let main_path = dir.path().join("main.ts");
    std::fs::write(&main_path, "import type { Foo } from \"./types\";\nfunction process(f: Foo): string { return f.name; }").unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);

    // Main file has no classes (Foo is imported, not defined here)
    assert!(analysis.classes.is_empty());

    // Resolve imports should find Foo
    let imported = analyze::resolve_imported_types(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
    );
    assert_eq!(imported.classes.len(), 1, "should find Foo in types.ts");
    assert_eq!(imported.classes[0].name, "Foo");
    assert_eq!(imported.classes[0].fields.len(), 2);
}

#[test]
fn resolve_imported_non_object_alias_from_sibling() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("types.ts"),
        "export type PathValue = string | number | Array<string | number>;",
    )
    .unwrap();

    let main_path = dir.path().join("main.ts");
    std::fs::write(
        &main_path,
        "import type { PathValue } from \"./types\";\nexport function toPath(value: PathValue): PathValue { return value; }",
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);
    assert!(
        analysis.aliases.is_empty(),
        "main file should not define aliases"
    );

    let imported = analyze::resolve_imported_types(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
    );
    assert_eq!(
        imported.aliases.len(),
        1,
        "should find PathValue in types.ts"
    );
    assert_eq!(imported.aliases[0].name, "PathValue");
    assert_eq!(
        imported.aliases[0].type_annotation,
        "string | number | Array<string | number>"
    );
}

// ── Per-function complexity (Change 2) ──────────────────────────────────────

#[test]
fn python_per_function_complexity() {
    let code = "\
def simple(x: int) -> int:
    return x + 1

def complex_fn(x: int) -> int:
    if x > 0:
        for i in range(x):
            if i > 5:
                return i
    return x
";
    let r = analyze(code, &Language::Python);
    assert_eq!(r.functions.len(), 2);

    let simple = r.functions.iter().find(|f| f.name == "simple").unwrap();
    assert_eq!(
        simple.complexity, 1,
        "simple function should have complexity 1"
    );

    let complex = r.functions.iter().find(|f| f.name == "complex_fn").unwrap();
    assert!(
        complex.complexity >= 4,
        "complex_fn should have complexity >= 4, got {}",
        complex.complexity
    );
}

#[test]
fn python_end_line() {
    let code = "\
def multi_line(x):
    if x:
        return 1
    return 0
";
    let r = analyze(code, &Language::Python);
    assert_eq!(r.functions.len(), 1);
    assert_eq!(r.functions[0].line, 1);
    assert!(
        r.functions[0].end_line >= 4,
        "end_line should be >= 4, got {}",
        r.functions[0].end_line
    );
}

#[test]
fn typescript_per_function_complexity() {
    let code = "\
function simple(x: number): number { return x + 1; }

function complex(x: number): number {
  if (x > 0) {
    for (let i = 0; i < x; i++) {
      if (i > 5) return i;
    }
  }
  return x;
}
";
    let r = analyze(code, &Language::TypeScript);
    let simple = r.functions.iter().find(|f| f.name == "simple").unwrap();
    assert_eq!(simple.complexity, 1);

    let complex = r.functions.iter().find(|f| f.name == "complex").unwrap();
    assert!(
        complex.complexity >= 4,
        "complex should have complexity >= 4, got {}",
        complex.complexity
    );
}

// ── Method detection (Change 3) ─────────────────────────────────────────────

#[test]
fn python_method_detected() {
    let code = "class Foo:\n    def bar(self, x: int) -> int:\n        return x";
    let r = analyze(code, &Language::Python);

    let bar = r.functions.iter().find(|f| f.name == "bar").unwrap();
    assert!(bar.is_method, "bar should be detected as a method");
    // self should be filtered from params
    assert!(
        bar.params.iter().all(|p| p.name != "self"),
        "self should be filtered"
    );
}

#[test]
fn python_free_function_not_method() {
    let code = "def standalone(x: int) -> int:\n    return x";
    let r = analyze(code, &Language::Python);

    assert!(
        !r.functions[0].is_method,
        "standalone function should not be a method"
    );
}

#[test]
fn typescript_method_detected() {
    let code = "class Foo {\n  bar(x: number): number { return x; }\n}";
    let r = analyze(code, &Language::TypeScript);

    let bar = r.functions.iter().find(|f| f.name == "bar").unwrap();
    assert!(bar.is_method, "bar should be detected as a TS method");
}

// ── Complexity threshold (Change 7) ─────────────────────────────────────────

#[test]
fn complexity_threshold_flags_violations() {
    let code = "\
def simple(x: int) -> int:
    return x

def complex_fn(x: int) -> int:
    if x > 0:
        for i in range(x):
            if i > 5:
                return i
    return x
";
    let r = analyze(code, &Language::Python);
    let violations = check_complexity_threshold(&r, 3);
    assert!(!violations.is_empty(), "should flag complex_fn");
    assert!(violations.iter().any(|v| v.function == "complex_fn"));
}

#[test]
fn complexity_threshold_passes_when_under() {
    let code = "def simple(x: int) -> int:\n    return x";
    let r = analyze(code, &Language::Python);
    let violations = check_complexity_threshold(&r, 100);
    assert!(violations.is_empty(), "nothing should exceed threshold 100");
}

// ── Diff-aware filtering (Change 4) ─────────────────────────────────────────

#[test]
fn filter_changed_functions_overlap() {
    let code = "\
def early(x: int) -> int:
    return x

def late(x: int) -> int:
    if x > 0:
        return x
    return 0
";
    let r = analyze(code, &Language::Python);
    assert_eq!(r.functions.len(), 2);

    // Simulate a diff that only touches lines 4-7 (the late function)
    let diff = "@@ -4,4 +4,5 @@\n+def late(x: int) -> int:\n+    if x > 0:\n+        return x\n+    return 0\n";
    let ranges = parse_changed_lines(diff);
    let filtered = filter_changed_functions(&r, &ranges);
    assert_eq!(filtered.len(), 1, "only late should overlap diff");
    assert_eq!(filtered[0].name, "late");
}

#[test]
fn filter_changed_functions_no_overlap() {
    let code = "def foo(x: int) -> int:\n    return x\n\ndef bar(y: int) -> int:\n    return y";
    let r = analyze(code, &Language::Python);

    // Diff touching line 100 — neither function overlaps
    let diff = "@@ -100,1 +100,1 @@\n+some change\n";
    let ranges = parse_changed_lines(diff);
    let filtered = filter_changed_functions(&r, &ranges);
    assert!(filtered.is_empty(), "no functions should overlap");
}
