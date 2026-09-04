use court_jester::tools::analyze;
use court_jester::tools::analyze::{
    analyze, analyze_with_context, check_complexity_threshold, filter_changed_functions,
    source_declared_properties, source_directive_suppresses_complexity,
};
use court_jester::tools::diff::parse_changed_lines;
use court_jester::tools::domain::{build_verification_plan, classify_input, domain_for_annotation};
use court_jester::types::Language;

#[test]
fn typescript_null_and_undefined_remain_distinct_in_planned_domains() {
    use court_jester::types::InputClassification;
    for (parameter, expected) in [
        ("flag: boolean | null", vec!["null", "false", "true"]),
        (
            "flag: boolean | undefined",
            vec!["false", "true", "undefined"],
        ),
        (
            "flag: boolean | null | undefined",
            vec!["null", "false", "true", "undefined"],
        ),
        ("flag?: boolean", vec!["false", "true", "undefined"]),
    ] {
        let code =
            format!("export function choose({parameter}): boolean {{ return flag === true; }}");
        let analysis = analyze(&code, &Language::TypeScript);
        let plan = build_verification_plan(
            &analysis.functions,
            &analysis.classes,
            &analysis.aliases,
            &Language::TypeScript,
            &[],
            &[],
            &[],
        );
        let values = plan
            .inputs
            .iter()
            .flat_map(|input| &input.arguments.positional)
            .map(|value| value.expression.as_str())
            .collect::<Vec<_>>();
        for value in &expected {
            assert!(
                values.contains(value),
                "{parameter}: missing {value}: {values:?}"
            );
        }
        assert!(
            values.iter().all(|value| expected.contains(value)),
            "{parameter}: unexpected nullish value: {values:?}"
        );
        assert!(
            plan.inputs
                .iter()
                .all(|input| input.classification == InputClassification::Valid),
            "{parameter}: {:?}",
            plan.inputs
        );
    }
}

#[test]
fn python_bytes_literal_prefix_does_not_capture_type_names() {
    use court_jester::types::DomainNode;
    assert!(matches!(
        domain_for_annotation(Some("bool"), &[], &[], &Language::Python),
        DomainNode::Boolean
    ));
    assert!(!matches!(
        domain_for_annotation(Some("bytes"), &[], &[], &Language::Python),
        DomainNode::Literal(_)
    ));
    assert!(!matches!(
        domain_for_annotation(Some("business_type"), &[], &[], &Language::Python),
        DomainNode::Literal(_)
    ));
    for literal in ["b'abc'", "B\"abc\"", "br'abc'", "rb'abc'"] {
        assert!(
            matches!(
                domain_for_annotation(Some(literal), &[], &[], &Language::Python),
                DomainNode::Literal(_)
            ),
            "{literal}"
        );
    }
}

