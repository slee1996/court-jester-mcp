# Court Jester Paper Memo

## Working title
Concrete Verifier Feedback Improves Agent Repair Loops Without Obvious Precision Collapse

## Alternative titles
- Court Jester: Verifier-Triggered Repair for AI Coding Agents
- Concrete Counterexamples for Agentic Code Repair
- Verifier-Guided Repair Loops for Semantic Code Tasks

## One-sentence contribution
Court Jester improves final task success in agent repair loops by generating concrete failing repros before the agent declares success, while remaining clean on the current false-positive controls.

## Abstract
AI coding agents are good at producing plausible code and bad at knowing when they are actually finished. We study whether a verifier that produces concrete failing repros can improve final task success inside an agent repair loop without paying for the gain through excessive false positives. We present Court Jester, a CLI verifier that combines parsing, project-context linting, sandboxed execution, and optional tests, then returns structured failure artifacts that can trigger another repair attempt. We evaluate Court Jester on a repeated semantic repair benchmark with 39 tasks, two frontier model families, and matched controls for one-shot baseline, public-test-guided repair, blind retry without verifier feedback, and verifier-guided repair. On the primary causal matrix, verifier-guided repair reaches 230/234 (98.3%), versus 216/234 (92.3%) for blind retry, 205/234 (87.6%) for public repair, and 208/234 (88.9%) for baseline. On a six-task proving ground designed to favor public repair, verifier-guided repair still leads at 25/36 versus 19/36 for blind retry and 14/36 for public repair. On a two-step robustness rerun, verifier-guided repair remains best at 156/156, ahead of blind retry at 150/156 and public repair at 140/156, while the tightened verifier remains clean on a 270/270 false-positive gauntlet across known-good and upstream replay controls.

## The paper in one paragraph
This is a benchmark-and-systems paper, not a grand unified theory paper. The central claim is narrow and useful: if you give agents concrete counterexamples at the moment they are about to overclaim success, they repair more often into a correct final state. The repo already supports that narrower claim. It does not yet support the larger claim that Court Jester is broadly solved, generally dominant, or ready for arbitrary repo-shaped work.

## Problem
The practical failure mode in agentic coding is not only bad generation. It is premature closure. The model edits code, the patch looks locally plausible, public checks may be thin or absent, and the loop ends without a concrete disproof. That means many failures are semantic rather than syntactic: cross-file contract mismatches, edge-case normalization bugs, and behavior that survives obvious checks but still fails real use.

A useful verifier for agent loops therefore needs to do two things at once:
1. catch failures early enough to trigger another attempt
2. avoid blocking correct code often enough that the loop becomes net harmful

That is the product question this paper should answer.

## Approach
Court Jester is a local CLI with four main commands: `analyze`, `lint`, `execute`, and `verify`. The paper should focus on `verify`.

`verify` composes:
- parse checks
- project-context linting
- sandboxed execution
- optional authoritative tests

The key design choice is not just that verification happens. It is that verification returns a concrete failing repro that can be fed back into the model. The product thesis is that this kind of counterexample is better repair fuel than a vague failure verdict.

## Main evidence already supported by the repo

### 1. End-to-end utility and causal lift
Primary causal matrix (`core-current`):
- 39 tasks
- 2 models: `claude-default`, `codex-default`
- 4 policies: `baseline`, `public-repair-1`, `retry-once-no-verify`, `repair-loop-verify-only`
- 3 repeats
- 936 complete cells
- 234 aggregate scored runs per policy

Result:
- baseline: `208 / 234` = `88.9%`
- public repair: `205 / 234` = `87.6%`
- blind retry: `216 / 234` = `92.3%`
- verify-only repair loop: `230 / 234` = `98.3%`
- verify-only vs baseline: `+22` successes, `+9.4` percentage points
- verify-only vs blind retry: `+14` successes, `+6.0` percentage points
- verify-only vs public repair: `+25` successes, `+10.7` percentage points

This is the main result. It is clean, causal, and much harder to wave away as “just another attempt.”

### 2. Precision did not obviously collapse
False-positive controls after verifier tightening:
- local known-good: `80 / 80`
- external known-good replay: `190 / 190`
- combined false-positive gauntlet: `270 / 270`

This matters because the most obvious skeptical read is: maybe the utility lift came from an over-aggressive verifier. The current package pushes back on that.

