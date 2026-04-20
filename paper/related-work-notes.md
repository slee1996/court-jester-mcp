# Related Work Notes

This note collects verified sources for the Court Jester paper. I am keeping the scope narrow and paper-serving rather than pretending to survey all of code-agent literature.

## 1. LLM-based program repair

### Impact of Code Language Models on Automated Program Repair
- Authors: Nan Jiang, Kevin Liu, Thibaud Lutellier, Lin Tan
- Venue: ICSE 2023
- DOI: 10.1109/ICSE48619.2023.00125
- Why it matters here: directly about code language models in automated program repair. Good anchor for the claim that LLM-based repair is already an established line of work.
- What to cite it for: prior work uses code models for program repair, but our paper is about verifier-triggered repair loops and benchmark attribution rather than only raw patch generation.

### Large Language Models for Automated Program Repair
- Author: Francisco Ribeiro
- Venue: SPLASH Companion 2023
- DOI: 10.1145/3618305.3623587
- Why it matters here: another direct APR reference. Likely lighter-weight than the ICSE paper, but still useful as evidence that LLM-driven repair is not a new premise.
- What to cite it for: the broader APR framing.

### Counterexample Guided Program Repair Using Zero-Shot Learning and MaxSAT-based Fault Localization
- Authors: Pedro Orvalho, Mikoláš Janota, Vasco M. Manquinho
- Venue: AAAI 2025
- DOI: 10.1609/aaai.v39i1.32046
- Why it matters here: useful bridge to the older counterexample-guided repair literature.
- What to cite it for: the idea that counterexamples are not just debugging artifacts; they can be active repair signals.

## 2. Iterative refinement and self-correction

### Self-Refine: Iterative Refinement with Self-Feedback
- Authors: Uri Alon et al.
- Venue: NeurIPS 2023
- DOI: 10.52202/075280-2019
- Why it matters here: canonical iterative refinement paper.
- What to cite it for: iterative repair/refinement is a known mechanism, but Court Jester differs by injecting external concrete counterexamples from execution rather than relying on open-ended self-feedback.

### Reflexion: Language Agents with Verbal Reinforcement Learning
- Authors: Federico Cassano, Ashwin Gopinath, Karthik Narasimhan, Noah Shinn, Shunyu Yao
- Venue: NeurIPS 2023
- DOI: 10.52202/075280-0377
- Why it matters here: another strong anchor for agent loops that improve through explicit feedback and retry.
- What to cite it for: agent-loop refinement with verbal feedback; contrast against verifier-generated failure artifacts.

## 3. Execution-aware code reasoning

### Code Execution with Pre-trained Language Models
- Authors: Chenxiao Liu et al.
- Venue: Findings of ACL 2023
- DOI: 10.18653/v1/2023.findings-acl.308
- Why it matters here: closest anchor for execution-aware language-model behavior.
- What to cite it for: execution is not merely downstream evaluation; it can be part of the reasoning or feedback loop.

## 4. Benchmark framing for real-world software tasks

### SWE-bench: Can Language Models Resolve Real-World GitHub Issues?
- Verified via arXiv title search
- arXiv: 2310.06770v3
- Why it matters here: strong anchor for real-world issue-resolution benchmarking.
- What to cite it for: repo-level software engineering benchmarks matter, but Court Jester is asking a different question: not “can an agent solve GitHub issues end-to-end?” but “does verifier-guided repair improve final outcomes without precision collapse?”

## How to organize the section

### Cluster A: LLM program repair
Use Jiang et al. and Ribeiro to establish that patch generation and LLM-based APR already exist.

### Cluster B: Iterative self-improvement
Use Self-Refine and Reflexion to establish that retry loops with feedback already exist.

### Cluster C: Execution-aware signals
Use Liu et al. to establish that execution can be part of the modeling loop, not just an evaluation afterthought.

### Cluster D: Real-world software benchmarks
Use SWE-bench to locate the paper inside the broader move toward realistic software engineering evaluation.

## What our paper is saying relative to these
It is not claiming to invent program repair, iterative refinement, or software-engineering evaluation.

It is claiming something narrower:
- concrete verifier-generated repros are a particularly effective repair signal
- this effect survives matched comparisons against public-test-guided repair and blind retries
- the gain does not appear to come from obvious precision collapse on the completed control package

## What to avoid in Related Work
- Do not pretend we are competing directly with full repo-level agent systems like a general SWE-bench paper.
- Do not say prior work lacks execution or feedback entirely; that is false.
- Do not turn the section into a laundry list of code LLM papers.
- Do not overclaim novelty at the level of “using feedback to improve outputs.” The novelty is the benchmarked verifier-feedback mechanism and its precision/utility attribution package.
