# Court Jester Paper Brief

## Provisional one-sentence contribution
Court Jester is an agent-facing code verifier that improves final task success in repair loops by turning plausible-but-wrong code into concrete failing repros, while maintaining a clean false-positive profile on known-good and upstream replay controls.

## What the repo currently supports

### Strongest supported claims
1. Court Jester can improve end-to-end final success in a verifier-triggered repair loop.
2. The current precision story is materially stronger than earlier versions.
3. The observed lift is caused by verifier-guided repair rather than public-test-guided repair or blind extra retries.

### Core evidence already in the repo
- Primary causal matrix (`core-current`, 39 tasks, 2 models, 4 policies, 3 repeats; `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`):
  - baseline: `208 / 234` = `88.9%`
  - `public-repair-1`: `205 / 234` = `87.6%`
  - `retry-once-no-verify`: `216 / 234` = `92.3%`
  - `repair-loop-verify-only`: `230 / 234` = `98.3%`
  - verify-only vs baseline: `+22` successes, `+9.4` percentage points
  - verify-only vs blind retry: `+14` successes, `+6.0` percentage points
  - verify-only vs public repair: `+25` successes, `+10.7` percentage points
- By model:
  - Claude: `101 / 117` baseline -> `115 / 117` verify-only; `98 / 117` public-repair-1; `108 / 117` retry-once-no-verify
  - Codex: `107 / 117` baseline -> `115 / 117` verify-only; `107 / 117` public-repair-1; `108 / 117` retry-once-no-verify
- Precision controls:
  - local known-good: `80 / 80`
  - external known-good replay: `190 / 190`
  - combined false-positive gauntlet: `270 / 270`
- Public-repair proving ground (`bench/results/matrix/2026-04-19-paper-proving-ground-r3`):
  - baseline: `11 / 36`
  - `public-repair-1`: `14 / 36`
  - `retry-once-no-verify`: `19 / 36`
  - `repair-loop-verify-only`: `25 / 36`
- Two-step robustness (`bench/results/matrix/2026-04-19-paper-core-robustness-r2`):
  - baseline: `137 / 156` = `87.8%`
  - `public-repair-2`: `140 / 156` = `89.7%`
  - `retry-twice-no-verify`: `150 / 156` = `96.2%`
  - `repair-loop-verify-only-2`: `156 / 156` = `100.0%`

## Best current paper thesis
The best current paper is not "we built another verifier." It is:

"Concrete verifier-generated counterexamples improve agent repair-loop outcomes on semantic code tasks without paying for the gain through obvious false positives."

That is specific, measurable, and matched to the actual benchmark package in the repo.

## Recommended claim structure

### Claim 1
Verifier-generated counterexamples improve final task success relative to one-shot editing.
- Evidence: repeated `core-current` benchmark.

### Claim 2
The gain is attributable to verifier-guided repair rather than public-test-guided repair or extra blind search budget.
- Evidence: the primary causal matrix on `core-current` shows `repair-loop-verify-only` at `230 / 234`, ahead of `retry-once-no-verify` at `216 / 234` and `public-repair-1` at `205 / 234`, and that ranking persists in both the proving ground and the two-step robustness package.

### Claim 3
Court Jester's verifier tightening did not buy utility by becoming overly aggressive.
- Evidence: `270 / 270` false-positive gauntlet after verifier tightening.

## What the current repo does NOT yet prove
- Broad readiness on arbitrary external repos.
- Statistical generalization beyond the current repeated benchmark suite.
- Whether lift is concentrated by task family or language in a way that cleanly supports a stronger scientific mechanism claim.

## Honest venue/frame read
This is closer to a systems / benchmarking / agent-tools paper than a pure ML-method paper.

Best fits:
- COLM / ICLR workshop / agent engineering workshop if framed as agent-loop tooling
- NeurIPS/ICML only if the paper becomes more rigorous on controls, ablations, and statistics
- TMLR could work if the benchmark methodology and causal controls are made airtight

## Immediate next experiments needed for a serious submission
1. Add significance analysis over repeated runs.
2. Break results out by model and task family.
3. Add a failure taxonomy table for remaining misses.
4. Decide whether to run the two-step proving-ground matrix or explicitly scope it out.
5. Decide whether the paper is a product benchmark paper or a more general claim about agent-verifier feedback.

## Draft abstract spine
- Problem: agents write plausible code and stop too early.
- Approach: insert a verifier that produces concrete counterexamples before the agent declares success.
- Evaluation: repeated semantic repair benchmark plus known-good and upstream replay precision controls.
- Main result: `208 / 234 -> 230 / 234` on the primary causal matrix, with verifier-guided repair also beating both public repair and blind retry under matched one-step and two-step controls, while precision remains `270 / 270` on the false-positive gauntlet.
- Takeaway: concrete verifier feedback can materially improve agent repair loops without obvious precision collapse.
