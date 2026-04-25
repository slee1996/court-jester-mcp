use court_jester_mcp::tools::analyze;
use court_jester_mcp::tools::analyze::{
    analyze, check_complexity_threshold, filter_changed_functions, source_declared_properties,
    source_directive_suppresses_complexity,
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

#[test]
fn python_complexity_directive_on_previous_comment_line_is_detected() {
    let code = "\
# court-jester-ignore complexity
def check_access(a: bool, b: bool, c: bool) -> int:
    if a:
        if b:
            if c:
                return 1
    return 0
";

    assert!(source_directive_suppresses_complexity(
        code,
        &Language::Python,
        2
    ));
}

#[test]
fn typescript_complexity_directive_in_block_comment_is_detected() {
    let code = "\
/**
 * @court-jester-ignore complexity
 */
export function route(kind: string): number {
  switch (kind) {
    case 'a':
      return 1;
    case 'b':
      return 2;
    default:
      return 0;
  }
}
";

    assert!(source_directive_suppresses_complexity(
        code,
        &Language::TypeScript,
        4
    ));
}

#[test]
fn typescript_declared_properties_are_parsed_from_source_comment() {
    let code = "\
// court-jester-properties sorted permutation
export function reorder(values: string[]): string[] {
  return [...values];
}
";
    let analysis = analyze(code, &Language::TypeScript);
    assert_eq!(
        analysis.functions[0].declared_properties,
        vec!["sorted".to_string(), "permutation".to_string()]
    );
    assert_eq!(
        source_declared_properties(code, &Language::TypeScript, 2),
        vec!["sorted".to_string(), "permutation".to_string()]
    );
}

#[test]
fn python_declared_properties_normalize_aliases() {
    let code = "\
# @court-jester-properties nonnegative antisymmetric nonempty
def check_metric(a: int, b: int) -> int:
    return a - b
";
    assert_eq!(
        source_declared_properties(code, &Language::Python, 2),
        vec![
            "nonneg".to_string(),
            "antisymmetric".to_string(),
            "nonempty_string".to_string()
        ]
    );
}

#[test]
fn typescript_exported_object_literal_methods_are_callable_surfaces() {
    let code = "\
export const reorderer = {
  reorder(values: string[]): string[] {
    return [...values].reverse();
  },
};
";
    let analysis = analyze(code, &Language::TypeScript);
    let reorder = analysis
        .functions
        .iter()
        .find(|function| function.name == "reorderer.reorder")
        .expect("exported object literal method should be analyzed");
    assert!(reorder.is_exported);
    assert!(reorder.is_method);
    assert_eq!(
        reorder.invocation_target.as_deref(),
        Some("reorderer.reorder")
    );
}

#[test]
fn typescript_exported_zero_arg_class_methods_are_callable_surfaces() {
    let code = "\
export class Reorderer {
  reorder(values: string[]): string[] {
    return [...values].reverse();
  }
}
";
    let analysis = analyze(code, &Language::TypeScript);
    let reorder = analysis
        .functions
        .iter()
        .find(|function| function.name == "Reorderer#reorder")
        .expect("exported zero-arg class method should be analyzed");
    assert!(reorder.is_exported);
    assert!(reorder.is_method);
    assert_eq!(
        reorder.invocation_target.as_deref(),
        Some("(new Reorderer()).reorder")
    );
}

#[test]
fn typescript_factory_functions_record_returned_callables() {
    let code = "\
export function createReorderer() {
  function reorder(values: string[]): string[] {
    return [...values].reverse();
  }
  return { reorder };
}
";
    let analysis = analyze(code, &Language::TypeScript);
    let factory = analysis
        .functions
        .iter()
        .find(|function| function.name == "createReorderer")
        .expect("factory should be analyzed");
    assert_eq!(factory.returned_callables, vec!["reorder".to_string()]);
}

#[test]
fn typescript_zustand_style_container_methods_are_callable_surfaces() {
    let code = "\
declare function create<T>(initializer: (set: unknown, get: unknown) => T): {
  getState(): T;
};

