# Figure Plan

## Figure 1 — Main causal matrix
Purpose: show the core claim in one glance.

Data:
- Baseline: 208/234 = 88.9%
- Public repair x1: 205/234 = 87.6%
- Blind retry x1: 216/234 = 92.3%
- Verify-only x1: 230/234 = 98.3%

Design:
- four vertical bars
- label exact numerator/denominator above bars
- annotate deltas from verify-only to retry and public repair
- no in-figure title; use caption

Caption draft:
"Main causal comparison on the repeated `core-current` matrix. Verifier-guided repair outperforms one-shot baseline, public-test-guided repair, and blind retry at matched one-repair budget."

## Figure 2 — Precision controls
Purpose: show the utility result did not come from obvious precision collapse.

Data:
- Local known-good: 80/80
- External replay: 190/190
- Combined gauntlet: 270/270

Design:
- three bars or dots near ceiling
- show 95% Wilson intervals despite ceiling effects

Caption draft:
"False-positive controls after verifier tightening. The completed known-good and upstream replay gauntlets stayed clean."

## Figure 3 — Proving-ground mechanism matrix
Purpose: show public repair was a fair comparator.

Data:
- Baseline: 11/36 = 30.6%
- Public repair x1: 14/36 = 38.9%
- Blind retry x1: 19/36 = 52.8%
- Verify-only x1: 25/36 = 69.4%

Design:
- same layout as Figure 1 for visual consistency
- emphasize that public repair improves over baseline but still trails verify-only

Caption draft:
"Mechanism-focused proving-ground suite. Public-test-guided repair improves over baseline when the task suite is designed to let it fire, but verifier-guided repair still performs best."

## Figure 4 — Two-step robustness by model
Purpose: show the ranking survives more budget.

Data:
Claude:
- baseline 67/78
- public repair x2 66/78
- blind retry x2 75/78
- verify-only x2 78/78

Codex:
- baseline 70/78
- public repair x2 74/78
- blind retry x2 75/78
- verify-only x2 78/78

Design:
- grouped bars by model
- same color per policy across all figures
- show verify-only x2 hitting ceiling on both models

Caption draft:
"Two-step robustness on `core-current`. Additional budget helps both public repair and blind retry, but verifier-guided repair remains best on both models."

## Figure 5 — Residual failures under verify-only
Purpose: keep the paper honest and concrete about what still fails.

Data currently available:
- Claude: 1 hidden_semantic_miss, 1 public_check_failure
- Codex: 2 verify_caught_hidden_bug

Design:
- tiny stacked bars or compact table-style figure
- probably appendix if space gets tight

Caption draft:
"Residual failures under verify-guided repair in the primary causal matrix. Remaining misses are few enough to inspect concretely rather than summarize abstractly."

## Style rules
- use one consistent policy color map across all figures
- use percent on y-axis, but also print counts on bars
- use vector PDF output
- no decorative gradients or dashboard junk
- keep the same ordering everywhere: baseline, public repair, blind retry, verify-only
