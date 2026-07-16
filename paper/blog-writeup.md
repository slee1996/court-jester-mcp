# Court Jester: Concrete Verifier Feedback Beats Blind Retry for Coding Agents

AI coding agents have a predictable failure mode.

They write code that looks done.
Sometimes it even passes the obvious checks.
And then it fails in exactly the place nobody explicitly disproved.

That is the problem Court Jester is built for.

This is not a CI replacement. It is not a universal software correctness system. It is a hostile verifier that sits in the repair loop and tries to break the patch before the agent gets to declare victory.

The core idea is simple:
- agent edits code
- Court Jester runs immediately after the edit
- if it finds a concrete failing repro, the agent gets another shot
- if it does not, the loop can move on with more confidence

The important part is not just “verification.” It is the kind of feedback. Court Jester tries to turn plausible-but-wrong code into a concrete counterexample the model can actually repair against.

That distinction ended up mattering a lot more than I expected.

## The question
The real product question was never “can the verifier fail code?”
Any aggressive verifier can fail code.

The real question was:

Does verifier-guided repair improve final task success without buying the gain through false positives or through extra retries that would have helped anyway?

That breaks into three separate questions:
- utility: does final success improve?
- precision: does the verifier avoid wrongly blocking correct code?
- attribution: is the gain actually coming from verifier feedback rather than from public test feedback or just another shot?

If you only answer one of those, you do not really know what you built.

## What Court Jester is benchmarking
Court Jester is benchmarked as part of an agent loop, not as a standalone static checker.

The benchmark harness creates a fresh workspace for every run, applies task-specific setup, lets the agent edit code, runs Court Jester when the policy requires it, then runs public and hidden evaluation separately. Hidden checks only score the final output. They do not feed back into the loop.

That separation matters. If hidden evaluation starts telling the model what to fix, you are no longer measuring a realistic repair loop. You are measuring evaluator-assisted patch search.

The main headline suite is called `core-current`.
It is not trying to look like a giant generic repo benchmark. It is a repeated semantic repair suite built to stress the exact failures that make agent loops dangerous:
- cross-file contract mistakes
- canonicalization bugs
- spec-like behavior mismatches
- hidden semantic misses that survive happy-path checks

There are also two precision suites:
- `known-good-corpus`, which asks whether the verifier wrongly fails already-correct local implementations
- `external-known-good-replay`, which applies upstream-derived gold patches and asks whether the verifier wrongly blocks the real fix

And there is one mechanism suite:
- `public-repair-proving-ground`, which is designed so public-test-guided repair actually has a fair chance to help

That last one matters because otherwise a reviewer can always say: “Sure, public repair lost, but maybe the benchmark never really let it fire.”

## The benchmark suites, explicitly
Here is the clean suite map.

### 1. `core-current`
This is the headline utility suite.

What it is:
- a repeated semantic repair benchmark
- small, adversarial tasks rather than giant repo sweeps
- built to produce plausible-but-wrong patches

What it stresses:
- cross-file contract mistakes
- canonicalization bugs
- spec-like behavior mismatches
- hidden semantic failures that survive obvious happy-path checks

What it answers:
- does verifier-guided repair improve final task success on the kind of semantic failures that actually make coding agents dangerous?

### 2. `known-good-corpus`
This is the first precision suite.

What it is:
- already-correct local implementations shipped as benchmark tasks

What it answers:
- does Court Jester wrongly fail code that is already correct?

Why it matters:
- a verifier can always look useful if it becomes aggressive enough
- this suite is the first check against that failure mode

### 3. `external-known-good-replay`
This is the stronger precision suite.

What it is:
- upstream-derived bug tasks
- but instead of asking a model to repair them, the harness applies the task’s known-good gold patch
- then it runs verify, public checks, and hidden checks normally

What it answers:
- does Court Jester wrongly block the actual known-good fix?