export const useReorderer = create(() => ({
  reorder(values: string[]): string[] {
    return [...values].reverse();
  },
}));
";
    let analysis = analyze(code, &Language::TypeScript);
    let reorder = analysis
        .functions
        .iter()
        .find(|function| function.name == "useReorderer.reorder")
        .expect("container method should be analyzed");
    assert!(reorder.is_exported);
    assert!(reorder.is_method);
    assert!(
        !reorder.is_nested,
        "surfaced container method should not be treated as nested"
    );
    assert_eq!(
        reorder.invocation_target.as_deref(),
        Some("useReorderer.getState().reorder")
    );
}

#[test]
fn typescript_curried_container_methods_are_callable_surfaces() {
    let code = "\
declare function create<T>(): (initializer: (set: unknown, get: unknown) => T) => {
  getState(): T;
};

export const useReorderer = create<{ reorder(values: string[]): string[] }>()(() => ({
  reorder(values: string[]): string[] {
    return [...values].reverse();
  },
}));
";
    let analysis = analyze(code, &Language::TypeScript);
    let reorder = analysis
        .functions
        .iter()
        .find(|function| function.name == "useReorderer.reorder")
        .expect("curried container method should be analyzed");
    assert!(reorder.is_exported);
    assert!(reorder.is_method);
    assert_eq!(
        reorder.invocation_target.as_deref(),
        Some("useReorderer.getState().reorder")
    );
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
fn typescript_export_list_and_default_export_mark_locals_exported() {
    let code = "\
function helper(): number { return 0; }
function Route(path: string): string { return path; }
const Router = (path: string): string => path.toUpperCase();
function express(): string { return \"ok\"; }
export { Route, Router };
export default express;
";
    let r = analyze(code, &Language::TypeScript);
    let exported: std::collections::HashMap<&str, bool> = r
        .functions
        .iter()
        .map(|func| (func.name.as_str(), func.is_exported))
        .collect();

    assert_eq!(exported.get("helper"), Some(&false));
    assert_eq!(exported.get("Route"), Some(&true));
    assert_eq!(exported.get("Router"), Some(&true));
    assert_eq!(exported.get("express"), Some(&true));
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

#[test]
fn typescript_enum_is_recorded_as_literal_union_alias() {
    let code = r#"
export enum DeliveryChannel {
  Email = "email",
  Sms = "sms",
}
"#;
    let r = analyze(code, &Language::TypeScript);
    let alias = r
        .aliases
        .iter()
        .find(|alias| alias.name == "DeliveryChannel")
        .expect("enum should be exposed as a type alias");
    assert_eq!(alias.type_annotation, "\"email\" | \"sms\"");
}

#[test]
fn typescript_const_tuple_type_alias_is_rewritten_to_literal_union() {
    let code = r#"
export const ALERT_LEVELS = ["info", "critical"] as const;
export type AlertLevel = typeof ALERT_LEVELS[number];
"#;
    let r = analyze(code, &Language::TypeScript);
    let alias = r
        .aliases
        .iter()
        .find(|alias| alias.name == "AlertLevel")
        .expect("const tuple type alias should be recorded");
    assert_eq!(alias.type_annotation, "\"info\" | \"critical\"");
}

#[test]
fn resolve_imported_closed_domain_aliases_from_sibling() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("types.ts"),
        r#"
export enum BillingCycle {
  Monthly = "monthly",
  Annual = "annual",
}
export const ALERT_CHANNELS = ["email", "sms"] as const;
export type AlertChannel = typeof ALERT_CHANNELS[number];
"#,
    )
    .unwrap();

    let main_path = dir.path().join("main.ts");
    std::fs::write(
        &main_path,
        r#"
import { BillingCycle } from "./types";
import type { AlertChannel } from "./types";
export function cycleDays(cycle: BillingCycle): number { return 30; }
export function channelLabel(channel: AlertChannel): string { return channel; }
"#,
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);
    let imported = analyze::resolve_imported_types(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
    );
    let aliases: std::collections::HashMap<_, _> = imported
        .aliases
        .iter()
        .map(|alias| (alias.name.as_str(), alias.type_annotation.as_str()))
        .collect();

    assert_eq!(
        aliases.get("BillingCycle"),
        Some(&"\"monthly\" | \"annual\"")
    );
    assert_eq!(aliases.get("AlertChannel"), Some(&"\"email\" | \"sms\""));
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

#[test]
fn resolve_imported_types_from_deep_typescript_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let types_dir = dir.path().join("src").join("types");
    let main_dir = dir.path().join("src").join("deep").join("nested");
    std::fs::create_dir_all(&types_dir).unwrap();
    std::fs::create_dir_all(&main_dir).unwrap();

    std::fs::write(
        types_dir.join("profile.ts"),
        "export type Profile = { id: number; timezone: string; };",
    )
    .unwrap();

    let main_path = main_dir.join("main.ts");
    std::fs::write(
        &main_path,
        "import type { Profile } from \"../../types/profile\";\nexport function tz(profile: Profile): string { return profile.timezone; }",
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);
    let imported = analyze::resolve_imported_types(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
    );

    assert_eq!(
        imported.classes.len(),
        1,
        "should resolve ../../types/profile"
    );
    assert_eq!(imported.classes[0].name, "Profile");
}

