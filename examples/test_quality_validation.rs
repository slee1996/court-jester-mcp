//! Fault-injected validation evidence. This does not run generated mutants.
use court_jester::tools::{analyze, test_quality};
use court_jester::types::{Language, SourceContext, SourceMode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn run_cases() -> Value {
    let mut cases = Vec::new();
    for (language, mode, label, source) in [
        (
            Language::Python,
            SourceMode::Python,
            "python",
            "def eligible(value: int) -> bool:\n    return value >= 1\n# λ\n",
        ),
        (
            Language::TypeScript,
            SourceMode::TypeScript,
            "typescript",
            "export function eligible(value: number): boolean { return value >= 1; }\n// λ\n",
        ),
    ] {
        let context = SourceContext {
            language,
            mode,
            source_file: None,
            virtual_file_path: None,
        };
        let analysis = analyze::analyze_with_context(source, &context);
        let functions = analysis.functions.iter().collect::<Vec<_>>();
        let planned = test_quality::plan_mutations(source, language, mode, &functions, 1).unwrap();
        assert_eq!(planned.len(), 1);
        for (fault, expected) in [
            ("valid", "valid"),
            ("stale_source", "invalid_edit"),
            ("invalid_range", "invalid_edit"),
            ("split_utf8", "invalid_edit"),
            ("syntax", "invalid_syntax"),
            ("changed_surface", "changed_surface"),
        ] {
            let mut candidate = planned[0].clone();
            match fault {
                "stale_source" => candidate.original = "outdated source text".into(),
                "invalid_range" => {
                    candidate.start_byte = source.len() + 1;
                    candidate.end_byte = source.len() + 2;
                }
                "split_utf8" => {
                    candidate.start_byte = source.find('λ').unwrap() + 1;
                    candidate.end_byte = candidate.start_byte + 1;
                }
                "syntax" => candidate.replacement = "(".into(),
                "changed_surface" => {
                    candidate.start_byte = source.find("eligible").unwrap();
                    candidate.end_byte = candidate.start_byte + "eligible".len();
                    candidate.original = "eligible".into();
                    candidate.replacement = "renamed".into();
                }
                "valid" => {}
                _ => unreachable!(),
            }
            let (observed, classification, detail) = match test_quality::validate_mutation(
                source, &candidate, &context, &functions,
            ) {
                Ok(validated) => (
                    "valid".to_string(),
                    "valid",
                    json!({"code":validated.code,"required_functions":validated.required_functions.len()}),
                ),
                Err(error) => (
                    serde_json::to_value(error.kind)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    "invalid",
                    serde_json::to_value(error).unwrap(),
                ),
            };
            cases.push(json!({"id":format!("{label}-{fault}"),"language":label,"fault":fault,"expected":expected,"observed":observed,"classification":classification,"matched":observed==expected,"detail":detail,"mutant_execution_started":false}));
        }
    }
    let passed = cases.iter().all(|case| case["matched"] == true);
    json!({"artifact_schema_version":1,"suite":"test-quality-validation-v1","evidence_kind":"fault_injected_validation_boundary_not_generated_runtime_mutants","package_version":env!("CARGO_PKG_VERSION"),"validation_source_sha256":format!("{:x}",Sha256::digest(include_bytes!("../src/tools/test_quality.rs"))),"fixture_source_sha256":format!("{:x}",Sha256::digest(include_bytes!("test_quality_validation.rs"))),"cases":cases,"status":if passed {"passed"} else {"failed"}})
}

#[cfg(not(test))]
fn main() {
    let Some(verifier) = std::env::args_os().nth(1) else {
        eprintln!("usage: test_quality_validation <verifier-binary>");
        std::process::exit(2);
    };
    let mut report = run_cases();
    let verifier = std::fs::read(verifier).expect("read verifier binary for evidence binding");
    let validator = std::fs::read(std::env::current_exe().unwrap())
        .expect("read validator binary for evidence binding");
    report["verifier_binary_sha256"] = json!(format!("{:x}", Sha256::digest(verifier)));
    report["validator_binary_sha256"] = json!(format!("{:x}", Sha256::digest(validator)));
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if report["status"] != "passed" {
        std::process::exit(1);
    }
}