### 3. The repair mechanism is now causally benchmarked
Within the primary causal matrix:
- verify-only beat the matched blind-retry control (`230 / 234` vs `216 / 234`)
- verify-only beat the matched public-repair control (`230 / 234` vs `205 / 234`)
- public repair was slightly worse than baseline overall (`205 / 234` vs `208 / 234`)

That means the current gain is not just hidden-evaluator leakage or generic extra search budget. The stronger read is that concrete verifier-generated repros are the useful part.

### 4. Public repair got a fair proving-ground shot and still lost
On the proving-ground suite:
- baseline: `11 / 36`
- public repair: `14 / 36`
- blind retry: `19 / 36`
- verify-only: `25 / 36`

This matters because it removes the easy objection that public repair only looked weak because the main suite did not expose enough visible failure signals.

### 5. The effect survived a larger retry budget
On the two-step robustness rerun:
- baseline: `137 / 156`
- public repair 2: `140 / 156`
- blind retry 2: `150 / 156`
- verify-only 2: `156 / 156`

So the ordering did not collapse once everyone got more search budget.

## Best narrative for the paper
The right narrative is not “verification is good.” That is too broad and too boring. The right narrative is:

Agents often stop at plausible code. Concrete verifier-generated counterexamples convert some of those plausible failures into successful repairs, and they do so without obvious precision collapse on the current control sets.

That story has a clear what, why, and so what:
- What: a verifier-triggered repair loop improves final task success
- Why: concrete failing repros are strong repair signals
- So what: this is a practical way to make coding agents ship fewer silent semantic misses

## What the paper can honestly claim now
- Court Jester improves final success on the current repeated semantic repair benchmark.
- Verify-only repair outperforms both public-test-guided repair and blind extra retries in the primary causal matrix.
- On the proving-ground suite designed to give public repair a fair chance, public repair does help over baseline, but verify-only still wins clearly.
- The two-step `core-current` robustness matrix preserves the same ranking: more budget helps the controls, but verifier-guided repair still finishes best on both models.
- The current false-positive story is materially stronger than the earlier package.
- The tool appears especially well-suited to small semantic repair tasks where plausible-but-wrong code is common.

## What the paper cannot honestly claim yet
- Broad readiness on arbitrary external repos
- Superiority over all possible public-test-driven repair loops
- Superiority over every possible blind-retry control design
- A universal theory of verifier-guided agent improvement
- Strong causal claims about model tier sensitivity beyond what the existing results directly show

## Reviewer-shaped weaknesses
If this were submitted now, the obvious reviewer objections would be:
1. No statistical significance treatment in the paper-ready sense.
2. Limited external validity beyond the curated benchmark suites.
3. Related work and baseline framing are still absent.
4. A two-step proving-ground run is still absent unless we intentionally scope it out.
5. No section yet isolating where lift comes from by task family or failure type.

Those are fixable. But they are real.

## The right next version of the paper
The next serious draft should be a benchmark paper with three pillars:

### Pillar 1: Utility
Show the repeated `core-current` lift clearly.

### Pillar 2: Precision
Show the 270/270 gauntlet clearly and explain why that matters.

### Pillar 3: Attribution
Show that verify-only beats at least one public-repair and one blind-retry control in the same matched matrix, then strengthen that story with the finished proving-ground and two-step robustness runs.

If those three hold, the paper becomes much harder to dismiss.

## Recommended submission posture
Best near-term target: workshop-quality or memo-quality draft.
Best medium-term target: TMLR or a rigorous systems/agent-tooling venue once the causal controls and statistical analysis are in place.

A main-conference submission is possible only if the paper becomes more disciplined on controls, baselines, and significance. Right now the repo has signal, but not yet the kind of closure that survives hostile review.

## Concrete next actions
1. Add paper-ready statistical treatment over repeated runs.
2. Break out results by model family, task family, and failure class.
3. Add paper figures before drafting full prose.
4. Build related work around agentic code repair, execution-based verification, and program-repair feedback loops. Use verified citations only.
5. Decide whether to run the two-step proving-ground matrix or explicitly scope it out in the paper.

## Bottom line
There is a real paper here. But the paper is smaller than the product ambition. That is fine. The smaller true paper is stronger than the bigger inflated one.
