//! Bounded local reduction, with a fresh managed process for every observation.
use super::*;

const MAX_OPERATIONS: usize = 32;
const MAX_SECONDS: f64 = 5.0;
const CANDIDATE_MARKER: &str = "__COURT_JESTER_NATIVE_CANDIDATES__";

struct Budget {
    start: Instant,
    seconds: f64,
    operations: usize,
    replays: usize,
}

impl Budget {
    fn remaining(&self) -> Option<f64> {
        let remaining = self.seconds - self.start.elapsed().as_secs_f64();
        (remaining > 0.0 && self.operations < MAX_OPERATIONS).then_some(remaining)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run<'a>(
    budget: &mut Budget,
    context: &ExecutionContext,
    code: String,
    opts: &VerifyOptions<'a>,
    language: &Language,
    project: Option<&'a str>,
    source: Option<&'a str>,
    replay_trial: bool,
) -> Result<String, MinimizationStatus> {
    let seconds = budget
        .remaining()
        .ok_or(MinimizationStatus::BudgetExhausted)?;
    budget.operations += 1;
    budget.replays += usize::from(replay_trial);
    let execution = tokio::time::timeout(
        std::time::Duration::from_secs_f64(seconds),
        execute_generated_harness(
            context,
            code,
            HarnessKind::GeneratedVerifier,
            opts,
            language,
            seconds,
            project,
            source,
        ),
    )
    .await
    .map_err(|_| MinimizationStatus::BudgetExhausted)?;
    let process = execution.process;
    if process.timed_out {
        return Err(MinimizationStatus::BudgetExhausted);
    }
    if process.exit_code != Some(0) || process.memory_error {
        return Err(MinimizationStatus::Failed);
    }
    Ok(process.stdout)
}

fn reproduced(stdout: &str) -> Result<bool, MinimizationStatus> {
    let payload = replay::replay_payload(stdout).map_err(|_| MinimizationStatus::Failed)?;
    let reproduced = payload["reproduced"]
        .as_bool()
        .ok_or(MinimizationStatus::Failed)?;
    let passed = payload["check_passed"]
        .as_bool()
        .ok_or(MinimizationStatus::Failed)?;
    if (reproduced && passed)
        || payload["severity"] != "crash"
        || payload["oracle_kind"] != "runtime_contract"
        || payload["category"] != "exception"
    {
        return Err(MinimizationStatus::Failed);
    }
    Ok(reproduced && !passed)
}

fn candidate_code(arguments: &[ReproValue], language: &Language) -> String {
    let expressions = arguments
        .iter()
        .map(|arg| arg.expression.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    match language {
        Language::Python => format!(
            "import json as _cj_native_json\n_cj_args = [{expressions}]\n{}\n{}",
            include_str!("../synthesize/native/python_value.py"),
            include_str!("native_candidates.py")
        ),
        Language::TypeScript => format!(
            "const _cj_args: any[] = [{expressions}];\n{}\n{}",
            include_str!("../synthesize/native/typescript_value.ts"),
            include_str!("native_candidates.ts")
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidates {
    candidates: Vec<Vec<ReproValue>>,
    truncated: bool,
}

fn candidates(stdout: &str, count: usize) -> Result<Candidates, MinimizationStatus> {
    if stdout.matches(CANDIDATE_MARKER).count() != 1 {
        return Err(MinimizationStatus::Failed);
    }
    let payload = stdout.split_once(CANDIDATE_MARKER).unwrap().1.trim();
    let result: Candidates =
        serde_json::from_str(payload).map_err(|_| MinimizationStatus::Failed)?;
    if result.candidates.len() > 32
        || result
            .candidates
            .iter()
            .any(|row| row.len() != count || row.iter().any(|arg| arg.expression.trim().is_empty()))
    {
        return Err(MinimizationStatus::Failed);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn minimize<'a>(
    findings: &mut [VerificationFinding],
    context: &ExecutionContext,
    target: &str,
    plan: &VerificationPlan,
    opts: &VerifyOptions<'a>,
    language: &Language,
    seconds: f64,
    project: Option<&'a str>,
    source: Option<&'a str>,
) -> serde_json::Value {
    let mut budget = Budget {
        start: Instant::now(),
        seconds: seconds.clamp(0.0, MAX_SECONDS),
        operations: 0,
        replays: 0,
    };
    let mut outcomes = Vec::new();
    for (index, finding) in findings.iter_mut().enumerate() {
        if finding.suppressed || finding.repro.native_replay.is_none() {
            outcomes.push(serde_json::json!({"index":index,"reason":if finding.suppressed {"suppressed"} else {"no_binding_contract"}}));
            continue;
        }
        let contract = finding.repro.native_replay.clone().unwrap();
        let original = finding.repro.arguments.clone();
        let mut best = original.clone();
        let initial_replays = budget.replays;
        let result: Result<MinimizationStatus, MinimizationStatus> = async {
            let snippet = replay::render_native_replay(&best, &contract, language)
                .map_err(|_| MinimizationStatus::Failed)?;
            let stdout = run(
                &mut budget,
                context,
                format!("{target}\n{snippet}"),
                opts,
                language,
                project,
                source,
                true,
            )
            .await?;
            if !reproduced(&stdout)? {
                return Err(MinimizationStatus::Failed);
            }
            let mut seen = std::collections::HashSet::new();
            seen.insert(serde_json::to_string(&best).unwrap());
            loop {
                let stdout = run(
                    &mut budget,
                    context,
                    candidate_code(&best, language),
                    opts,
                    language,
                    project,
                    source,
                    false,
                )
                .await?;
                let proposals = candidates(&stdout, best.len())?;
                let mut accepted = false;
                for candidate in proposals.candidates {
                    if !seen.insert(serde_json::to_string(&candidate).unwrap()) {
                        continue;
                    }
                    if finding.input_classification == InputClassification::Valid
                        && planned_closed_input_classification(
                            &finding.location.function,
                            finding.location.line,
                            &candidate,
                            plan,
                        ) != InputClassification::Valid
                    {
                        continue;
                    }
                    let snippet = replay::render_native_replay(&candidate, &contract, language)
                        .map_err(|_| MinimizationStatus::Failed)?;
                    let stdout = run(
                        &mut budget,
                        context,
                        format!("{target}\n{snippet}"),
                        opts,
                        language,
                        project,
                        source,
                        true,
                    )
                    .await?;
                    if reproduced(&stdout)? {
                        best = candidate;
                        accepted = true;
                        break;
                    }
                }
                if !accepted {
                    return Ok(if proposals.truncated {
                        MinimizationStatus::BudgetExhausted
                    } else if best != original {
                        MinimizationStatus::Preserved
                    } else {
                        MinimizationStatus::NotNeeded
                    });
                }
            }
        }
        .await;
        finding.minimization.status = result.unwrap_or_else(|status| status);
        let attempts = budget.replays - initial_replays;
        finding.minimization.attempts = attempts;
        if best != original {
            finding.repro.snippet =
                replay::render_native_replay(&best, &contract, language).unwrap();
            finding.repro.arguments = best.clone();
            // Raw fuzzer bytes belong only to the original decoder input.
            finding.repro.input_text = None;
            finding.minimization.minimized = Some(ReproCase {
                arguments: best,
                input_text: None,
            });
        }
        outcomes.push(serde_json::json!({"index":index,"status":finding.minimization.status,"attempts":attempts}));
    }
    serde_json::json!({"budget_seconds":budget.seconds,"max_operations":MAX_OPERATIONS,"operations":budget.operations,"outcomes":outcomes})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_and_protocol_fail_closed() {
        let mut budget = Budget {
            start: Instant::now(),
            seconds: 0.0,
            operations: 0,
            replays: 0,
        };
        assert!(budget.remaining().is_none());
        budget.seconds = 5.0;
        assert!(budget.remaining().is_some());
        budget.operations = MAX_OPERATIONS;
        assert!(budget.remaining().is_none());
        for payload in ["", "{}", "{\"reproduced\":true,\"check_passed\":true}", "{\"reproduced\":true,\"check_passed\":false,\"severity\":\"crash\",\"oracle_kind\":\"other\",\"category\":\"exception\"}"] {
            assert!(reproduced(&format!("__COURT_JESTER_REPLAY_JSON__\n{payload}")).is_err());
        }
        let failure = "__COURT_JESTER_REPLAY_JSON__\n{\"reproduced\":true,\"check_passed\":false,\"severity\":\"crash\",\"oracle_kind\":\"runtime_contract\",\"category\":\"exception\"}";
        assert!(reproduced(failure).unwrap());
        assert!(reproduced(&format!("{failure}\n{failure}")).is_err());
        for payload in [
            "{}",
            "{\"candidates\":[[]],\"truncated\":false}",
            "{\"candidates\":[[{\"expression\":\" \"}]],\"truncated\":false}",
        ] {
            assert!(candidates(&format!("{CANDIDATE_MARKER}\n{payload}"), 1).is_err());
        }
    }

    #[test]
    fn runtime_candidates_preserve_native_constructors_and_bound_output() {
        for (language, expression, expected, faithful) in [
            (
                Language::Python,
                "bytearray(b'\\x00\\x80\\xff')",
                "bytearray(b'')",
                false,
            ),
            (Language::Python, "b'\\x80'", "b''", false),
            (Language::Python, "[float('nan')]", "[0.0]", true),
            (Language::Python, "-8", "-1", true),
            (
                Language::TypeScript,
                "new Uint8Array([0,128,255])",
                "new Uint8Array([])",
                false,
            ),
            (
                Language::TypeScript,
                "Buffer.from([128])",
                "Buffer.from([])",
                false,
            ),
            (Language::TypeScript, "new Date(7)", "new Date(0)", false),
            (Language::TypeScript, "8n", "0n", false),
            (
                Language::TypeScript,
                "[NaN, -0, undefined]",
                "[0, -0, undefined]",
                false,
            ),
            (Language::TypeScript, "'😀x'", "\"😀\"", true),
        ] {
            let arguments = vec![ReproValue {
                expression: expression.into(),
                json_value: None,
            }];
            let output = run_candidates(&arguments, &language);
            let proposals = candidates(&output, 1).unwrap();
            assert!(!proposals.truncated);
            let matched = proposals
                .candidates
                .iter()
                .find(|row| row[0].expression == expected)
                .unwrap_or_else(|| panic!("missing {expected}: {output}"));
            assert_eq!(matched[0].json_value.is_some(), faithful, "{output}");
        }
        for language in [Language::Python, Language::TypeScript] {
            let expression =
                serde_json::to_string(&"abcdefghijklmnopqrstuvwxyz0123456789".repeat(2)).unwrap();
            let output = run_candidates(
                &[ReproValue {
                    expression,
                    json_value: None,
                }],
                &language,
            );
            let proposals = candidates(&output, 1).unwrap();
            assert_eq!(proposals.candidates.len(), 32);
            assert!(proposals.truncated);
        }
    }

    fn run_candidates(arguments: &[ReproValue], language: &Language) -> String {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, name) = match language {
            Language::Python => ("python3", "candidates.py"),
            Language::TypeScript => ("node", "candidates.mts"),
        };
        let path = dir.path().join(name);
        std::fs::write(&path, candidate_code(arguments, language)).unwrap();
        let mut command = std::process::Command::new(runtime);
        if *language == Language::TypeScript {
            command.arg("--experimental-transform-types");
        }
        let output = command.arg(path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
