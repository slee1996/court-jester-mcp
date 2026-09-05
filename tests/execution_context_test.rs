use std::path::Path;

use court_jester::types::{ContextRequest, Language, SourceMode};
use court_jester::{resolve_execution_context, resolve_verification_context};

#[test]
fn project_only_context_resolves_from_project_not_invocation_directory() {
    let invocation = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".venv")).unwrap();
    let context = resolve_execution_context(ContextRequest {
        invocation_dir: invocation.path(),
        explicit_project_dir: Some(project.path()),
        target_file: None,
        test_file: None,
        language: Language::Python,
        virtual_file_path: None,
    })
    .unwrap();
    let root = project.path().canonicalize().unwrap();
    assert_eq!(context.target_package_root, root);
    assert_eq!(context.dependency_roots, vec![root]);
    assert_eq!(
        context.invocation_dir,
        invocation.path().canonicalize().unwrap()
    );
}

#[test]
fn monorepo_execution_context_preserves_package_and_dependency_roots() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();
    let package = workspace.join("packages/app");
    let source = package.join("src/index.ts");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(workspace.join("node_modules")).unwrap();
    std::fs::write(workspace.join("package.json"), "{}\n").unwrap();
    std::fs::write(package.join("package.json"), "{}\n").unwrap();
    std::fs::write(&source, "export const answer = 42;\n").unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: None,
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();

    assert_eq!(context.target_source.mode, SourceMode::TypeScript);
    assert!(context.target_package_root.ends_with("packages/app"));
    assert!(context
        .dependency_roots
        .iter()
        .any(|root| root == &std::fs::canonicalize(workspace).unwrap()));
}

#[test]
fn explicit_package_dir_uses_declared_pnpm_workspace_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();
    let package = workspace.join("packages/app");
    let source = package.join("src/index.ts");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(workspace.join("node_modules")).unwrap();
    std::fs::write(
        workspace.join("pnpm-workspace.yaml"),
        "packages:\n  - packages/*\n",
    )
    .unwrap();
    std::fs::write(workspace.join("package.json"), "{}\n").unwrap();
    std::fs::write(package.join("package.json"), "{}\n").unwrap();
    std::fs::write(&source, "export const answer = 42;\n").unwrap();

    let context = resolve_execution_context(ContextRequest {
        invocation_dir: workspace,
        explicit_project_dir: Some(&package),
        target_file: Some(&source),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();

    assert_eq!(
        context.workspace_root,
        std::fs::canonicalize(workspace).unwrap()
    );
    assert_eq!(
        context.target_package_root,
        std::fs::canonicalize(package).unwrap()
    );
    assert_eq!(
        context.dependency_roots,
        vec![std::fs::canonicalize(workspace).unwrap()]
    );
}

#[test]
fn explicit_project_dir_rejects_external_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let outside = temp.path().join("outside.ts");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(&outside, "export const value = 1;\n").unwrap();

    let error = resolve_execution_context(ContextRequest {
        invocation_dir: temp.path(),
        explicit_project_dir: Some(&project),
        target_file: Some(&outside),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap_err();
    assert!(error.to_string().contains("outside project"));
}

#[test]
fn differential_context_keeps_independent_candidate_and_base_roots() {
    let temp = tempfile::tempdir().unwrap();
    let candidate_root = temp.path().join("candidate");
    let base_root = temp.path().join("base");
    let candidate = candidate_root.join("src/app.ts");
    let base = base_root.join("src/app.tsx");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::create_dir_all(base.parent().unwrap()).unwrap();
    std::fs::write(&candidate, "export const candidate = 1;\n").unwrap();
    std::fs::write(&base, "export const Base = () => <div />;\n").unwrap();

    let contexts = resolve_verification_context(
        ContextRequest {
            invocation_dir: temp.path(),
            explicit_project_dir: Some(&candidate_root),
            target_file: Some(&candidate),
            test_file: None,
            language: Language::TypeScript,
            virtual_file_path: None,
        },
        Some(ContextRequest {
            invocation_dir: temp.path(),
            explicit_project_dir: Some(&base_root),
            target_file: Some(&base),
            test_file: None,
            language: Language::TypeScript,
            virtual_file_path: None,
        }),
    )
    .unwrap();

    assert_ne!(
        contexts.candidate.workspace_root,
        contexts.base.as_ref().unwrap().workspace_root
    );
    assert_eq!(
        contexts.candidate.target_source.mode,
        SourceMode::TypeScript
    );
    assert_eq!(contexts.base.unwrap().target_source.mode, SourceMode::Tsx);
}

#[test]
fn relative_project_dir_is_resolved_from_invocation_directory() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let source = project.join("main.ts");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(&source, "export const value = 1;\n").unwrap();
    let context = resolve_execution_context(ContextRequest {
        invocation_dir: temp.path(),
        explicit_project_dir: Some(Path::new("project")),
        target_file: Some(Path::new("project/main.ts")),
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: None,
    })
    .unwrap();
    assert_eq!(
        context.workspace_root,
        std::fs::canonicalize(project).unwrap()
    );
}

#[test]
fn ambiguous_virtual_typescript_path_uses_nearest_jsx_config() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("tsconfig.json");
    std::fs::write(&config, r#"{"compilerOptions":{"jsx":"react-jsx"}}"#).unwrap();
    let context = resolve_execution_context(ContextRequest {
        invocation_dir: temp.path(),
        explicit_project_dir: None,
        target_file: None,
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: Some(Path::new("src/component")),
    })
    .unwrap();
    assert_eq!(context.target_source.mode, SourceMode::Tsx);
}

#[test]
fn malformed_ambiguous_jsx_config_falls_back_to_typescript() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("tsconfig.json"), "{not json").unwrap();
    let context = resolve_execution_context(ContextRequest {
        invocation_dir: temp.path(),
        explicit_project_dir: None,
        target_file: None,
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: Some(Path::new("src/component")),
    })
    .unwrap();
    assert_eq!(context.target_source.mode, SourceMode::TypeScript);
}

#[test]
fn virtual_file_path_rejects_lexical_project_escape() {
    let temp = tempfile::tempdir().unwrap();
    let error = resolve_execution_context(ContextRequest {
        invocation_dir: temp.path(),
        explicit_project_dir: None,
        target_file: None,
        test_file: None,
        language: Language::TypeScript,
        virtual_file_path: Some(Path::new("../outside.ts")),
    })
    .unwrap_err();
    assert!(error.to_string().contains("outside project"));
}
