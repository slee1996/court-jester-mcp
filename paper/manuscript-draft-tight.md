# Concrete Verifier Feedback Improves Agent Repair Loops Without Obvious Precision Collapse

## Abstract
AI coding agents often stop at plausible code rather than correct code. We study whether a verifier that produces concrete failing repros can improve final task success inside an agent repair loop without paying for the gain through excessive false positives. We present Court Jester, a CLI verifier that combines parsing, project-context linting, sandboxed execution, and optional tests, then returns structured failure artifacts that can trigger another repair attempt. We evaluate Court Jester on a repeated semantic repair benchmark with 39 tasks, two model families, and four matched policies: one-shot baseline, public-test-guided repair, blind retry without verifier feedback, and verifier-guided repair. On the primary causal matrix, verifier-guided repair reaches 230/234 (98.3%), versus 216/234 (92.3%) for blind retry, 205/234 (87.6%) for public repair, and 208/234 (88.9%) for baseline. On a proving-ground suite designed to give public repair a fair chance to help, public repair improves over baseline, but verifier-guided repair still performs best. The tightened verifier also remains clean on a 270/270 false-positive gauntlet across known-good and upstream replay controls. These results support a narrow but useful claim: concrete verifier-generated counterexamples are a strong repair signal for coding agents under matched control comparisons.

## 1. Introduction
AI coding agents often fail in a specific way: they stop at plausible code rather than correct code. This is especially costly on semantic repair tasks, where a patch looks locally coherent, may satisfy obvious visible checks, and still fails under edge cases, cross-file interactions, or hidden behavior constraints. In practice, the problem is often not that the model cannot produce code. The problem is that the loop ends before anything concretely disproves the candidate patch.

One-shot editing and thin public checks are weak defenses against this failure mode. A model can generate a patch that looks reasonable, survive a shallow visible test surface, and still ship the wrong behavior. What is missing at the decision boundary is often a concrete counterexample: an executable failure artifact that tells the model, and the loop around it, that the current patch is wrong in a specific way.

We study whether this kind of counterexample improves repair-loop outcomes. We present Court Jester, an agent-facing CLI verifier designed to run immediately after an edit and before the agent declares success. Its core command, `verify`, combines parsing, project-context linting, sandboxed execution, and optional tests, then returns structured failure artifacts that can feed the next repair attempt.

The product question is simple: does verifier-guided repair improve final task success without introducing enough false positives to make the loop net harmful? To answer that question, we separate three things that agent benchmarks often blur together: utility, precision, and attribution. Utility asks whether final task success improves. Precision asks whether the verifier wrongly blocks already-correct code. Attribution asks whether any improvement comes from verifier-generated feedback rather than from extra attempts or visible public-test feedback.

We evaluate Court Jester on a repeated semantic repair benchmark with matched control policies. On the primary causal matrix, verifier-guided repair reaches 230/234 (98.3%), compared with 216/234 (92.3%) for blind retry, 205/234 (87.6%) for public-test-guided repair, and 208/234 (88.9%) for baseline. On a proving-ground suite designed to give public repair a fair chance to help, public repair improves over baseline (14/36 vs. 11/36), but verifier-guided repair still performs best at 25/36. On a completed two-step robustness matrix, giving more budget to the controls strengthens both public repair and blind retry, yet verifier-guided repair remains best on both models. At the same time, the tightened verifier stays clean on a 270/270 false-positive gauntlet across local known-good and upstream replay controls.

This paper makes four contributions:
- We present Court Jester, a verifier designed for agent repair loops rather than human-only inspection.
- We introduce a benchmark methodology that separates utility, precision, and repair attribution.
- We show that verifier-guided repair reaches 230/234 on the primary causal matrix, beating blind retry at 216/234, public repair at 205/234, and baseline at 208/234.
- We show that the tightened verifier remains clean on a 270/270 false-positive gauntlet across known-good and upstream replay controls.

The paper does not claim broad arbitrary-repo validity. It studies verifier-guided repair on curated semantic code tasks and precision controls designed to stress the failure modes most relevant to agent loops.

## 2. Court Jester
Court Jester is a local CLI with four main commands: `analyze`, `lint`, `execute`, and `verify`. This paper focuses on `verify`, because it is the command that sits directly inside an agent repair loop.

`verify` composes several checks into one machine-actionable verdict. It parses the target file, optionally applies a complexity gate, runs linting in the target project context, executes synthesized or provided checks inside a sandbox, and optionally runs an authoritative test file. The goal is not to produce a long human-facing report. The goal is to produce a small number of structured outcomes and, when possible, a concrete failing repro that an agent can use immediately.

That distinction matters. Many systems can tell a model that something failed. Court Jester is designed to tell the loop why the current patch failed in a form that can trigger a repair. The point is not merely to reject bad code. The point is to convert plausible-but-wrong patches into concrete counterexamples before the loop ends.

