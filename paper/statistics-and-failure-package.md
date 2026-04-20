# Statistics and Failure-Analysis Package

## Purpose
This document turns the completed control package into paper-ready quantitative claims without overclaiming.

The right posture is descriptive and honest:
- report proportions clearly
- report uncertainty bands
- report effect sizes in percentage points
- avoid pretending these runs imply arbitrary-repo generalization
- treat Wilson intervals here as descriptive uncertainty summaries for observed success rates

## Recommended statistical framing

### Unit of analysis
Primary unit: benchmark cell = task × model × policy × repeat.

That should be stated explicitly in the paper. It matters because the paper is evaluating loop outcomes on a repeated benchmark harness, not estimating a population parameter for arbitrary repositories.

### What to report
For each main matrix:
1. successes / total
2. success rate
3. 95% Wilson interval
4. absolute deltas in percentage points versus key comparators

### What not to do
- Do not imply iid real-world repo sampling.
- Do not bury the denominator.
- Do not use the transient `159/157` aggregate artifact from the robustness flush.
- Do not oversell tiny differences between weak comparators when the paper's central story is the larger verify-only advantage.

## Main completed package

### A. Precision controls

| Suite | Successes | Rate | 95% Wilson CI |
|------|-----------|------|---------------|
| Local known-good | 80/80 | 100.0% | [95.4%, 100.0%] |
| External replay | 190/190 | 100.0% | [98.0%, 100.0%] |
| Combined gauntlet | 270/270 | 100.0% | [98.6%, 100.0%] |

Interpretation:
The precision story is unusually strong for this kind of benchmark package. The right claim is not “zero possible false positives forever.” The right claim is “the current tightened verifier stayed clean across the completed known-good and upstream replay controls.”

### B. Primary causal matrix (`core-current`, one repair)

| Policy | Successes | Rate | 95% Wilson CI |
|-------|-----------|------|---------------|
| Baseline | 208/234 | 88.9% | [84.2%, 92.3%] |
| Public repair x1 | 205/234 | 87.6% | [82.8%, 91.2%] |
| Blind retry x1 | 216/234 | 92.3% | [88.2%, 95.1%] |
| Verify-only x1 | 230/234 | 98.3% | [95.7%, 99.3%] |

Key deltas:
- verify-only vs baseline: +9.4 percentage points
- verify-only vs blind retry: +6.0 percentage points
- verify-only vs public repair: +10.7 percentage points
- blind retry vs baseline: +3.4 percentage points
- public repair vs baseline: -1.3 percentage points

Interpretation:
This is the paper's main causal table. Verify-only does not merely beat baseline; it also beats the two better skeptical alternatives: public-test-guided repair and blind extra search.

### C. Proving-ground mechanism matrix

| Policy | Successes | Rate | 95% Wilson CI |
|-------|-----------|------|---------------|
| Baseline | 11/36 | 30.6% | [18.0%, 46.9%] |
| Public repair x1 | 14/36 | 38.9% | [24.8%, 55.1%] |
| Blind retry x1 | 19/36 | 52.8% | [37.0%, 68.0%] |
| Verify-only x1 | 25/36 | 69.4% | [53.1%, 82.0%] |

Key deltas:
- verify-only vs public repair: +30.6 percentage points
- verify-only vs blind retry: +16.7 percentage points
- public repair vs baseline: +8.3 percentage points

Interpretation:
This suite matters because it rescues the public-repair comparator from the easy dismissal that it never had a real chance to work. Public repair does improve over baseline here. Verify-only still wins clearly.

### D. Two-step `core-current` robustness matrix

Use the stable completed per-model summaries as authoritative.

| Model | Baseline | Public repair x2 | Blind retry x2 | Verify-only x2 |
|------|----------|------------------|----------------|----------------|
| Claude | 67/78 | 66/78 | 75/78 | 78/78 |
| Codex | 70/78 | 74/78 | 75/78 | 78/78 |

Per-model rates and 95% Wilson intervals:

| Model / Policy | Rate | 95% Wilson CI |
|---------------|------|---------------|
| Claude baseline | 85.9% | [76.5%, 91.9%] |
| Claude public repair x2 | 84.6% | [75.0%, 91.0%] |
| Claude blind retry x2 | 96.2% | [89.3%, 98.7%] |
| Claude verify-only x2 | 100.0% | [95.3%, 100.0%] |
| Codex baseline | 89.7% | [81.0%, 94.7%] |
| Codex public repair x2 | 94.9% | [87.5%, 98.0%] |
| Codex blind retry x2 | 96.2% | [89.3%, 98.7%] |
| Codex verify-only x2 | 100.0% | [95.3%, 100.0%] |

Interpretation:
More budget helps both public repair and blind retry. That is good; it means the controls are live. But verifier-guided repair still finishes best on both models. That is the robustness result the paper needs.

## Failure-analysis package

### Failure categories already in play
Use the categories already present in the benchmark docs and summaries:
- `hidden_semantic_miss`
- `verify_caught_hidden_bug`
- `public_check_failure`
- optionally separate provider / infrastructure failures only when discussing older benchmark hygiene, not main final tables

### Current paper-ready residual failures
#### Primary causal matrix, verify-only residuals
- Claude:
  - 1 `hidden_semantic_miss`
  - 1 `public_check_failure`
- Codex:
  - 2 `verify_caught_hidden_bug`

Paper interpretation:
- residual errors remain
- verify-only is not magic
- the remaining misses are small enough that the paper can discuss them concretely rather than hiding them in a generic limitations paragraph

### Failure-table template for final paper

| Matrix | Policy | Failure type | Count | Interpretation |
|-------|--------|--------------|-------|----------------|
| Core causal | Verify-only | hidden_semantic_miss | 1 | residual semantic miss on Claude |
| Core causal | Verify-only | public_check_failure | 1 | repair loop still failed visible check on Claude |
| Core causal | Verify-only | verify_caught_hidden_bug | 2 | verifier prevented bad Codex finals from counting as success |

This should be expanded with baseline and retry failure mixes once raw summarized counts are extracted from the run artifacts.

## Recommended figures

### Figure 1: Main causal comparison
Four bars:
- baseline
- public repair x1
- blind retry x1
- verify-only x1

This is the headline figure.

### Figure 2: Precision summary
Three bars or dot plot:
- local known-good
- external replay
- combined gauntlet

### Figure 3: Proving-ground comparison
Same four-bar layout as Figure 1.
This shows public repair is live and still loses.

### Figure 4: Two-step robustness by model
Grouped bars for Claude and Codex across:
- baseline
- public repair x2
- blind retry x2
- verify-only x2

### Figure 5: Residual failure breakdown
Small stacked bar or table-figure hybrid for verify-only residuals.

## Suggested prose for the Results section
"On the primary causal matrix, verifier-guided repair reached 230/234 (98.3%), compared with 216/234 (92.3%) for blind retry, 205/234 (87.6%) for public-test-guided repair, and 208/234 (88.9%) for baseline. On the proving-ground suite designed to give public repair a fair chance to help, public repair improved over baseline (14/36 vs. 11/36), but verifier-guided repair still performed best at 25/36. In the completed two-step core robustness matrix, additional budget improved both public repair and blind retry, yet verifier-guided repair remained best on both models, finishing 78/78 on Claude and 78/78 on Codex." 

## What remains
1. Extract full failure counts for baseline, public repair, and retry policies from run artifacts.
2. Turn these tables into paper figures.
3. Decide whether to run the two-step proving-ground matrix or scope it out explicitly.
4. Add related work so the paper stops looking like a product memo and starts looking like a paper.
