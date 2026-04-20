# Experiment Log

## Contribution (one sentence)
Court Jester improves verifier-triggered agent repair outcomes on semantic code tasks while preserving a clean false-positive profile on known-good controls.

## Experiments Run

### Experiment 1: Primary causal matrix on `core-current` (2026-04-18)
- Claim tested: Court Jester improves final task success in an agent loop, and the gain is specifically attributable to verifier-guided repair rather than public-test-guided repair or blind extra retries.
- Setup: `core-current`, 39 tasks, models `claude-default` and `codex-default`, policies `baseline`, `public-repair-1`, `retry-once-no-verify`, and `repair-loop-verify-only`, 3 repeats, blocked-random schedule, shuffle seed 7.
- Key result: aggregate success was `208 / 234` for baseline, `205 / 234` for `public-repair-1`, `216 / 234` for `retry-once-no-verify`, and `230 / 234` for `repair-loop-verify-only`.
- Result files: `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`
- Figures generated: none yet in `paper/`
- Surprising findings: verify-only beat both matched-attempt controls, and `public-repair-1` was slightly worse than baseline overall.

### Experiment 2: Local known-good false-positive control
- Claim tested: verifier tightening did not introduce obvious false positives on already-correct local implementations.
- Setup: `known-good-corpus`, model `noop`, policy `required-final`, 10 repeats.
- Key result: `80 / 80` clean passes.
- Result files: `bench/results/matrix/2026-04-18-known-good-corpus-r10`
- Figures generated: none yet in `paper/`
- Surprising findings: none recorded.

### Experiment 3: External upstream replay control
- Claim tested: Court Jester does not wrongly block known-good upstream-derived fixes.
- Setup: `external-known-good-replay`, model `noop`, policy `required-final`, 10 repeats, `--use-task-gold-patches`.
- Key result: `190 / 190` clean passes.
- Result files: `/tmp/cj-external-known-good-replay-r10-v2`
- Figures generated: none yet in `paper/`
- Surprising findings: authoritative run had to be moved to `/tmp` because repo-local workspace placement introduced a path-sensitive benchmark artifact.

### Experiment 4: Model-level read from the primary causal matrix
- Claim tested: the causal advantage of verifier-guided repair is not carried by only one model family.
- Setup: model-level breakout from `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`.
- Key result: Claude improved from `101 / 117` to `115 / 117` under verify-only, versus `98 / 117` for public repair and `108 / 117` for blind retry; Codex improved from `107 / 117` to `115 / 117` under verify-only, versus `107 / 117` for public repair and `108 / 117` for blind retry.
- Result files: `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`
- Figures generated: none yet in `paper/`
- Surprising findings: the verifier win is larger on Claude, but still clearly present on Codex.

### Experiment 5: Proving-ground mechanism matrix
- Claim tested: public-test-guided repair is being given a fair chance to help, rather than being underpowered by the headline task suite.
- Setup: `public-repair-proving-ground`, models `claude-default` and `codex-default`, policies `baseline`, `public-repair-1`, `retry-once-no-verify`, and `repair-loop-verify-only`, 3 repeats.
- Key result: aggregate success was `11 / 36` for baseline, `14 / 36` for `public-repair-1`, `19 / 36` for `retry-once-no-verify`, and `25 / 36` for `repair-loop-verify-only`.
- Result files: `bench/results/matrix/2026-04-19-paper-proving-ground-r3`
- Figures generated: none yet in `paper/`
- Surprising findings: public repair did improve over baseline on the suite designed to favor it, but verify-only still won clearly.

## Figures
| Filename | Description | Which section it belongs in |
|----------|-------------|---------------------------|
| [TBD] | Four-way causal bar chart: baseline vs public repair vs blind retry vs verify-only | Results, main figure |
| [TBD] | Precision controls summary (`80/80`, `190/190`, `270/270`) | Results / precision subsection |
| [TBD] | Policy / repair-trigger diagram | Method / benchmark design |
| [TBD] | Residual failure breakdown under verify-only | Results / failure analysis |

## Failed Experiments / Limits
- Current package does not yet prove broad readiness on arbitrary external repos.
- Two-step robustness packages are still missing from the full causal runbook.
- Remaining verify-only non-successes in the primary causal matrix: Claude had one `hidden_semantic_miss` and one `public_check_failure`; Codex had two `verify_caught_hidden_bug` results.

## Open Questions
- Does the verifier advantage survive the two-step robustness packages?
- Is the gain concentrated in certain task families or languages?
- Is the right venue a major conference, TMLR, or a strong workshop?