Why it matters:
- this is much stronger than a local known-good smoke test
- it tests whether the verifier rejects the real repair on tasks grounded in upstream behavior

### 4. `public-repair-proving-ground`
This is the mechanism suite.

What it is:
- a smaller task set specifically chosen so public-test-guided repair gets a fair shot

How it is biased toward public repair:
- the visible public checks expose a meaningful part of the bug surface
- a failed public check can legitimately trigger another repair attempt
- but a smaller hidden edge remains for final scoring

What it answers:
- if public repair is given a suite where it can actually fire, does verifier-guided repair still win?

Why it matters:
- without this suite, public repair can always be dismissed as underpowered by task choice
- with this suite, public repair becomes a live comparator rather than a token ablation

## The agents we actually benchmarked
This benchmark is against CLI agent systems, not raw single-call model APIs.

Specifically:
- Claude Code via the `claude_cli` provider, configured as `claude-default`, using `claude-opus-4-6` at medium effort
- Codex CLI via the `codex_cli` provider, configured as `codex-default`, using `gpt-5.4` at medium reasoning effort

That is worth being explicit about.

The Codex path runs through `codex exec` in a constrained benchmark mode with user MCP servers disabled, plugins disabled, `--ephemeral`, `--full-auto`, and an explicit output schema.

The Claude path runs through `claude -p` with JSON output, `--setting-sources project,local`, slash commands disabled, browser UI disabled, bypass-permissions mode, and the default tool surface.

So the claims here are not “all Claude models do X” or “all GPT-family models do Y.”
They are claims about these agent systems under this harness.

## The headline result
The primary causal matrix compares four conditions on the repeated `core-current` suite:
- baseline: one shot, no repair loop
- public repair: one extra attempt driven only by visible public test failure
- blind retry: one extra attempt with no verifier or evaluator feedback
- verify-only: one extra attempt driven only by Court Jester

Results:
- baseline: 208/234 = 88.9%
- public repair: 205/234 = 87.6%
- blind retry: 216/234 = 92.3%
- verify-only: 230/234 = 98.3%

Primary result source: [`docs/benchmark-2026-04-20.md`](../docs/benchmark-2026-04-20.md). That document records the task set, policies, repeats, and result artifacts for the causal matrix.

That is the main result.

Notably:
- verify-only beat baseline by 9.4 percentage points
- verify-only beat blind retry by 6.0 points
- verify-only beat public repair by 10.7 points

That means the gain is not just “more attempts help.”
It also means the gain is not “visible public tests were enough.”

The strongest simple read is:
concrete verifier-generated repros are a better repair signal than either blind extra search or visible-test-only repair.

## By agent system
The ranking holds on both evaluated agent systems.

On the one-step causal matrix:

Claude Code:
- baseline: 101/117
- public repair: 98/117
- blind retry: 108/117
- verify-only: 115/117

Codex CLI:
- baseline: 107/117
- public repair: 107/117
- blind retry: 108/117
- verify-only: 115/117

The gain is larger on Claude Code, but it is not carried by Claude alone.
Codex CLI still improves materially under verifier-guided repair.

## Precision: did we buy the gain by over-failing good code?
This is the obvious skeptical read, so it had to be tested directly.

False-positive controls:
- local known-good: 80/80
- external replay: 190/190
- combined gauntlet: 270/270

The right claim is not “Court Jester can never false-positive.”
The right claim is narrower and stronger:
across the completed known-good and upstream replay controls, the verifier stayed clean.

That matters because a lot of verification stories look good only because the tool got more aggressive. This one did not need that crutch in the completed control package.

## The proving ground: public repair got a fair chance
One possible objection to the headline result is that public repair might simply be underpowered by task choice. If the public tests do not expose enough of the bug surface, of course verifier-guided repair will look better.

That is why I built the `public-repair-proving-ground` suite.

These tasks are chosen so that the visible public checks expose a meaningful part of the bug surface and can legitimately trigger another repair attempt, while a smaller hidden edge still remains for final scoring.