#[test]
fn python_variadic_parameters_preserve_binding_and_annotations() {
    for code in [
        "def f(*values: int, mode: bool, **options: str): pass",
        "def f(*values, mode: bool, **options): pass",
    ] {
        let analysis = analyze(code, &Language::Python);
        let params = &analysis.functions[0].params;
        assert_eq!(params.len(), 3);
        assert_eq!(params[0].name, "values");
        assert!(params[0].is_positional_variadic());
        assert!(params[0].optional);
        assert_eq!(params[1].name, "mode");
        assert!(params[1].keyword_only);
        assert!(!params[1].is_variadic());
        assert_eq!(params[2].name, "options");
        assert!(params[2].is_keyword_variadic());
        if code.contains("values:") {
            assert_eq!(params[0].type_annotation.as_deref(), Some("int"));
            assert_eq!(params[2].type_annotation.as_deref(), Some("str"));
        }
    }
}

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
fn typescript_infers_parameter_types_from_literal_defaults() {
    let code = r#"
function defaults(
    text = "world",
    count = 3,
    enabled = true,
    tags = ["primary"],
    options = { prefix: "/", retries: 2 },
    unresolved = makeDefault(),
) {}
"#;
    let analysis = analyze(code, &Language::TypeScript);
    let params = &analysis.functions[0].params;

    assert_eq!(params[0].type_annotation.as_deref(), Some("string"));
    assert_eq!(params[1].type_annotation.as_deref(), Some("number"));
    assert_eq!(params[2].type_annotation.as_deref(), Some("boolean"));
    assert_eq!(params[3].type_annotation.as_deref(), Some("string[]"));
    assert_eq!(
        params[4].type_annotation.as_deref(),
        Some("{ prefix: string; retries: number }")
    );
    assert_eq!(params[5].type_annotation, None);
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
fn declared_metamorphic_properties_normalize_aliases() {
    let code = "\
// court-jester-properties involutive monotonic order-independent
export function transform(value: number): number {
  return value;
}
";
    assert_eq!(
        source_declared_properties(code, &Language::TypeScript, 2),
        vec![
            "involution".to_string(),
            "monotonic".to_string(),
            "order_invariant".to_string()
        ]
    );
}

#[test]
fn python_factory_returns_only_track_nested_callables() {
    let code = "\
def create_counter():
    def increment(value: int) -> int:
        return value + 1
    return {'increment': increment}

def identity(value: int) -> int:
    result = value
    return result
";
    let analysis = analyze(code, &Language::Python);
    let factory = analysis
        .functions
        .iter()
        .find(|function| function.name == "create_counter")
        .expect("factory");
    let identity = analysis
        .functions
        .iter()
        .find(|function| function.name == "identity")
        .expect("identity");

    assert_eq!(factory.returned_callables, vec!["increment"]);
    assert!(identity.returned_callables.is_empty());
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
fn typescript_exported_accessors_are_properties_not_callable_methods() {
    let code = "\
export class Choice {
  private selected = false;
  get isDefault(): boolean {
    return this.selected;
  }
  set isDefault(value: boolean) {
    this.selected = value;
  }
}
";
    let analysis = analyze(code, &Language::TypeScript);
    let accessors = analysis
        .functions
        .iter()
        .filter(|function| function.name == "Choice#isDefault")
        .collect::<Vec<_>>();
    assert_eq!(accessors.len(), 2);
    assert!(accessors.iter().all(|function| function.is_exported));
    assert!(
        accessors
            .iter()
            .all(|function| function.invocation_target.is_none()),
        "property accessors must not be emitted as function calls: {accessors:#?}"
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

#[test]
fn resolve_imported_types_through_workspace_package_reexport() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("apps/api");
    let package_dir = app_dir.join("node_modules/@acme/types");
    std::fs::create_dir_all(package_dir.join("src")).unwrap();
    std::fs::write(
        package_dir.join("package.json"),
        r#"{"name":"@acme/types","types":"src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        package_dir.join("src/index.ts"),
        "export * from './field-value';",
    )
    .unwrap();
    std::fs::write(
        package_dir.join("src/field-value.ts"),
        r#"
export const FieldValueTypeSchema = z.enum(["NUMBER", "STRING", "BOOLEAN"]);
export type FieldValueType = keyof typeof FieldValueTypeSchema.enum;
"#,
    )
    .unwrap();

    let main_path = app_dir.join("main.ts");
    std::fs::write(
        &main_path,
        r#"
import type { FieldValueType } from "@acme/types";
export function normalize(type: FieldValueType): string { return type; }
"#,
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

    let alias = imported
        .aliases
        .iter()
        .find(|alias| alias.name == "FieldValueType")
        .expect("workspace package alias");
    assert_eq!(
        alias.type_annotation,
        "\"NUMBER\" | \"STRING\" | \"BOOLEAN\""
    );
}

#[test]
fn resolve_imported_type_used_only_by_generic_constraint() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("types.ts"),
        "export type Entity = { id: string };",
    )
    .unwrap();
    let main_path = dir.path().join("main.ts");
    let code = r#"
import type { Entity } from "./types";
export function identity<T extends Entity>(value: T): T { return value; }
"#;
    std::fs::write(&main_path, code).unwrap();

    let analysis = analyze(code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    assert!(
        referenced.contains("Entity"),
        "generic constraint should participate in the referenced-type closure"
    );
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );
    assert!(
        imported.classes.iter().any(|class| class.name == "Entity"),
        "type used only by T extends Entity should resolve"
    );
}

#[test]
fn resolve_typescript_package_root_from_exports_types_target() {
    let dir = tempfile::tempdir().unwrap();
    let package_dir = dir.path().join("node_modules/exports-root");
    std::fs::create_dir_all(package_dir.join("dist")).unwrap();
    std::fs::write(
        package_dir.join("package.json"),
        r#"{"name":"exports-root","exports":{".":{"types":"./dist/root.d.ts","default":"./dist/root.js"}}}"#,
    )
    .unwrap();
    std::fs::write(
        package_dir.join("dist/root.d.ts"),
        "export type RootContract = { root: string };",
    )
    .unwrap();
    std::fs::write(
        package_dir.join("dist/root.js"),
        "export type WrongRootContract = { wrong: boolean };",
    )
    .unwrap();
    let main_path = dir.path().join("main.ts");
    let code = r#"
import type { RootContract } from "exports-root";
export function readRoot(value: RootContract): string { return value.root; }
"#;
    std::fs::write(&main_path, code).unwrap();

    let analysis = analyze(code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );
    assert!(
        imported
            .classes
            .iter()
            .any(|class| class.name == "RootContract"),
        "root exports.types target should resolve before exports.default"
    );
}

#[test]
fn resolve_typescript_package_subpath_from_exports_types_target() {
    let dir = tempfile::tempdir().unwrap();
    let package_dir = dir.path().join("node_modules/@acme/contracts");
    std::fs::create_dir_all(package_dir.join("dist")).unwrap();
    std::fs::write(
        package_dir.join("package.json"),
        r#"{"name":"@acme/contracts","exports":{"./models":{"types":"./dist/models.d.ts","import":"./dist/models.js"}}}"#,
    )
    .unwrap();
    std::fs::write(
        package_dir.join("dist/models.d.ts"),
        "export type ModelContract = { model: string };",
    )
    .unwrap();
    std::fs::write(
        package_dir.join("dist/models.js"),
        "export type WrongModelContract = { wrong: boolean };",
    )
    .unwrap();
    let main_path = dir.path().join("main.ts");
    let code = r#"
import type { ModelContract } from "@acme/contracts/models";
export function readModel(value: ModelContract): string { return value.model; }
"#;
    std::fs::write(&main_path, code).unwrap();

    let analysis = analyze(code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );
    assert!(
        imported
            .classes
            .iter()
            .any(|class| class.name == "ModelContract"),
        "subpath exports.types target should resolve before exports.import"
    );
}

#[test]
fn resolve_imported_types_through_export_type_wildcard_barrel() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("models.ts"),
        "export type BarrelContract = { value: string };",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("barrel.ts"),
        "export type * from './models';",
    )
    .unwrap();
    let main_path = dir.path().join("main.ts");
    let code = r#"