## 3. Benchmark Methodology
The benchmark asks one product question: does Court Jester improve final task success in an agent loop without introducing enough false positives to make it net harmful?

To answer that question honestly, we separate suites by role. The headline utility suite is `core-current`, a repeated semantic repair benchmark. Precision is measured separately with two false-positive controls: `known-good-corpus`, which contains already-correct local implementations, and `external-known-good-replay`, which applies gold patches for upstream-derived tasks and asks whether the verifier wrongly blocks the known-good fix. We also use `public-repair-proving-ground`, a smaller suite designed so that public-test-guided repair has a fair chance to help.

We also separate policies by what can trigger another attempt. `baseline` is one-shot editing with no repair loop. `public-repair-1` allows one additional attempt driven by visible public-test feedback. `retry-once-no-verify` allows one additional attempt without verifier or evaluator feedback, isolating extra search budget. `repair-loop-verify-only` allows one additional attempt driven only by Court Jester `verify`. The two-step robustness matrix uses the corresponding two-repair variants: `public-repair-2`, `retry-twice-no-verify`, and `repair-loop-verify-only-2`.

The primary unit of analysis is the benchmark cell: task × model × policy × repeat. We report successes, rates, Wilson intervals as descriptive uncertainty summaries, and absolute deltas in percentage points. We do not interpret these runs as an iid sample of arbitrary repositories.

## 4. Results
### 4.1 Primary causal matrix
The main result comes from the repeated `core-current` causal matrix with 39 tasks, two models, four matched policies, and three repeats per condition.

| Policy | Successes | Rate | 95% Wilson CI |
|-------|-----------|------|---------------|
| Baseline | 208/234 | 88.9% | [84.2%, 92.3%] |
| Public repair x1 | 205/234 | 87.6% | [82.8%, 91.2%] |
| Blind retry x1 | 216/234 | 92.3% | [88.2%, 95.1%] |
| Verify-only x1 | 230/234 | 98.3% | [95.7%, 99.3%] |

Verifier-guided repair improves over baseline by 9.4 percentage points, over blind retry by 6.0 points, and over public-test-guided repair by 10.7 points. The key point is not merely that verify-only beats baseline. It also beats the two stronger skeptical alternatives: visible-test-guided repair and blind extra search.

By model, the same ranking broadly holds. On the one-step causal matrix, Claude improves from 101/117 to 115/117 under verify-only, while public repair reaches 98/117 and blind retry reaches 108/117. Codex improves from 107/117 to 115/117 under verify-only, while public repair remains at 107/117 and blind retry reaches 108/117.

### 4.2 Precision controls
The utility result would be much less interesting if it came from an over-aggressive verifier.

| Suite | Successes | Rate | 95% Wilson CI |
|------|-----------|------|---------------|
| Local known-good | 80/80 | 100.0% | [95.4%, 100.0%] |
| External replay | 190/190 | 100.0% | [98.0%, 100.0%] |
| Combined gauntlet | 270/270 | 100.0% | [98.6%, 100.0%] |

The right claim is not “the verifier can never false-positive.” The right claim is narrower: after verifier tightening, the completed known-good and upstream replay controls stayed clean.

### 4.3 Proving-ground mechanism matrix
A reviewer could still argue that public repair only looks weak because the headline suite is a bad place for public tests to help. The proving-ground suite addresses that objection.

| Policy | Successes | Rate | 95% Wilson CI |
|-------|-----------|------|---------------|
| Baseline | 11/36 | 30.6% | [18.0%, 46.9%] |
| Public repair x1 | 14/36 | 38.9% | [24.8%, 55.1%] |
| Blind retry x1 | 19/36 | 52.8% | [37.0%, 68.0%] |
| Verify-only x1 | 25/36 | 69.4% | [53.1%, 82.0%] |

This suite shows two things at once. Public repair is a live comparator: it improves over baseline when the tasks are chosen to let visible tests matter. Verifier-guided repair still wins clearly.

### 4.4 Two-step robustness on `core-current`
We also tested whether the ranking survives more repair budget.

| Model | Baseline | Public repair x2 | Blind retry x2 | Verify-only x2 |
|------|----------|------------------|----------------|----------------|
| Claude | 67/78 | 66/78 | 75/78 | 78/78 |
| Codex | 70/78 | 74/78 | 75/78 | 78/78 |

More budget helps both public repair and blind retry. That is good: it means the controls are live. But verifier-guided repair still finishes best on both models.

### 4.5 Residual failures
Verifier-guided repair is not magic, and the paper should say so plainly. On the primary causal matrix, the remaining verify-only residuals are small enough to discuss concretely. Claude has one `hidden_semantic_miss` and one `public_check_failure`. Codex has two `verify_caught_hidden_bug` outcomes. These residuals show both where the loop still fails and where the verifier successfully prevents a bad final patch from being miscounted as a success.