#[test]
fn resolve_imported_types_from_parent_python_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let pkg_dir = dir.path().join("pkg");
    let sub_dir = pkg_dir.join("sub");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::write(pkg_dir.join("__init__.py"), "").unwrap();
    std::fs::write(sub_dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        pkg_dir.join("models.py"),
        "class Profile:\n    timezone: str\n    locale: str\n",
    )
    .unwrap();

    let main_path = sub_dir.join("main.py");
    std::fs::write(
        &main_path,
        "from ..models import Profile\n\ndef preferred_timezone(profile: Profile) -> str:\n    return profile.timezone\n",
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::Python);
    let imported =
        analyze::resolve_imported_types(&analysis, main_path.to_str().unwrap(), &Language::Python);

    assert_eq!(imported.classes.len(), 1, "should resolve ..models");
    assert_eq!(imported.classes[0].name, "Profile");
    assert_eq!(imported.classes[0].fields.len(), 2);
}

#[test]
fn resolve_imported_types_for_names_only_loads_referenced_bindings() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("types.ts"),
        "\
export type Foo = { id: number; };
export type Bar = { name: string; };
",
    )
    .unwrap();

    let main_path = dir.path().join("main.ts");
    std::fs::write(
        &main_path,
        "\
import type { Foo, Bar } from \"./types\";
export function onlyFoo(value: Foo): number { return value.id; }
",
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );

    assert!(
        imported.classes.iter().any(|class| class.name == "Foo"),
        "referenced Foo should resolve"
    );
    assert!(
        !imported.classes.iter().any(|class| class.name == "Bar"),
        "unreferenced Bar should not resolve"
    );
    assert!(
        imported.aliases.iter().any(|alias| alias.name == "Foo"),
        "referenced Foo alias should resolve"
    );
    assert!(
        !imported.aliases.iter().any(|alias| alias.name == "Bar"),
        "unreferenced Bar alias should not resolve"
    );
}