import type { BarrelContract } from "./barrel";
export function readBarrel(value: BarrelContract): string { return value.value; }
"#;
    std::fs::write(&main_path, code).unwrap();

    let analysis = analyze(code, &Language::TypeScript);
    let referenced = analyze::referenced_type_names_for_functions(&analysis.functions);
    let imported = analyze::resolve_imported_types_for_names(
        &analysis,
        main_path.to_str().unwrap(),
        &Language::TypeScript,
        &referenced,
    );
    assert!(
        imported
            .classes
            .iter()
            .any(|class| class.name == "BarrelContract"),
        "export type * should behave as a wildcard re-export"
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
#[test]
fn domain_ir_classifies_literal_boundary_inputs() {
    let domain = domain_for_annotation(
        Some("Literal[\"draft\", \"published\"]"),
        &[],
        &[],
        &Language::Python,
    );
    assert!(
        matches!(domain, court_jester::types::DomainNode::Literal(ref values) if values.len() == 2)
    );

    let domains = vec![court_jester::types::ParameterDomain {
        surface_id: "status:1".into(),
        parameter: "status".into(),
        index: 0,
        closed: true,
        domain: domain.clone(),
        sources: vec![],
        keyword_only: false,
        optional: false,
        variadic: None,
    }];
    let valid = court_jester::types::PlannedArguments {
        positional: vec![court_jester::types::DomainLiteral {
            expression: "\"draft\"".into(),
            json_value: Some(serde_json::json!("draft")),
        }],
        named: std::collections::BTreeMap::new(),
    };
    let invalid = court_jester::types::PlannedArguments {
        positional: vec![court_jester::types::DomainLiteral {
            expression: "\"archived\"".into(),
            json_value: Some(serde_json::json!("archived")),
        }],
        named: std::collections::BTreeMap::new(),
    };
    assert_eq!(
        classify_input(&valid, &domains),
        court_jester::types::InputClassification::Valid
    );
    assert_eq!(
        classify_input(&invalid, &domains),
        court_jester::types::InputClassification::Invalid
    );
}

#[test]
fn domain_ir_resolves_enum_alias_and_recursive_alias_without_fabricating_objects() {
    let analysis = analyze(
        "enum Channel { Email = 'email', Sms = 'sms' }\ntype Loop = Loop;",
        &Language::TypeScript,
    );
    let channel = domain_for_annotation(
        Some("Channel"),
        &analysis.aliases,
        &analysis.classes,
        &Language::TypeScript,
    );
    assert!(matches!(channel, court_jester::types::DomainNode::Union(values) if values.len() == 2));
    let loop_domain = domain_for_annotation(
        Some("Loop"),
        &analysis.aliases,
        &analysis.classes,
        &Language::TypeScript,
    );
    assert_eq!(
        loop_domain,
        court_jester::types::DomainNode::Opaque("recursive_or_depth_limit".into())
    );
}

#[test]
fn verification_plan_exhausts_closed_literal_product_deterministically() {
    let analysis = analyze(
        "export function choose(channel: 'email' | 'sms', priority: 1 | 2): string { return channel + priority; }",
        &Language::TypeScript,
    );
    let first = build_verification_plan(
        &analysis.functions,
        &analysis.classes,
        &analysis.aliases,
        &Language::TypeScript,
        &[],
        &[],
        &[],
    );
    let second = build_verification_plan(
        &analysis.functions,
        &analysis.classes,
        &analysis.aliases,
        &Language::TypeScript,
        &[],
        &[],
        &[],
    );
    assert_eq!(first, second);
    let rows: Vec<Vec<String>> = first
        .inputs
        .iter()
        .filter(|row| row.surface_id.starts_with("choose:"))
        .map(|row| {
            row.arguments
                .positional
                .iter()
                .map(|value| value.expression.clone())
                .collect()
        })
        .collect();
    assert_eq!(
        rows.len(),
        4,
        "both closed parameters must be exhaustively scheduled"
    );
    assert!(rows.iter().all(|row| row.len() == 2));
}

#[test]
fn tsx_source_context_uses_tsx_grammar() {
    let analysis = analyze_with_context(
        "export function Badge() { return <span data-kind=\"ok\">ok</span>; }",
        &court_jester::types::SourceContext {
            language: Language::TypeScript,
            mode: court_jester::types::SourceMode::Tsx,
            source_file: None,
            virtual_file_path: Some("Badge.tsx".into()),
        },
    );
    assert!(!analysis.parse_error);
    assert_eq!(analysis.source_mode, court_jester::types::SourceMode::Tsx);
    assert!(analysis
        .functions
        .iter()
        .any(|function| function.name == "Badge"));
}

#[test]
fn jsx_source_context_uses_tsx_grammar() {
    let analysis = analyze_with_context(
        "const view = () => <div className=\"x\" />;",
        &court_jester::types::SourceContext {
            language: Language::TypeScript,
            mode: court_jester::types::SourceMode::Tsx,
            source_file: None,
            virtual_file_path: Some("view.jsx".into()),
        },
    );
    assert!(!analysis.parse_error);
}

#[test]
fn malformed_tsx_reports_structured_diagnostic() {
    let analysis = analyze_with_context(
        "const view = () => <div>",
        &court_jester::types::SourceContext {
            language: Language::TypeScript,
            mode: court_jester::types::SourceMode::Tsx,
            source_file: None,
            virtual_file_path: Some("view.tsx".into()),
        },
    );
    assert!(analysis.parse_error);
    let diagnostic = analysis.parse_diagnostics.first().expect("diagnostic");
    assert!(diagnostic.start_line >= 1);
    assert!(diagnostic.start_column >= 1);
    assert!(diagnostic.message.contains("syntax node"));
}

#[test]
fn branch_predicates_produce_literal_and_neighbor_seeds() {
    let analysis = analyze(
        r#"export function guarded(value: number, mode: string): string {
  if (value === 777125) return "exact";
  if (value < 10 && ["safe", "strict"].includes(mode)) return "bounded";
  return "other";
}"#,
        &Language::TypeScript,
    );
    let function = &analysis.functions[0];
    let values = function
        .predicate_seeds
        .iter()
        .filter(|seed| seed.parameter == "value")
        .map(|seed| seed.value.clone())
        .collect::<Vec<_>>();
    assert!(values.contains(&serde_json::json!(777124)));
    assert!(values.contains(&serde_json::json!(777125)));
    assert!(values.contains(&serde_json::json!(777126)));
    assert!(values.contains(&serde_json::json!(9)));
    assert!(values.contains(&serde_json::json!(10)));
    assert!(values.contains(&serde_json::json!(11)));
    let modes = function
        .predicate_seeds
        .iter()
        .filter(|seed| seed.parameter == "mode")
        .map(|seed| seed.value.clone())
        .collect::<Vec<_>>();
    assert!(modes.contains(&serde_json::json!("safe")));
    assert!(modes.contains(&serde_json::json!("strict")));

    let plan = build_verification_plan(
        &analysis.functions,
        &analysis.classes,
        &analysis.aliases,
        &Language::TypeScript,
        &[],
        &[],
        &[],
    );
    assert!(plan.inputs.iter().any(|input| {
        input
            .arguments
            .positional
            .first()
            .and_then(|value| value.json_value.as_ref())
            == Some(&serde_json::json!(777125))
            && input
                .sources
                .iter()
                .any(|source| source.kind == court_jester::types::DomainSourceKind::ValidationGuard)
    }));
}

