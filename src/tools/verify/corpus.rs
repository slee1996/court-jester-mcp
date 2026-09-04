//! Saved fuzz inputs and their conversion back into verification-plan inputs.

use crate::tools::domain;
use crate::types::{
    DomainSource, DomainSourceKind, FunctionInfo, InputClassification, Language, PlannedArguments,
    PlannedInput,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) type PersistentCorpus = BTreeMap<String, Vec<Vec<serde_json::Value>>>;

const CORPUS_MARKER: &str = "__COURT_JESTER_CORPUS_JSON__";

fn stable_corpus_key(source_file: Option<&str>, language: &Language) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in format!("{language:?}:{}", source_file.unwrap_or("<inline>")).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn persistent_corpus_path(
    output_dir: Option<&str>,
    source_file: Option<&str>,
    language: &Language,
) -> Option<PathBuf> {
    output_dir.map(|directory| {
        Path::new(directory).join(format!(
            ".court-jester-corpus-{}.json",
            stable_corpus_key(source_file, language)
        ))
    })
}

pub(super) fn read_persistent_corpus(path: &Path) -> PersistentCorpus {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub(super) fn parse_corpus(stdout: &str) -> PersistentCorpus {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(CORPUS_MARKER))
        .and_then(|payload| serde_json::from_str(payload).ok())
        .unwrap_or_default()
}

pub(super) fn corpus_inputs(
    corpus: &PersistentCorpus,
    functions: &[FunctionInfo],
    language: &Language,
    source_file: Option<&str>,
) -> Vec<PlannedInput> {
    let mut inputs = Vec::new();
    for function in functions.iter().filter(|function| !function.is_nested) {
        let surface_id = format!("{}:{}", function.name, function.line);
        let Some(rows) = corpus.get(&surface_id) else {
            continue;
        };
        let params = function
            .params
            .iter()
            .filter(|param| !param.is_variadic())
            .collect::<Vec<_>>();
        for row in rows.iter().take(64) {
            if row.len() != params.len() {
                continue;
            }
            let mut positional = Vec::new();
            let mut named = BTreeMap::new();
            for (param, value) in params.iter().zip(row) {
                let literal = domain::literal_from_json_value(value.clone(), language);
                if matches!(language, Language::Python) && param.keyword_only {
                    named.insert(param.name.clone(), literal);
                } else {
                    positional.push(literal);
                }
            }
            inputs.push(PlannedInput {
                surface_id: surface_id.clone(),
                arguments: PlannedArguments { positional, named },
                classification: InputClassification::Unknown,
                sources: vec![DomainSource {
                    kind: DomainSourceKind::CoverageCorpus,
                    symbol: Some(function.name.clone()),
                    source_file: source_file.map(str::to_string),
                    line: None,
                }],
            });
        }
    }
    inputs
}

pub(super) fn persist_corpus(path: Option<&Path>, update: &PersistentCorpus) -> usize {
    let Some(path) = path else {
        return update.values().map(Vec::len).sum();
    };
    let mut corpus = read_persistent_corpus(path);
    for (surface, rows) in update {
        let retained = corpus.entry(surface.clone()).or_default();
        for row in rows {
            let duplicate = retained.iter().any(|existing| existing == row);
            if !duplicate && retained.len() < 64 {
                retained.push(row.clone());
            }
        }
    }
    let retained_count = corpus.values().map(Vec::len).sum();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_vec_pretty(&corpus) {
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, content).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
    retained_count
}