#[test]
fn resolve_imported_types_for_names_keeps_transitive_dependencies() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("shared.ts"),
        "export type PathValue = string | number;",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("types.ts"),
        "\
import type { PathValue } from \"./shared\";
export type Profile = { key: PathValue; };
",
    )
    .unwrap();

    let main_path = dir.path().join("main.ts");
    std::fs::write(
        &main_path,
        "\
import type { Profile } from \"./types\";
export function profileKey(value: Profile): string | number { return value.key; }
",
    )
    .unwrap();

    let code = std::fs::read_to_string(&main_path).unwrap();
    let analysis = analyze(&code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );

    assert!(
        imported.classes.iter().any(|class| class.name == "Profile"),
        "referenced Profile should resolve"
    );
    assert!(
        imported
            .aliases
            .iter()
            .any(|alias| alias.name == "PathValue"),
        "transitive PathValue alias should resolve"
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
    assert_eq!(complex.complexity_breakdown.get("if"), Some(&2));
    assert_eq!(complex.complexity_breakdown.get("for"), Some(&1));
    assert!(
        complex.cognitive_complexity >= 4,
        "complex_fn should have cognitive complexity >= 4, got {}",
        complex.cognitive_complexity
    );
    assert!(
        complex.max_nesting_depth >= 2,
        "complex_fn should report nesting depth >= 2, got {}",
        complex.max_nesting_depth
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
    assert_eq!(complex.complexity_breakdown.get("if"), Some(&2));
    assert_eq!(complex.complexity_breakdown.get("for"), Some(&1));
}

#[test]
fn python_nested_function_complexity_does_not_include_child() {
    let code = "\
def outer(x: int) -> int:
    def inner(y: int) -> int:
        if y > 0:
            return y
        return 0
    return x
";
    let r = analyze(code, &Language::Python);
    let outer = r.functions.iter().find(|f| f.name == "outer").unwrap();
    let inner = r.functions.iter().find(|f| f.name == "inner").unwrap();

    assert_eq!(
        outer.complexity, 1,
        "outer should not inherit nested inner complexity"
    );
    assert_eq!(outer.cognitive_complexity, 0);
    assert!(inner.is_nested, "inner should be marked nested");
    assert!(
        inner.complexity >= 2,
        "inner should still report its own branch complexity, got {}",
        inner.complexity
    );
}

#[test]
fn python_match_case_counts_complexity() {
    let code = "\
def classify(x: int) -> str:
    match x:
        case 0:
            return \"zero\"
        case 1 | 2:
            return \"small\"
        case _:
            return \"other\"
";
    let r = analyze(code, &Language::Python);
    let classify = r.functions.iter().find(|f| f.name == "classify").unwrap();

    assert_eq!(
        classify.complexity, 4,
        "base + three case clauses should produce complexity 4"
    );
    assert_eq!(classify.complexity_breakdown.get("case"), Some(&3));
    assert!(
        classify.cognitive_complexity >= 6,
        "match/case should accumulate cognitive complexity, got {}",
        classify.cognitive_complexity
    );
    assert!(
        classify.max_nesting_depth >= 1,
        "match/case should report nesting depth, got {}",
        classify.max_nesting_depth
    );
}

#[test]
fn typescript_switch_for_of_and_logical_operators_count_complexity() {
    let code = "\
function score(items: number[] | null, fallback: number): number {
  let total = 0;
  for (const item of items ?? []) {
    switch (item) {
      case 0:
        total += fallback || 1;
        break;
      default:
        total += item && fallback ? item : fallback;
    }
  }
  return total;
}
";
    let r = analyze(code, &Language::TypeScript);
    let score = r.functions.iter().find(|f| f.name == "score").unwrap();

    assert!(
        score.complexity >= 8,
        "for-of, switch branches, logical ops, ternary, and ?? should all count; got {}",
        score.complexity
    );
    assert_eq!(score.complexity_breakdown.get("for_of"), Some(&1));
    assert_eq!(score.complexity_breakdown.get("switch_case"), Some(&1));
    assert_eq!(score.complexity_breakdown.get("switch_default"), Some(&1));
    assert_eq!(score.complexity_breakdown.get("logical_or"), Some(&1));
    assert_eq!(score.complexity_breakdown.get("logical_and"), Some(&1));
    assert_eq!(
        score.complexity_breakdown.get("nullish_coalescing"),
        Some(&1)
    );
    assert_eq!(score.complexity_breakdown.get("ternary"), Some(&1));
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