#[test]
fn typeof_predicates_do_not_seed_type_labels_as_runtime_values() {
    let analysis = analyze(
        r#"export function normalize(input: unknown): unknown {
  if (typeof input === "string" && input === "ready") return input;
  if (typeof input === "function") return input;
  return null;
}"#,
        &Language::TypeScript,
    );
    let values = analysis.functions[0]
        .predicate_seeds
        .iter()
        .map(|seed| seed.value.clone())
        .collect::<Vec<_>>();
    assert!(values.contains(&serde_json::json!("ready")));
    assert!(!values.contains(&serde_json::json!("string")));
    assert!(!values.contains(&serde_json::json!("function")));
}

#[test]
fn python_length_and_membership_guards_seed_boundaries() {
    let analysis = analyze(
        "def classify(value: str) -> str:\n    if len(value) <= 5:\n        return 'short'\n    if value in {'admin', 'owner'}:\n        return 'role'\n    return 'other'\n",
        &Language::Python,
    );
    let values = analysis.functions[0]
        .predicate_seeds
        .iter()
        .map(|seed| seed.value.clone())
        .collect::<Vec<_>>();
    assert!(values.contains(&serde_json::json!(4)));
    assert!(values.contains(&serde_json::json!(5)));
    assert!(values.contains(&serde_json::json!(6)));
    assert!(values.contains(&serde_json::json!("admin")));
    assert!(values.contains(&serde_json::json!("owner")));
}