## 5. Discussion
The main takeaway is narrow but useful: concrete verifier-generated repros improve repair-loop outcomes on the current benchmarked semantic task pool. The value is not only bug detection in isolation. The value is closing the premature-success gap in agent loops.

The paper is not saying that verification in general is good. It is not saying that public tests are useless. It is not saying that extra retries never help. In fact, the robustness and proving-ground results show the opposite: visible public feedback and extra budget can help materially. The stronger claim is that concrete verifier-generated failure artifacts appear to be a better repair signal under matched comparisons.

The results also suggest a product-shaping lesson. Court Jester is strongest where plausible-but-wrong code is common: semantic repair tasks, cross-file issues, and spec-like behavior where edge cases matter. It is not a CI replacement or a universal agent-evaluation oracle. It is a hostile verifier designed to sit in the repair loop when “looks done” is not good enough.

## 6. Related Work
Large language models have already been studied as engines for automated program repair. Jiang et al. analyze the impact of code language models on automated program repair and show that code-specialized models can materially affect repair performance on APR tasks (Jiang et al., 2023; DOI: 10.1109/ICSE48619.2023.00125). Ribeiro likewise frames large language models as a viable repair mechanism in automated program repair settings (Ribeiro, 2023; DOI: 10.1145/3618305.3623587). Our paper sits adjacent to this literature but asks a different question. We are not primarily comparing model architectures for patch generation. We are studying whether a verifier that produces concrete failing repros improves final outcomes inside an agent repair loop.

Our work is also related to iterative refinement and self-correction for language models. Self-Refine shows that models can improve outputs through iterative self-feedback (Alon et al., 2023; DOI: 10.52202/075280-2019), and Reflexion studies language agents that improve behavior via explicit verbal reinforcement and retry (Cassano et al., 2023; DOI: 10.52202/075280-0377). Court Jester differs in the source and structure of the feedback. Instead of relying on open-ended self-critique or verbal reflection, it injects externally grounded failure artifacts generated by parsing, execution, and tests. The main claim is therefore not that iteration helps in the abstract, but that concrete verifier-generated counterexamples are a strong repair signal under matched control comparisons.

A third related thread studies execution-aware language-model behavior. Code Execution with Pre-trained Language Models argues that execution can be integrated into language-model reasoning rather than treated as a purely downstream evaluator (Liu et al., 2023; DOI: 10.18653/v1/2023.findings-acl.308). Our setting is different from direct code-execution modeling, but philosophically aligned: execution is useful not only for scoring code, but for generating informative feedback that changes what the model does next.

Finally, our evaluation framing relates to the recent shift toward more realistic software-engineering benchmarks for language models and agents. SWE-bench established issue-resolution on real GitHub repositories as a serious benchmark target for model evaluation (verified via arXiv title search: SWE-bench: Can Language Models Resolve Real-World GitHub Issues?, arXiv:2310.06770v3). Court Jester is narrower in scope. We do not attempt to solve general repository-level software engineering. Instead, we isolate a product question inside agent loops: whether verifier-guided repair improves final task success without paying for that gain through obvious precision collapse. That is why our benchmark package emphasizes causal controls and false-positive controls alongside task success.

Counterexample-guided repair work provides a final conceptual bridge. Orvalho et al. show that counterexamples can serve as active ingredients in program repair pipelines rather than mere post hoc diagnostics (Orvalho et al., 2025; DOI: 10.1609/aaai.v39i1.32046). Our contribution is not to introduce counterexamples as a concept, but to show that verifier-generated counterexamples are a practically strong repair signal for coding agents under a benchmark package that separates utility, precision, and attribution.

## 7. Limitations
This paper has four important limitations. First, the benchmark remains curated. The task pool is designed to stress semantic repair behavior that matters for agent loops, but it is not a random sample of external repositories. Second, the paper does not establish broad arbitrary-repo readiness. The right scope is narrower: verifier-guided repair on the completed benchmark package. Third, a two-step proving-ground robustness run is still absent unless we intentionally scope it out. Fourth, the paper is about loop behavior, not a universal theory of verifier-guided improvement across all model tiers, languages, or engineering environments.

These limitations do not undermine the central claim. They define it.

## 8. Conclusion
Court Jester supports a smaller but stronger claim than the broadest product ambition. Concrete verifier-generated counterexamples can improve coding-agent repair loops on semantic tasks without obvious precision collapse on the completed control package. Verify-only repair beats matched public-test-guided repair and blind retry in the headline causal matrix, still wins on the suite designed to favor public repair, and remains best under a completed two-step core robustness run. The remaining work is not to invent a story. It is to package this one cleanly, honestly, and with enough quantitative and qualitative detail to survive review.