In other words: public repair gets a fair shot.

Results:
- baseline: 11/36 = 30.6%
- public repair: 14/36 = 38.9%
- blind retry: 19/36 = 52.8%
- verify-only: 25/36 = 69.4%

This is one of the most important results in the package.

Because now the argument is not just:
“public repair lost on the main suite.”

It is:
“public repair does help when the task mix is designed to let it help, and verifier-guided repair still wins clearly.”

That is a much stronger statement.

## Robustness: what happens if you give the controls more budget?
Another obvious objection is that maybe verifier-guided repair only wins at one extra attempt. Maybe blind retry or public repair catches up if you just give them more shots.

So I ran a two-step robustness matrix on `core-current`.

Claude Code:
- baseline: 67/78
- public repair x2: 66/78
- blind retry x2: 75/78
- verify-only x2: 78/78

Codex CLI:
- baseline: 70/78
- public repair x2: 74/78
- blind retry x2: 75/78
- verify-only x2: 78/78

This is the right kind of robustness result.
More budget helps the controls. Good. That means they are live.

But verifier-guided repair still finishes best on both systems.

So the story now is not fragile. It is not hanging on one lucky one-step comparison.

Aggregated across both agent systems, the two-step matrix was:
- baseline: 137/156 = 87.8%
- public repair x2: 140/156 = 89.7%
- blind retry x2: 150/156 = 96.2%
- verify-only x2: 156/156 = 100.0%

The important thing is not the perfect-looking top-line number. It is that the skeptical controls moved too. More budget helped public repair and blind retry, and verifier-guided repair still stayed ahead.

## What still fails
Court Jester is not magic. The paper should be honest about that.

On the primary causal matrix, the remaining verify-only residuals were:
- Claude: 1 `hidden_semantic_miss`, 1 `public_check_failure`
- Codex: 2 `verify_caught_hidden_bug`

That last category matters. In those Codex cases, the verifier prevented a bad final patch from being counted as success.
So even the misses are informative.

The right posture here is not “we solved coding-agent correctness.”
It is:
we made the residual error surface smaller, more visible, and more concrete.

## What this paper is actually claiming
Not:
- that I invented program repair
- that iterative feedback is new
- that execution-aware repair is new
- that Court Jester is broadly ready for arbitrary external repos
- that every Claude or GPT-family model behaves this way

The real claim is smaller:

Concrete verifier-generated counterexamples are a strong repair signal for coding agents, and that claim survives matched comparisons against public-test-guided repair, blind retry, false-positive controls, and a larger retry budget.

That is enough.
It is a real paper result.

## Reproducibility notes
The benchmark package is documented in [`docs/benchmark-2026-04-20.md`](../docs/benchmark-2026-04-20.md), with the suite names, policy names, repeat counts, and artifact paths used for the claims above.

The main matrix was run with:

```bash
python3 -m bench.run_matrix \
  --task-set core-current \
  --models claude-default,codex-default \
  --policies baseline,public-repair-1,retry-once-no-verify,repair-loop-verify-only \
  --repeats 3 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-18-paper-core-causal-r3-v2
```

The precision controls were the local `known-good-corpus` and the upstream-derived `external-known-good-replay` lanes. Those are the source of the 80/80, 190/190, and combined 270/270 false-positive-control claims.

Public source: https://github.com/slee1996/court-jester-mcp


## Why I think this matters
Most discussion about coding agents still collapses too many layers together.
Model quality. Prompting. Tooling. Test coverage. Search budget. Repair loop structure. Hidden evaluation leakage.

If you want to know what actually helps, you have to separate them.

What Court Jester seems to show is that there is something especially useful about turning “this patch seems wrong” into “here is the concrete failing behavior.”

That sounds obvious after the fact.
Most important product truths do.

But obvious-sounding is not the same thing as measured.
Now it is measured.