#[test]
fn object_predicates_seed_complete_argument_rows() {
    let analysis = analyze(
        r#"export function routeJob(input: { kind: string; attempts: number }): string {
  if (
    input.kind === "priority"
    && input.attempts === 7
  ) {
    throw new RangeError("priority retry overflow");
  }
  return input.kind;
}"#,
        &Language::TypeScript,
    );
    let function = &analysis.functions[0];
    assert!(function.predicate_seeds.iter().any(|seed| {
        seed.parameter == "input"
            && seed.property_path == ["kind"]
            && seed.value == serde_json::json!("priority")
    }));
    assert!(function.predicate_seeds.iter().any(|seed| {
        seed.parameter == "input"
            && seed.property_path == ["attempts"]
            && seed.value == serde_json::json!(7)
    }));
    let predicate_lines = function
        .predicate_seeds
        .iter()
        .filter(|seed| {
            seed.parameter == "input"
                && (seed.property_path == ["kind"] || seed.property_path == ["attempts"])
        })
        .map(|seed| seed.line)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        predicate_lines.len(),
        1,
        "all comparisons in one multiline guard must share predicate provenance"
    );

    let plan = build_verification_plan(
        &analysis.functions,
        &analysis.classes,
        &analysis.aliases,
        &Language::TypeScript,
        &[],
        &[],
        &[],
    );
    let complete_rows = plan
        .inputs
        .iter()
        .filter(|input| {
            input.classification == court_jester::types::InputClassification::Valid
                && input
                    .arguments
                    .positional
                    .first()
                    .and_then(|value| value.json_value.as_ref())
                    == Some(&serde_json::json!({
                        "kind": "priority",
                        "attempts": 7,
                    }))
        })
        .count();
    assert_eq!(
        complete_rows, 1,
        "the multiline predicate must produce one complete guarded row"
    );
}
