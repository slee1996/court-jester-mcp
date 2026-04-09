#[test]
fn debug_typed_param() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .unwrap();
    let code = "def f(a: int, b: int = 5, c = 3): pass";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let func = root.named_child(0).unwrap();
    let params = func.child_by_field_name("parameters").unwrap();
    let mut cursor = params.walk();
    let bytes = code.as_bytes();
    for child in params.named_children(&mut cursor) {
        eprintln!(
            "\nchild kind={}, text='{}'",
            child.kind(),
            child.utf8_text(bytes).unwrap()
        );
        // Print all children with field names
        let mut c2 = child.walk();
        if c2.goto_first_child() {
            loop {
                let n = c2.node();
                let field = c2.field_name().unwrap_or("(none)");
                eprintln!(
                    "  field={}, kind={}, text='{}'",
                    field,
                    n.kind(),
                    n.utf8_text(bytes).unwrap()
                );
                if !c2.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

#[test]
fn debug_keyword_only_star() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .unwrap();
    let code = "def foo(a: str, *, b: str = \"x\", c: int = 0):\n    pass\n";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let func = root.named_child(0).unwrap();
    let params = func.child_by_field_name("parameters").unwrap();
    let bytes = code.as_bytes();
    // Print ALL children (not just named) to see the `*`
    let mut cursor = params.walk();
    if cursor.goto_first_child() {
        loop {
            let n = cursor.node();
            eprintln!(
                "node: kind={:?} named={} text={:?}",
                n.kind(),
                n.is_named(),
                n.utf8_text(bytes).unwrap()
            );
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

#[test]
fn debug_python_class_fields() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .unwrap();
    let code =
        "@dataclass(slots=True)\nclass Foo:\n    x: int\n    y: str = \"hello\"\n    z: float\n";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let bytes = code.as_bytes();
    // Find the class body
    let class_node = root.named_child(0).unwrap(); // decorated_definition
    fn dump(node: &tree_sitter::Node, source: &[u8], indent: usize) {
        let txt = node.utf8_text(source).unwrap_or("").replace('\n', "\\n");
        let short = if txt.len() > 60 { &txt[..60] } else { &txt };
        eprintln!(
            "{:indent$}{} named={} text={:?}",
            "",
            node.kind(),
            node.is_named(),
            short,
            indent = indent
        );
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dump(&child, source, indent + 2);
        }
    }
    dump(&class_node, bytes, 0);
}

#[test]
fn debug_ts_interface_fields() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let code = "interface Foo {\n  id: number;\n  name: string;\n  items?: string[];\n}\n";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let bytes = code.as_bytes();
    let iface = root.named_child(0).unwrap();
    fn dump(node: &tree_sitter::Node, source: &[u8], indent: usize) {
        let txt = node.utf8_text(source).unwrap_or("").replace('\n', "\\n");
        let short = if txt.len() > 60 { &txt[..60] } else { &txt };
        eprintln!(
            "{:indent$}{} named={} field={:?} text={:?}",
            "",
            node.kind(),
            node.is_named(),
            "",
            short,
            indent = indent
        );
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dump(&child, source, indent + 2);
        }
    }
    dump(&iface, bytes, 0);
}

#[test]
fn debug_ts_param() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let code = "function add(a: number, b: number): number { return a + b; }";
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let func = root.named_child(0).unwrap();
    let params = func.child_by_field_name("parameters").unwrap();
    let mut cursor = params.walk();
    let bytes = code.as_bytes();
    for child in params.named_children(&mut cursor) {
        eprintln!(
            "\nchild kind={}, text='{}'",
            child.kind(),
            child.utf8_text(bytes).unwrap()
        );
        let mut c2 = child.walk();
        if c2.goto_first_child() {
            loop {
                let n = c2.node();
                let field = c2.field_name().unwrap_or("(none)");
                eprintln!(
                    "  field={}, kind={}, text='{}'",
                    field,
                    n.kind(),
                    n.utf8_text(bytes).unwrap()
                );
                if !c2.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

#[test]
fn debug_ts_type_alias() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();

    let code = "export type AuthzObject = {\n  type: string;\n  id: string;\n};\n\nexport type TupleFilter = {\n  objectType?: string;\n  objectId?: string | null;\n};";
    eprintln!("Code: {code}");
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    let bytes = code.as_bytes();
    fn dump(node: &tree_sitter::Node, source: &[u8], indent: usize) {
        let txt = node.utf8_text(source).unwrap_or("").replace('\n', "\\n");
        let short = if txt.len() > 80 { &txt[..80] } else { &txt };
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                let field = cursor.field_name().unwrap_or("");
                let ct = child.utf8_text(source).unwrap_or("").replace('\n', "\\n");
                let cs = if ct.len() > 60 { &ct[..60] } else { &ct };
                eprintln!(
                    "{:indent$}{field:<12} kind={:<28} named={} text={:?}",
                    "",
                    child.kind(),
                    child.is_named(),
                    cs,
                    indent = indent,
                    field = if field.is_empty() { "." } else { field },
                );
                if child.is_named() {
                    dump(&child, source, indent + 4);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        } else if indent == 0 {
            eprintln!("(leaf) kind={} text={:?}", node.kind(), short);
        }
    }
    dump(&root, bytes, 0);
}

#[test]
fn debug_ts_arrow() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();

    let cases = vec![
        (
            "const arrow",
            "const foo = (x: string): string => x.trim();",
        ),
        (
            "export const arrow",
            "export const bar = (a: number, b: number): number => a + b;",
        ),
        (
            "arrow block body",
            "const multi = (x: string): string => { return x.trim(); };",
        ),
        (
            "export function",
            "export function baz(x: string): string { return x; }",
        ),
    ];

    for (label, code) in cases {
        eprintln!("\n===== {} =====", label);
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        let bytes = code.as_bytes();
        fn dump(node: &tree_sitter::Node, source: &[u8], indent: usize) {
            let txt = node.utf8_text(source).unwrap_or("").replace('\n', "\\n");
            let short = if txt.len() > 80 { &txt[..80] } else { &txt };

            // Walk all children (including anonymous) to see field names
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    let field = cursor.field_name().unwrap_or("");
                    let ct = child.utf8_text(source).unwrap_or("").replace('\n', "\\n");
                    let cs = if ct.len() > 60 { &ct[..60] } else { &ct };
                    eprintln!(
                        "{:indent$}{field:<12} kind={:<28} named={} text={:?}",
                        "",
                        child.kind(),
                        child.is_named(),
                        cs,
                        indent = indent,
                        field = if field.is_empty() { "." } else { field },
                    );
                    // Recurse into named children only
                    if child.is_named() {
                        dump(&child, source, indent + 4);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            } else if indent == 0 {
                eprintln!("(leaf) kind={} text={:?}", node.kind(), short);
            }
        }
        dump(&root, bytes, 0);
    }
}
